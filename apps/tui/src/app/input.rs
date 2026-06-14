//! Input handling — all `handle_*_key` methods for `App`.
//!
//! Each modal and view mode has its own handler, dispatched from the top-level
//! `handle_key`. Methods are `pub(super)` so only the `app` module sees them.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::keys::{Action, Chord, Context};
use crate::profiles;

use super::tasks::{bracketed_paste, complete_path, fetch_agents, seed_prompt};
use super::{App, AppMsg, ChatMode, PaletteAction, PaletteItem, SpawnForm, TabKind, View, Workspace, Tab};

impl App {
    // ── top-level dispatcher ──────────────────────────────────────────────

    pub fn handle_key(&mut self, key: KeyEvent) {
        // The help overlay swallows input: any key dismisses it (Ctrl-C still
        // quits, so a stuck overlay can never trap the user).
        if self.help {
            if is_ctrl_c(&key) {
                self.should_quit = true;
            } else {
                self.help = false;
            }
            return;
        }
        // Text-entry / raw modes capture keys literally — before the keymap.
        if self.spawn_form.is_some() {
            self.handle_spawn_key(key);
            return;
        }
        if self.palette.is_some() {
            self.handle_palette_key(key);
            return;
        }
        // The review pane is a modal over the agent view with its own keys.
        if self.review.is_some() {
            self.handle_review_key(key);
            return;
        }
        // When attached to the live terminal, every key goes to Claude (so
        // Ctrl-C interrupts the agent, not the TUI). Ctrl-] detaches.
        if self.term_attached() {
            self.handle_terminal_key(key);
            return;
        }
        // The composer captures characters when typing a message/answer.
        if matches!(self.view, View::Agent { .. }) && self.insert_mode {
            self.handle_insert_key(key);
            return;
        }

        let chord = Chord::from_event(&key);
        // Global bindings (palette / quit / help) win in every non-text view.
        if let Some(action) = self.keymap.action(Context::Global, chord) {
            self.dispatch_action(action);
            return;
        }
        // Numeric answer keys are positional, not remappable.
        if let KeyCode::Char(c @ '1'..='9') = key.code {
            if !key.modifiers.contains(KeyModifiers::CONTROL) {
                self.answer_option(c);
                return;
            }
        }
        if let Some(action) = self.keymap.action(self.key_context(), chord) {
            self.dispatch_action(action);
        }
    }

    /// Which binding table the current view uses.
    fn key_context(&self) -> Context {
        match &self.view {
            View::List => Context::List,
            View::Agent { .. } => {
                let on_shell = matches!(self.active_tab().map(|t| t.kind), Some(TabKind::Shell));
                if on_shell || self.chat_mode == ChatMode::Terminal {
                    Context::AgentTerminal
                } else {
                    Context::AgentTranscript
                }
            }
        }
    }

    /// Execute a resolved keymap action. Actions are semantic; a few check the
    /// active tab so they no-op where they don't apply (e.g. transcript toggle
    /// on a shell tab).
    fn dispatch_action(&mut self, action: Action) {
        use Action::*;
        match action {
            Quit => self.should_quit = true,
            Back => self.close_chat(),
            Refresh => self.on_changed(),
            Help => self.help = true,
            Palette => self.open_palette(),
            SelectNext => self.select_next(),
            SelectPrev => self.select_prev(),
            SelectFirst => self.selected = 0,
            SelectLast => self.selected = self.agents.len(),
            JumpAttention => self.jump_to_attention(),
            OpenAgent => self.open_agent(),
            OpenAgentTerminal => {
                if self.selected_agent().is_some() {
                    self.open_agent();
                    self.new_terminal_tab();
                }
            }
            OpenReview => self.open_review(),
            NewAgent => self.open_spawn(),
            NewTerminal => self.new_terminal_tab(),
            CloseTab => self.close_tab(),
            TabNext => self.tab_next(),
            TabPrev => self.tab_prev(),
            ToggleTranscript => {
                let on_shell = matches!(self.active_tab().map(|t| t.kind), Some(TabKind::Shell));
                if !on_shell {
                    self.toggle_chat_mode();
                }
            }
            Attach => {
                if self.open_session_id().is_some() {
                    self.term_attached = true;
                }
            }
            InsertMode => self.insert_mode = true,
            ScrollDown => {
                self.chat_follow = false;
                self.chat_scroll = self.chat_scroll.saturating_add(1);
            }
            ScrollUp => {
                self.chat_follow = false;
                self.chat_scroll = self.chat_scroll.saturating_sub(1);
            }
            Approve => self.approve("yes", "Approved"),
            Deny => self.approve("no", "Denied"),
            ApproveAlways => self.approve("always", "Approved (always)"),
            Interrupt => self.signal("SIGINT", "Interrupted"),
            Stop => self.signal("SIGTERM", "Stopped"),
        }
    }

