//! Input handling — all `handle_*_key` methods for `App`.
//!
//! Each modal and view mode has its own handler, dispatched from the top-level
//! `handle_key`. Methods are `pub(super)` so only the `app` module sees them.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::keys::{Action, Chord, Context, KeyMatch};
use crate::profiles;

use super::tasks::{bracketed_paste, complete_path, fetch_agents, seed_prompt};
use super::{App, AppMsg, ChatMode, NotesState, PaletteAction, PaletteItem, RenameForm, SplitDir, SpawnForm, TabKind, View, Workspace, Tab};

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
        // The rename overlay captures text until enter/esc.
        if self.rename.is_some() {
            self.handle_rename_key(key);
            return;
        }
        // The notes scratchpad is a modal with view/edit modes.
        if self.notes_view.is_some() {
            self.handle_notes_key(key);
            return;
        }
        // The review pane is a modal over the agent view with its own keys.
        if self.review.is_some() {
            self.handle_review_key(key);
            return;
        }
        // The sidebar filter captures keys literally while it's being typed.
        if self.filter_editing {
            self.handle_filter_key(key);
            return;
        }
        // The `:` ex-command line captures keys while it's open.
        if self.cmdline.is_some() {
            self.handle_cmdline_key(key);
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

        // Normal / navigation mode: feed the key into the pending sequence and
        // let the keymap decide. This powers the leader (which-key) menu and any
        // multi-key binding, while single-key bindings still fire on first press.
        let chord = Chord::from_event(&key);

        // Esc abandons a half-typed sequence (or a pending count) — and only
        // that; it doesn't also fire Esc's own binding.
        if (!self.pending_keys.is_empty() || self.count.is_some()) && key.code == KeyCode::Esc {
            self.pending_keys.clear();
            self.count = None;
            return;
        }

        // Vim count prefix: a leading digit accumulates a count for the next
        // motion (e.g. `3j`). Skipped when a question is pending — there `1`-`9`
        // answer it — and `0` only extends an existing count (never starts one).
        if self.pending_keys.is_empty() && !self.target_has_question() {
            if let KeyCode::Char(c @ '0'..='9') = key.code {
                if !key.modifiers.contains(KeyModifiers::CONTROL) {
                    let d = (c as u8 - b'0') as usize;
                    if !(d == 0 && self.count.is_none()) {
                        self.count =
                            Some(self.count.unwrap_or(0).saturating_mul(10).saturating_add(d));
                        return;
                    }
                }
            }
        }

        self.pending_keys.push(chord);
        // Leader + a digit teleports to a harpoon slot — positional, like the
        // answer keys, so it isn't nine separate keymap entries.
        if self.pending_keys.len() == 2 && self.pending_keys[0] == self.keymap.leader() {
            if let KeyCode::Char(d @ '1'..='9') = key.code {
                if !key.modifiers.contains(KeyModifiers::CONTROL) {
                    self.pending_keys.clear();
                    self.harpoon_jump((d as u8 - b'0') as usize);
                    return;
                }
            }
        }
        let ctxs = [Context::Global, self.key_context()];
        match self.keymap.resolve(&ctxs, &self.pending_keys) {
            KeyMatch::Action(action) => {
                self.pending_keys.clear();
                self.dispatch_action(action);
            }
            // Keep collecting; the which-key popup renders from `pending_keys`.
            KeyMatch::Pending => {}
            KeyMatch::None => {
                // Dead end. A lone unbound key falls back to the positional
                // answer keys (1–9), which live outside the remappable keymap.
                let was_single = self.pending_keys.len() == 1;
                self.pending_keys.clear();
                if was_single {
                    if let KeyCode::Char(c @ '1'..='9') = key.code {
                        if !key.modifiers.contains(KeyModifiers::CONTROL) {
                            self.answer_option(c);
                        }
                    }
                }
            }
        }
    }

    /// Which binding table the current view uses.
    pub(crate) fn key_context(&self) -> Context {
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
        // Consume any pending count: motions repeat / jump by it, every other
        // action just clears it.
        let count = self.count.take();
        let n = count.unwrap_or(1);
        match action {
            Quit => self.should_quit = true,
            Back => self.close_chat(),
            Refresh => self.on_changed(),
            Help => self.help = true,
            Palette => self.open_palette(),
            SelectNext => {
                for _ in 0..n {
                    self.select_next();
                }
            }
            SelectPrev => {
                for _ in 0..n {
                    self.select_prev();
                }
            }
            SelectFirst => self.selected = 0,
            // `G` goes to the last agent, or to agent N with a count (`5G`).
            SelectLast => {
                self.selected = match count {
                    Some(c) => c.min(self.agents.len()),
                    None => self.agents.len(),
                };
            }
            JumpAttention => self.jump_to_attention(),
            OpenAgent => self.open_agent(),
            OpenAgentTerminal => {
                if self.selected_agent().is_some() {
                    self.open_agent();
                    self.new_terminal_tab();
                }
            }
            OpenReview => self.open_review(),
            OpenNotes => self.open_notes(),
            RenameAgent => self.open_rename(),
            Respawn => self.respawn(),
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
                self.chat_scroll = self.chat_scroll.saturating_add(n as u16);
            }
            ScrollUp => {
                self.chat_follow = false;
                self.chat_scroll = self.chat_scroll.saturating_sub(n as u16);
            }
            Approve => self.approve("yes", "Approved"),
            Deny => self.approve("no", "Denied"),
            ApproveAlways => self.approve("always", "Approved (always)"),
            Interrupt => self.signal("SIGINT", "Interrupted"),
            Stop => self.signal("SIGTERM", "Stopped"),
            SplitRight => self.split_pane(SplitDir::Columns),
            SplitDown => self.split_pane(SplitDir::Rows),
            FocusNextPane => self.focus_pane(1),
            FocusPrevPane => self.focus_pane(-1),
            ClosePane => self.close_pane(),
            OnlyPane => self.only_pane(),
            HarpoonToggle => self.harpoon_toggle(),
            AltAgent => self.alt_agent(),
            JumpBack => self.jump_history(-1),
            JumpForward => self.jump_history(1),
            ToggleStopped => self.toggle_stopped(),
            OpenFilter => self.open_filter(),
            OpenCmdline => self.cmdline = Some(String::new()),
        }
    }

    // ── ex command line (`:`) ─────────────────────────────────────────────

    /// Keys while the `:` command line is open. `enter` runs the command,
    /// `esc` cancels.
    fn handle_cmdline_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => self.cmdline = None,
            KeyCode::Enter => {
                let cmd = self.cmdline.take().unwrap_or_default();
                self.run_command(&cmd);
            }
            KeyCode::Backspace => {
                if let Some(c) = self.cmdline.as_mut() {
                    c.pop();
                }
            }
            KeyCode::Char(c) => {
                if let Some(s) = self.cmdline.as_mut() {
                    s.push(c);
                }
            }
            _ => {}
        }
    }

    /// Run an ex command. Verbs map to existing actions; unknown ones toast.
    pub(super) fn run_command(&mut self, cmd: &str) {
        let cmd = cmd.trim();
        if cmd.is_empty() {
            return;
        }
        let (verb, arg) = cmd.split_once(char::is_whitespace).unwrap_or((cmd, ""));
        let arg = arg.trim();
        match verb {
            "q" | "quit" => self.should_quit = true,
            "vs" | "vsplit" => self.split_pane(SplitDir::Columns),
            "sp" | "split" => self.split_pane(SplitDir::Rows),
            "on" | "only" => self.only_pane(),
            "clo" | "close" => self.close_pane(),
            "new" | "spawn" => self.open_spawn(),
            "term" | "terminal" => self.new_terminal_tab(),
            "notes" => self.open_notes(),
            "review" => self.open_review(),
            "pin" => self.harpoon_toggle(),
            "help" | "h" => self.help = true,
            "ls" | "dashboard" => {
                self.view = View::List;
                self.selected = 0;
            }
            "rename" => {
                if arg.is_empty() {
                    self.open_rename();
                } else if let Some(cwd) = self.target_agent().map(|a| a.cwd_str().to_string()) {
                    if cwd.is_empty() {
                        self.set_toast("no working directory to name");
                    } else {
                        self.names.insert(cwd, arg.to_string());
                        crate::names::save(&self.names);
                        self.set_toast("Renamed");
                    }
                }
            }
            "filter" => {
                if arg.is_empty() {
                    self.open_filter();
                } else {
                    self.filter = Some(arg.to_string());
                    self.filter_editing = false;
                    self.apply_filter();
                }
            }
            other => self.set_toast(format!("unknown command: {other}")),
        }
    }

    // ── sidebar filter (`/`) ──────────────────────────────────────────────

    /// Start (or resume) typing the sidebar filter.
    pub(super) fn open_filter(&mut self) {
        self.filter_editing = true;
        if self.filter.is_none() {
            self.filter = Some(String::new());
        }
    }

    /// Keys while the `/` filter input is active. Live-filters as you type;
    /// `enter` keeps the filter and returns to navigation, `esc` clears it.
    pub(super) fn handle_filter_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.filter = None;
                self.filter_editing = false;
                self.apply_filter();
            }
            KeyCode::Enter => {
                self.filter_editing = false;
                // An empty query is the same as no filter.
                if self.filter.as_deref().is_some_and(str::is_empty) {
                    self.filter = None;
                }
                self.apply_filter();
            }
            KeyCode::Backspace => {
                if let Some(q) = self.filter.as_mut() {
                    q.pop();
                }
                self.apply_filter();
            }
            KeyCode::Char(c) => {
                if let Some(q) = self.filter.as_mut() {
                    q.push(c);
                }
                self.apply_filter();
            }
            _ => {}
        }
    }

    /// Show / hide stopped (incl. hydrated history) sessions in the sidebar, then
    /// re-pull so the change takes effect immediately.
    pub(super) fn toggle_stopped(&mut self) {
        self.show_all_sessions = !self.show_all_sessions;
        self.set_toast(if self.show_all_sessions {
            "Showing stopped sessions"
        } else {
            "Hiding stopped sessions"
        });
        self.refresh();
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

    /// Open the rename overlay for the targeted agent, prefilled with its
    /// current custom name (if any).
    pub(super) fn open_rename(&mut self) {
        let Some(agent) = self.target_agent() else { return };
        let cwd = agent.cwd_str().to_string();
        if cwd.is_empty() {
            self.set_toast("no working directory to name");
            return;
        }
        let input = self.names.get(&cwd).cloned().unwrap_or_default();
        self.rename = Some(RenameForm { cwd, input });
    }

    fn handle_rename_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => self.rename = None,
            KeyCode::Enter => self.submit_rename(),
            KeyCode::Backspace => {
                if let Some(f) = self.rename.as_mut() {
                    f.input.pop();
                }
            }
            KeyCode::Char(c) => {
                if let Some(f) = self.rename.as_mut() {
                    f.input.push(c);
                }
            }
            _ => {}
        }
    }

    /// Save the rename: store (or clear, when blank) the cwd's custom name and
    /// persist the map.
    fn submit_rename(&mut self) {
        let Some(form) = self.rename.take() else { return };
        let name = form.input.trim().to_string();
        if name.is_empty() {
            self.names.remove(&form.cwd);
        } else {
            self.names.insert(form.cwd.clone(), name);
        }
        crate::names::save(&self.names);
        self.set_toast("Renamed");
    }

    // ── notes scratchpad ────────────────────────────────────────────────────

    pub(super) fn open_notes(&mut self) {
        let Some(agent) = self.target_agent() else { return };
        let cwd = agent.cwd_str().to_string();
        if cwd.is_empty() {
            self.set_toast("no working directory for notes");
            return;
        }
        let text = self.notes.get(&cwd).cloned().unwrap_or_default();
        self.notes_view = Some(NotesState { cwd, text, editing: false, scroll: 0 });
    }

    fn handle_notes_key(&mut self, key: KeyEvent) {
        let editing = self.notes_view.as_ref().is_some_and(|n| n.editing);
        if editing {
            match key.code {
                // esc leaves edit mode (stays in the pane) and saves.
                KeyCode::Esc => {
                    if let Some(n) = self.notes_view.as_mut() {
                        n.editing = false;
                    }
                    self.save_notes();
                }
                KeyCode::Enter => {
                    if let Some(n) = self.notes_view.as_mut() {
                        n.text.push('\n');
                    }
                }
                KeyCode::Backspace => {
                    if let Some(n) = self.notes_view.as_mut() {
                        n.text.pop();
                    }
                }
                KeyCode::Char(c) => {
                    if let Some(n) = self.notes_view.as_mut() {
                        n.text.push(c);
                    }
                }
                _ => {}
            }
            return;
        }
        match key.code {
            KeyCode::Char('i') | KeyCode::Char('e') | KeyCode::Enter => {
                if let Some(n) = self.notes_view.as_mut() {
                    n.editing = true;
                }
            }
            KeyCode::Char('j') | KeyCode::Down => {
                if let Some(n) = self.notes_view.as_mut() {
                    n.scroll = n.scroll.saturating_add(1);
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if let Some(n) = self.notes_view.as_mut() {
                    n.scroll = n.scroll.saturating_sub(1);
                }
            }
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('h') => self.close_notes(),
            _ => {}
        }
    }

    fn close_notes(&mut self) {
        self.save_notes();
        self.notes_view = None;
    }

    /// Persist the open note (clearing the entry when blank).
    fn save_notes(&mut self) {
        let Some(n) = self.notes_view.as_ref() else { return };
        let cwd = n.cwd.clone();
        let text = n.text.trim_end().to_string();
        if text.is_empty() {
            self.notes.remove(&cwd);
        } else {
            self.notes.insert(cwd, text);
        }
        crate::notes::save(&self.notes);
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
        // Jump to a live agent (the full set, so the palette reaches agents the
        // `/` filter is hiding).
        for a in &self.all_agents {
            items.push(PaletteItem {
                label: format!("Go to {}", self.agent_name(a)),
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
                // Open by id (works even if the `/` filter is hiding it).
                if self.all_agents.iter().any(|a| a.session_id == sid) {
                    self.open_single(sid, true);
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
        // Opening from the sidebar resets the layout to a single pane.
        self.tiles = vec![id.clone()];
        self.tile_focus = 0;
        self.focus_agent(id);
    }

    /// Make `id` the interactive (focused) agent: sync the view, ensure its
    /// workspace exists, and warm its terminal. Records the move in the jump
    /// history / alternate-agent. Shared by open / split / pane-focus.
    pub(super) fn focus_agent(&mut self, id: String) {
        self.focus_agent_inner(id, true);
    }

    /// Focus `id`, optionally recording it in the jump history. `record` is
    /// false when we're *navigating* the history itself (`Ctrl-o` / forward), so
    /// stepping back and forth doesn't keep appending.
    fn focus_agent_inner(&mut self, id: String, record: bool) {
        if record {
            if let Some(prev) = self.open_agent_id() {
                if prev != id {
                    let prev = prev.to_string();
                    self.prev_focus = Some(prev);
                    // Drop any forward history, then append the destination.
                    self.jumplist.truncate(self.jump_idx + 1);
                    if self.jumplist.last().map(String::as_str) != Some(id.as_str()) {
                        self.jumplist.push(id.clone());
                    }
                    self.jump_idx = self.jumplist.len().saturating_sub(1);
                }
            } else if self.jumplist.last().map(String::as_str) != Some(id.as_str()) {
                // First agent opened — seed the history.
                self.jumplist.push(id.clone());
                self.jump_idx = self.jumplist.len().saturating_sub(1);
            }
        }
        let cwd = self
            .agents
            .iter()
            .find(|a| a.session_id == id)
            .map(|a| a.cwd_str().to_string())
            .unwrap_or_default();
        self.load_git_summary(cwd);
        self.view = View::Agent { id: id.clone() };
        self.workspaces.entry(id.clone()).or_insert_with(|| Workspace {
            tabs: vec![Tab { title: "claude".into(), session_id: id.clone(), kind: TabKind::Claude }],
            active: 0,
        });
        self.enter_active_tab();
    }

    // ── harpoon (pinned agents) + jump history ────────────────────────────

    /// Open `id` as a single full-content pane (collapsing any splits), the way
    /// a harpoon/jumplist teleport behaves.
    fn open_single(&mut self, id: String, record: bool) {
        self.tiles = vec![id.clone()];
        self.tile_focus = 0;
        self.focus_agent_inner(id, record);
    }

    /// Pin or unpin the target agent in the harpoon list.
    pub(super) fn harpoon_toggle(&mut self) {
        let Some(sid) = self.target_session() else {
            self.set_toast("no agent to pin");
            return;
        };
        if let Some(pos) = self.harpoon.iter().position(|s| s == &sid) {
            self.harpoon.remove(pos);
            self.set_toast("Unpinned");
        } else {
            self.harpoon.push(sid);
            self.set_toast(format!("Pinned #{}", self.harpoon.len()));
        }
    }

    /// Teleport to the 1-based harpoon slot, if it's filled.
    pub(super) fn harpoon_jump(&mut self, slot: usize) {
        let Some(sid) = slot.checked_sub(1).and_then(|i| self.harpoon.get(i)).cloned() else {
            self.set_toast(format!("no agent pinned at {slot}"));
            return;
        };
        self.open_single(sid, true);
    }

    /// Jump to the alternate agent (the one focused just before this one).
    pub(super) fn alt_agent(&mut self) {
        let Some(alt) = self.prev_focus.clone() else {
            self.set_toast("no alternate agent");
            return;
        };
        if !self.all_agents.iter().any(|a| a.session_id == alt) {
            self.set_toast("alternate agent is gone");
            return;
        }
        self.open_single(alt, true);
    }

    /// Step back / forward through the jump history.
    pub(super) fn jump_history(&mut self, delta: i32) {
        if self.jumplist.is_empty() {
            return;
        }
        let target = self.jump_idx as i32 + delta;
        if target < 0 || target as usize >= self.jumplist.len() {
            self.set_toast(if delta < 0 { "start of jumps" } else { "end of jumps" });
            return;
        }
        self.jump_idx = target as usize;
        let id = self.jumplist[self.jump_idx].clone();
        self.open_single(id, false);
    }

    // ── window splits (panes) ─────────────────────────────────────────────

    /// First agent not already tiled, for bringing another agent into view.
    /// Uses the full set so splits can pull in agents the `/` filter hides.
    fn next_untiled_agent(&self) -> Option<String> {
        self.all_agents
            .iter()
            .map(|a| &a.session_id)
            .find(|sid| !self.tiles.contains(sid))
            .cloned()
    }

    /// Split the focused pane: tile another agent beside/below it and move focus
    /// to the new pane (vim's `Ctrl-w v` / `Ctrl-w s`). No-op outside an agent
    /// view; capped so cells stay usable.
    pub(super) fn split_pane(&mut self, dir: SplitDir) {
        const MAX_TILES: usize = 4;
        if self.open_agent_id().is_none() {
            return;
        }
        if self.tiles.len() >= MAX_TILES {
            self.set_toast("max splits reached");
            return;
        }
        let Some(next) = self.next_untiled_agent() else {
            self.set_toast("no other agent to split");
            return;
        };
        self.split_dir = dir;
        let at = (self.tile_focus + 1).min(self.tiles.len());
        self.tiles.insert(at, next.clone());
        self.tile_focus = at;
        self.focus_agent(next);
    }

    /// Move focus to another tiled pane (wrapping). `delta` is +1 / -1.
    pub(super) fn focus_pane(&mut self, delta: i32) {
        let n = self.tiles.len();
        if n <= 1 {
            return;
        }
        let next = (self.tile_focus as i32 + delta).rem_euclid(n as i32) as usize;
        self.tile_focus = next;
        let id = self.tiles[next].clone();
        self.focus_agent(id);
    }

    /// Close the focused pane. The last pane closing leaves the agent view.
    pub(super) fn close_pane(&mut self) {
        if self.tiles.len() <= 1 {
            self.close_chat();
            return;
        }
        self.tiles.remove(self.tile_focus);
        if self.tile_focus >= self.tiles.len() {
            self.tile_focus = self.tiles.len() - 1;
        }
        let id = self.tiles[self.tile_focus].clone();
        self.focus_agent(id);
    }

    /// Keep only the focused pane (vim's `Ctrl-w o`).
    pub(super) fn only_pane(&mut self) {
        if self.tiles.len() <= 1 {
            return;
        }
        let id = self.tiles[self.tile_focus].clone();
        self.tiles = vec![id];
        self.tile_focus = 0;
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
        let initial_prompt = form.initial_prompt.clone();
        self.spawn_form = None;
        self.spawn_agent_in(cwd, profile, initial_prompt, None);
    }

    /// Spawn a fresh Claude session in `cwd` with `profile`, optionally seeding a
    /// prompt once it reaches its input prompt. Shared by the spawn modal and the
    /// respawn action.
    pub(super) fn spawn_agent_in(
        &self,
        cwd: String,
        profile: profiles::Profile,
        initial_prompt: Option<String>,
        resume_session_id: Option<String>,
    ) {
        // Resuming reuses the prior session id (which is also claude's transcript
        // uuid, since we pin `--session-id` at spawn) and passes it as `--resume`
        // so claude reopens that conversation. A fresh spawn mints a new id and
        // pins it up front so claude's transcript file, claudemon's id, and the
        // id we track all agree — no cwd-based guessing.
        let resume = resume_session_id.is_some();
        let session_id =
            resume_session_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let argv = profiles::build_argv(&profile, None, false, &session_id, resume);
        let env = profiles::build_env(&profile);
        let claudemon = self.claudemon.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let sid = match claudemon.spawn(argv, cwd, env, &session_id).await {
                Ok(sid) => {
                    let verb = if resume { "Resumed" } else { "Spawned" };
                    let _ = tx.send(AppMsg::Toast(format!("{verb} agent")));
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

    /// Restart a stopped agent by resuming its conversation: spawn
    /// `claude --resume <id>` in its cwd with the default profile. The id is the
    /// agent's own session id, which doubles as claude's transcript uuid (we pin
    /// `--session-id` at spawn), so claude reopens the prior conversation instead
    /// of starting blank. (claudemon keeps the old stopped session in its list
    /// until it's pruned; this adds a live one in the same directory.)
    pub(super) fn respawn(&mut self) {
        let Some(agent) = self.target_agent() else { return };
        if agent.state() != "stopped" {
            self.set_toast("agent is still running");
            return;
        }
        let session_id = agent.session_id.clone();
        let cwd = agent.cwd_str().to_string();
        if cwd.is_empty() {
            self.set_toast("no working directory");
            return;
        }
        let Some(profile) = self
            .profiles
            .iter()
            .find(|p| p.is_default)
            .or_else(|| self.profiles.first())
            .cloned()
        else {
            self.set_toast("no profile available");
            return;
        };
        self.set_toast("Resuming…");
        self.spawn_agent_in(cwd, profile, None, Some(session_id));
    }

    pub(super) fn close_chat(&mut self) {
        // Leave the terminal warm in the background so coming back is instant.
        self.view = View::List;
        self.chat_mode = ChatMode::Terminal;
        self.term_attached = false;
        self.insert_mode = false;
        self.input.clear();
        self.turns.clear();
        self.tiles.clear();
        self.tile_focus = 0;
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

    /// Whether the targeted agent has a pending question — if so, `1`-`9` answer
    /// it rather than starting a vim count.
    fn target_has_question(&self) -> bool {
        self.target_agent().is_some_and(|a| a.has_question())
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