    /// Forward a keystroke to the PTY, or detach on Ctrl-].
    fn handle_terminal_key(&mut self, key: KeyEvent) {
        if crate::terminal::is_detach(&key) {
            self.term_attached = false;
            return;
        }
        let Some(sid) = self.open_session_id() else { return };
        let Some(bytes) = crate::terminal::encode_key(&key) else { return };
        let cm = self.claudemon.clone();
        tokio::spawn(async move {
            let _ = cm.input_bytes(&sid, &bytes).await;
        });
    }

    /// Keys for the git review pane (a modal over the agent view). Bypasses the
    /// keymap — this pane owns all its keys, including a commit-message composer.
    fn handle_review_key(&mut self, key: KeyEvent) {
        // Commit-message composer captures characters until enter/esc.
        if self.review.as_ref().is_some_and(|r| r.commit_msg.is_some()) {
            match key.code {
                KeyCode::Esc => {
                    if let Some(r) = self.review.as_mut() {
                        r.commit_msg = None;
                    }
                }
                KeyCode::Enter => self.review_submit_commit(),
                KeyCode::Backspace => {
                    if let Some(m) = self.review.as_mut().and_then(|r| r.commit_msg.as_mut()) {
                        m.pop();
                    }
                }
                KeyCode::Char(c) => {
                    if let Some(m) = self.review.as_mut().and_then(|r| r.commit_msg.as_mut()) {
                        m.push(c);
                    }
                }
                _ => {}
            }
            return;
        }

        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc | KeyCode::Char('h') | KeyCode::Char('R') | KeyCode::Char('q') => {
                self.close_review()
            }
            KeyCode::Char('j') | KeyCode::Down => self.review_select(1),
            KeyCode::Char('k') | KeyCode::Up => self.review_select(-1),
            // Diff scroll: J/K by a line, Ctrl-D/U or PageDn/Up by a chunk.
            KeyCode::Char('J') => self.review_scroll(1),
            KeyCode::Char('K') => self.review_scroll(-1),
            KeyCode::Char('d') if ctrl => self.review_scroll(10),
            KeyCode::Char('u') if ctrl => self.review_scroll(-10),
            KeyCode::PageDown => self.review_scroll(10),
            KeyCode::PageUp => self.review_scroll(-10),
            KeyCode::Char('t') => self.review_toggle_staged(),
            KeyCode::Char('s') => self.review_stage(),
            KeyCode::Char('u') => self.review_unstage(),
            KeyCode::Char('a') => self.review_stage_all(),
            KeyCode::Char('c') => {
                if let Some(r) = self.review.as_mut() {
                    r.commit_msg = Some(String::new());
                }
            }
            KeyCode::Char('P') => self.review_push(),
            KeyCode::Char('r') => self.review_reload(),
            _ => {}
        }
    }

    pub(super) fn handle_spawn_key(&mut self, key: KeyEvent) {
        let n = self.profiles.len();
        let Some(form) = self.spawn_form.as_mut() else { return };
        match key.code {
            KeyCode::Esc => self.spawn_form = None,
            KeyCode::Enter => self.submit_spawn(),
            // Shell-style path completion on the cwd field.
            KeyCode::Tab => complete_path(form),
            // Cycle the chosen profile.
            KeyCode::Down | KeyCode::Right => {
                if n > 0 {
                    form.profile_idx = (form.profile_idx + 1) % n;
                }
            }
            KeyCode::Up | KeyCode::Left => {
                if n > 0 {
                    form.profile_idx = (form.profile_idx + n - 1) % n;
                }
            }
            KeyCode::Backspace => {
                form.cwd.pop();
                form.completions.clear();
            }
            KeyCode::Char(c) => {
                form.cwd.push(c);
                form.completions.clear();
            }
            _ => {}
        }
    }

    // ── command palette (Ctrl-K) ──────────────────────────────────────────

    pub(super) fn open_palette(&mut self) {
        let mut items = vec![
            PaletteItem { label: "New agent".into(), hint: "spawn".into(), action: PaletteAction::NewAgent },
            PaletteItem { label: "New terminal".into(), hint: "shell tab".into(), action: PaletteAction::NewTerminal },
            PaletteItem { label: "Dashboard".into(), hint: "overview".into(), action: PaletteAction::Dashboard },
        ];
        // Jump to a live agent.
        for a in &self.agents {
            items.push(PaletteItem {
                label: format!("Go to {}", a.short_cwd()),
                hint: a.state().to_string(),
                action: PaletteAction::OpenAgent(a.session_id.clone()),
            });
        }
        // Library items — run in a new agent, or insert into the focused one.
        for item in &self.library {
            items.push(PaletteItem {
                label: format!("Run \"{}\" in new agent", item.title),
                hint: item.kind.clone(),
                action: PaletteAction::SpawnWithPrompt(item.body.clone()),
            });
            items.push(PaletteItem {
                label: format!("Insert \"{}\"  ({})", item.title, item.kind),
                hint: item.description.clone().unwrap_or_default(),
                action: PaletteAction::Insert(item.body.clone()),
            });
        }
        self.palette = Some(super::Palette::new(items));
    }

    pub(super) fn handle_palette_key(&mut self, key: KeyEvent) {
        let Some(p) = self.palette.as_mut() else { return };
        match key.code {
            KeyCode::Esc => self.palette = None,
            KeyCode::Enter => {
                let action = p.chosen().map(|it| it.action.clone());
                self.palette = None;
                if let Some(action) = action {
                    self.run_palette_action(action);
                }
            }
            KeyCode::Down => {
                if !p.filtered.is_empty() {
                    p.selected = (p.selected + 1).min(p.filtered.len() - 1);
                }
            }
            KeyCode::Up => p.selected = p.selected.saturating_sub(1),
            KeyCode::Backspace => {
                p.query.pop();
                p.refilter();
            }
            KeyCode::Char(c) => {
                p.query.push(c);
                p.refilter();
            }
            _ => {}
        }
    }

    pub(super) fn run_palette_action(&mut self, action: PaletteAction) {
        match action {
            PaletteAction::NewAgent => self.open_spawn(),
            PaletteAction::NewTerminal => {
                if self.open_agent_id().is_some() {
                    self.new_terminal_tab();
                } else if self.selected_agent().is_some() {
                    self.open_agent();
                    self.new_terminal_tab();
                } else {
                    self.set_toast("select an agent first");
                }
            }
            PaletteAction::Dashboard => {
                self.view = View::List;
                self.selected = 0;
            }
            PaletteAction::OpenAgent(sid) => {
                if let Some(i) = self.agents.iter().position(|a| a.session_id == sid) {
                    self.selected = i + 1;
                    self.open_agent();
                }
            }
            PaletteAction::Insert(body) => {
                let Some(sid) = self.open_session_id() else {
                    self.set_toast("open an agent's terminal to insert");
                    return;
                };
                let cm = self.claudemon.clone();
                let bytes = bracketed_paste(&body);
                self.set_toast("Inserted");
                tokio::spawn(async move {
                    let _ = cm.input_bytes(&sid, &bytes).await;
                });
            }
            PaletteAction::SpawnWithPrompt(body) => self.open_spawn_with_prompt(body),
        }
    }

    pub(super) fn handle_insert_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => self.insert_mode = false,
            KeyCode::Enter => self.send_input(),
            KeyCode::Backspace => {
                self.input.pop();
            }
            KeyCode::Char(c) => self.input.push(c),
            _ => {}
        }
    }

    // ── action helpers ────────────────────────────────────────────────────

    pub(super) fn select_next(&mut self) {
        // Rows are [Dashboard, ..agents], so the max index is agents.len().
        self.selected = (self.selected + 1).min(self.agents.len());
    }

    pub(super) fn select_prev(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    /// Move selection to the next agent (after the current one, wrapping) that
    /// needs the user — the `m` "jump to attention" key.
    pub(super) fn jump_to_attention(&mut self) {
        let n = self.agents.len();
        if n == 0 {
            return;
        }
        // Current agent index, or "before the first" when the Dashboard is selected.
        let cur = if self.selected == 0 { n - 1 } else { self.selected - 1 };
        for offset in 1..=n {
            let i = (cur + offset) % n;
            if self.agents[i].is_waiting() {
                self.selected = i + 1;
                return;
            }
        }
        self.set_toast("Nothing waiting");
    }

    /// Open the selected agent into its tab view (creating its workspace with a
    /// single Claude tab the first time). The Dashboard row doesn't "open" — it's
    /// a live preview, so Enter there is a no-op.
    pub(super) fn open_agent(&mut self) {
        let Some(agent) = self.selected_agent() else { return };
        let id = agent.session_id.clone();
        self.view = View::Agent { id: id.clone() };
        self.workspaces.entry(id.clone()).or_insert_with(|| Workspace {
            tabs: vec![Tab { title: "claude".into(), session_id: id.clone(), kind: TabKind::Claude }],
            active: 0,
        });
        self.enter_active_tab();
    }

    /// Set up rendering for whatever the active tab points at: warm its terminal
    /// (or fall back to transcript for no-PTY sessions).
    pub(super) fn enter_active_tab(&mut self) {
        self.turns.clear();
        self.chat_scroll = 0;
        self.chat_follow = true;
        self.insert_mode = false;
        self.term_attached = false;
        let Some(tab) = self.active_tab().cloned() else { return };
        if tab.kind == TabKind::Claude && self.no_terminal.contains(&tab.session_id) {
            self.chat_mode = ChatMode::Transcript;
            self.load_transcript(tab.session_id);
        } else {
            self.chat_mode = ChatMode::Terminal;
            self.ensure_terminal(tab.session_id);
        }
    }

    pub(super) fn tab_next(&mut self) {
        if let Some(ws) = self.workspace_mut() {
            if !ws.tabs.is_empty() {
                ws.active = (ws.active + 1) % ws.tabs.len();
            }
        }
        self.enter_active_tab();
    }

    pub(super) fn tab_prev(&mut self) {
        if let Some(ws) = self.workspace_mut() {
            let n = ws.tabs.len();
            if n > 0 {
                ws.active = (ws.active + n - 1) % n;
            }
        }
        self.enter_active_tab();
    }

    /// Open a new shell tab: spawn `$SHELL` via claudemon in the agent's cwd (so
    /// it's a real PTY we can stream, and shows in the system-wide list). The
    /// session id comes back async and is added as a tab then.
    pub(super) fn new_terminal_tab(&mut self) {
        let Some(id) = self.open_agent_id().map(|s| s.to_string()) else { return };
        let cwd = self
            .chat_agent()
            .map(|a| a.cwd_str().to_string())
            .filter(|c| !c.is_empty())
            .or_else(|| std::env::current_dir().ok().map(|p| p.display().to_string()))
            .unwrap_or_else(|| "/".into());
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".into());
        let cm = self.claudemon.clone();
        let tx = self.tx.clone();
        self.set_toast("Opening terminal…");
        tokio::spawn(async move {
            match cm.spawn(vec![shell], cwd, serde_json::Map::new(), "").await {
                Ok(sid) => {
                    let _ = tx.send(AppMsg::ShellSpawned { agent_id: id, session_id: sid });
                }
                Err(e) => {
                    let _ = tx.send(AppMsg::Toast(format!("Terminal failed: {e}")));
                }
            }
        });
    }

    /// Close the active tab. Closing the primary Claude tab leaves the agent
    /// (back to the list); closing a shell tab stops that shell.
    pub(super) fn close_tab(&mut self) {
        let Some(ws) = self.workspace() else { return };
        let Some(tab) = ws.active_tab().cloned() else { return };
        if tab.kind == TabKind::Claude {
            self.close_chat();
            return;
        }
        // Shell: stop the process and drop the tab.
        let sid = tab.session_id.clone();
        let cm = self.claudemon.clone();
        tokio::spawn(async move {
            let _ = cm.signal(&sid, "SIGTERM").await;
        });
        if let Some(ws) = self.workspace_mut() {
            let idx = ws.active;
            ws.tabs.remove(idx);
            if ws.active >= ws.tabs.len() {
                ws.active = ws.tabs.len().saturating_sub(1);
            }
        }
        self.enter_active_tab();
    }

    /// Open the spawn modal, prefilling the cwd with where the TUI was launched.
    pub(super) fn open_spawn(&mut self) {
        self.open_spawn_inner(None);
    }

    /// Open the spawn modal carrying a prompt to seed into the new agent.
    pub(super) fn open_spawn_with_prompt(&mut self, prompt: String) {
        self.open_spawn_inner(Some(prompt));
    }

    pub(super) fn open_spawn_inner(&mut self, initial_prompt: Option<String>) {
        let cwd = std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        self.spawn_form = Some(SpawnForm {
            cwd,
            profile_idx: 0,
            completions: Vec::new(),
            initial_prompt,
        });
    }

    /// Spawn a Claude session in the chosen cwd with the chosen profile, via
    /// claudemon's REST API. The new agent surfaces in the sidebar on the next
    /// state-change event (claudemon emits one once Claude starts up).
    pub(super) fn submit_spawn(&mut self) {
        let Some(form) = self.spawn_form.clone() else { return };
        let cwd = profiles::normalize_cwd(&form.cwd);
        if cwd.is_empty() {
            self.set_toast("working directory required");
            return;
        }
        let Some(profile) = self.profiles.get(form.profile_idx).cloned() else {
            self.set_toast("no profile selected");
            return;
        };
        self.spawn_form = None;

        // Pin the session id up front so claude's transcript file, claudemon's
        // id, and the id we track all agree — no cwd-based guessing.
        let session_id = uuid::Uuid::new_v4().to_string();
        let argv = profiles::build_argv(&profile, None, false, &session_id);
        let env = profiles::build_env(&profile);
        let initial_prompt = form.initial_prompt.clone();
        let claudemon = self.claudemon.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let sid = match claudemon.spawn(argv, cwd, env, &session_id).await {
                Ok(sid) => {
                    let _ = tx.send(AppMsg::Toast("Spawned agent".into()));
                    fetch_agents(&claudemon, &tx).await;
                    sid
                }
                Err(e) => {
                    let _ = tx.send(AppMsg::Toast(format!("Spawn failed: {e}")));
                    return;
                }
            };
            if let Some(prompt) = initial_prompt {
                seed_prompt(&claudemon, &tx, &sid, &prompt).await;
            }
        });
    }

    pub(super) fn close_chat(&mut self) {
        // Leave the terminal warm in the background so coming back is instant.
        self.view = View::List;
        self.chat_mode = ChatMode::Terminal;
        self.term_attached = false;
        self.insert_mode = false;
        self.input.clear();
        self.turns.clear();
    }

    /// The agent a key action targets: the chat's agent when in chat, else the
    /// selected sidebar agent.
    pub(super) fn target_session(&self) -> Option<String> {
        match &self.view {
            View::Agent { .. } => self.chat_session_id(),
            View::List => self.selected_agent().map(|a| a.session_id.clone()),
        }
    }

    pub(super) fn target_agent(&self) -> Option<&super::super::types::Agent> {
        match &self.view {
            View::Agent { .. } => self.chat_agent(),
            View::List => self.selected_agent(),
        }
    }

    pub(super) fn approve(&mut self, decision: &str, ok: &str) {
        let Some(agent) = self.target_agent() else { return };
        if agent.approval().is_none() {
            return;
        }
        let sid = agent.session_id.clone();
        let cm = self.claudemon.clone();
        let decision = decision.to_string();
        self.dispatch(ok, async move { cm.approve(&sid, &decision, None).await });
    }

    /// Answer the first pending question with the option at 1-based key `c`.
    pub(super) fn answer_option(&mut self, c: char) {
        let Some(agent) = self.target_agent() else { return };
        if !agent.has_question() {
            return;
        }
        let option = (c as u8 - b'0') as u64; // '1'..='9' → 1..=9
        let sid = agent.session_id.clone();
        let cm = self.claudemon.clone();
        self.dispatch("Answered", async move { cm.answer_option(&sid, option).await });
    }

    pub(super) fn signal(&mut self, signal: &str, ok: &str) {
        let Some(sid) = self.target_session() else { return };
        let cm = self.claudemon.clone();
        let signal = signal.to_string();
        self.dispatch(ok, async move { cm.signal(&sid, &signal).await });
    }

    /// Send the composer's contents — as an answer if the agent is on a
    /// question, otherwise as a chat message. Mirrors the `/remote` heuristic.
    pub(super) fn send_input(&mut self) {
        let text = self.input.trim().to_string();
        if text.is_empty() {
            return;
        }
        let Some(agent) = self.target_agent() else { return };
        let sid = agent.session_id.clone();
        let answering = agent.has_question();
        let cm = self.claudemon.clone();
        self.dispatch("Sent", async move {
            if answering {
                cm.answer_text(&sid, &text).await
            } else {
                cm.message(&sid, &text).await
            }
        });
        self.input.clear();
    }
}

/// Ctrl-C, the universal escape hatch — honored even while the help overlay is
/// up so the user can always quit.
fn is_ctrl_c(key: &KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c')
}
