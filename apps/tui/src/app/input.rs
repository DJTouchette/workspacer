//! Input handling — all `handle_*_key` methods for `App`.
//!
//! Each modal and view mode has its own handler, dispatched from the top-level
//! `handle_key`. Methods are `pub(super)` so only the `app` module sees them.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::keys::{Action, Chord, Context, KeyMatch};
use crate::profiles;

use super::tasks::{bracketed_paste, complete_path, fetch_agents, fetch_search_index, seed_prompt};
use super::{
    App, AppMsg, ChatMode, NotesState, PaletteAction, PaletteItem, Picker, PickerItem, PickerKind,
    RenameForm, SearchState, SpawnForm, SplitDir, Tab, TabKind, View, Workspace,
};

/// Ex-command verbs surfaced in the Ctrl-K palette as a "command" source — the
/// fuzzy-findable mirror of the `:` command line. `(verb, description)`.
const COMMAND_PALETTE: &[(&str, &str)] = &[
    ("vsplit", "split window into columns"),
    ("split", "split window into rows"),
    ("only", "keep only the focused pane"),
    ("close", "close the focused pane"),
    ("spawn", "new agent"),
    ("term", "new terminal tab"),
    ("notes", "open the notes scratchpad"),
    ("review", "open the git review"),
    ("pin", "pin / unpin the agent (harpoon)"),
    ("search", "search transcripts across all agents"),
    ("model", "switch the model (live)"),
    ("permission", "cycle the permission mode"),
    ("handoff", "hand off to another provider"),
    ("rename", "rename the agent"),
    ("filter", "filter the sidebar"),
    ("dashboard", "go to the dashboard"),
    ("help", "keybindings"),
    ("quit", "quit"),
];

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
        // The model / handoff-provider picker captures keys while it's open.
        if self.picker.is_some() {
            self.handle_picker_key(key);
            return;
        }
        // The content-search modal captures keys while it's open.
        if self.search.is_some() {
            self.handle_search_key(key);
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

        // Question stepper keys, intercepted before the keymap (like the
        // positional 1-9 answer keys): Enter confirms the current multi-select
        // question's toggles; Esc steps back to the previous question mid-set.
        // Only where the stepper is actually visible (the list detail pane and
        // the transcript's ask block) — in terminal mode no stepper renders
        // and Enter must keep meaning attach.
        if self.pending_keys.is_empty()
            && self.target_has_question()
            && self.key_context() != Context::AgentTerminal
        {
            if key.code == KeyCode::Enter && self.current_question_is_multiselect() {
                self.question_confirm_multiselect();
                return;
            }
            if key.code == KeyCode::Esc && self.question_can_step_back() {
                self.question_back();
                return;
            }
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
                    // Consume any pending vim count so it can't leak into the
                    // next motion (mirrors dispatch_action's `self.count.take()`).
                    self.count = None;
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
                self.chat_scroll = self.chat_scroll.saturating_add(n);
            }
            ScrollUp => {
                self.chat_follow = false;
                self.chat_scroll = self.chat_scroll.saturating_sub(n);
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
            OpenSearch => self.open_search(),
            SwitchModel => self.open_model_picker(),
            CyclePermissionMode => self.cycle_permission_mode(),
            Handoff => self.open_handoff_picker(),
        }
    }

    // ── cross-agent content search ────────────────────────────────────────

    /// Open the content-search modal and kick off indexing: fetch each non-shell
    /// session's transcript in the background; lines stream in as `SearchEntries`.
    pub(super) fn open_search(&mut self) {
        let targets: Vec<(String, String, String)> = self
            .all_agents
            .iter()
            .filter(|a| !self.is_shell_session(&a.session_id))
            .map(|a| {
                (
                    a.session_id.clone(),
                    self.agent_name(a),
                    a.transport.clone(),
                )
            })
            .collect();
        self.search = Some(SearchState {
            query: String::new(),
            entries: Vec::new(),
            matched: Vec::new(),
            selected: 0,
            pending: targets.len(),
        });
        for (sid, name, transport) in targets {
            let cm = self.claudemon.clone();
            let tx = self.tx.clone();
            tokio::spawn(async move { fetch_search_index(&cm, &tx, sid, name, transport).await });
        }
    }

    fn handle_search_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => self.search = None,
            KeyCode::Enter => {
                let sid = self
                    .search
                    .as_ref()
                    .and_then(|s| s.chosen())
                    .map(|h| h.session_id.clone());
                if let Some(sid) = sid {
                    self.search = None;
                    self.open_session_transcript(sid);
                }
            }
            KeyCode::Down => {
                if let Some(s) = self.search.as_mut() {
                    if !s.matched.is_empty() {
                        s.selected = (s.selected + 1).min(s.matched.len() - 1);
                    }
                }
            }
            KeyCode::Up => {
                if let Some(s) = self.search.as_mut() {
                    s.selected = s.selected.saturating_sub(1);
                }
            }
            KeyCode::Backspace => {
                if let Some(s) = self.search.as_mut() {
                    s.query.pop();
                    s.rematch();
                }
            }
            KeyCode::Char(c) => {
                if let Some(s) = self.search.as_mut() {
                    s.query.push(c);
                    s.rematch();
                }
            }
            _ => {}
        }
    }

    /// Jump to a session and show its transcript (content search lands on text,
    /// not the raw terminal).
    fn open_session_transcript(&mut self, sid: String) {
        self.open_single(sid, true);
        if let Some(open) = self.chat_session_id() {
            self.chat_mode = ChatMode::Transcript;
            self.term_attached = false;
            self.chat_follow = true;
            self.load_transcript(open);
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
            "search" | "grep" => self.open_search(),
            "model" => self.open_model_picker(),
            "perm" | "permission" => self.cycle_permission_mode(),
            "handoff" => self.open_handoff_picker(),
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
        let Some(sid) = self.open_session_id() else {
            return;
        };
        let Some(bytes) = crate::terminal::encode_key(&key) else {
            return;
        };
        let drv = self.driver();
        tokio::spawn(async move {
            let _ = drv.terminal_input(&sid, &bytes).await;
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
        let Some(agent) = self.target_agent() else {
            return;
        };
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
        let Some(form) = self.rename.take() else {
            return;
        };
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
        let Some(agent) = self.target_agent() else {
            return;
        };
        let cwd = agent.cwd_str().to_string();
        if cwd.is_empty() {
            self.set_toast("no working directory for notes");
            return;
        }
        let text = self.notes.get(&cwd).cloned().unwrap_or_default();
        self.notes_view = Some(NotesState {
            cwd,
            text,
            editing: false,
            scroll: 0,
        });
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
        let Some(n) = self.notes_view.as_ref() else {
            return;
        };
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
        let np = crate::app::SPAWN_PROVIDERS.len();
        let Some(form) = self.spawn_form.as_mut() else {
            return;
        };
        match key.code {
            KeyCode::Esc => self.spawn_form = None,
            KeyCode::Enter => self.submit_spawn(),
            // Shell-style path completion on the cwd field.
            KeyCode::Tab => complete_path(form),
            // ←/→ cycle the provider; ↑/↓ cycle the (claude) profile.
            KeyCode::Right => form.provider_idx = (form.provider_idx + 1) % np,
            KeyCode::Left => form.provider_idx = (form.provider_idx + np - 1) % np,
            KeyCode::Down => {
                if n > 0 {
                    form.profile_idx = (form.profile_idx + 1) % n;
                }
            }
            KeyCode::Up => {
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
            PaletteItem {
                label: "New agent".into(),
                hint: "spawn".into(),
                action: PaletteAction::NewAgent,
            },
            PaletteItem {
                label: "New terminal".into(),
                hint: "shell tab".into(),
                action: PaletteAction::NewTerminal,
            },
            PaletteItem {
                label: "Dashboard".into(),
                hint: "overview".into(),
                action: PaletteAction::Dashboard,
            },
        ];
        // Commands — the `:`-line verbs, so Ctrl-K is a real command palette.
        for (verb, desc) in COMMAND_PALETTE {
            items.push(PaletteItem {
                label: format!(": {verb}"),
                hint: (*desc).to_string(),
                action: PaletteAction::Command((*verb).to_string()),
            });
        }
        // Jump to a live agent (the full set, so the palette reaches agents the
        // `/` filter is hiding). The cwd goes in the hint so fuzzy search finds
        // an agent by its path, not just its short name.
        for a in &self.all_agents {
            if self.is_shell_session(&a.session_id) {
                continue; // shells live in their agent's tab bar, not here
            }
            items.push(PaletteItem {
                label: format!("Go to {}", self.agent_name(a)),
                hint: format!("{}  {}", a.state(), a.cwd_str()),
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
        let Some(p) = self.palette.as_mut() else {
            return;
        };
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
                let drv = self.driver();
                let bytes = bracketed_paste(&body);
                self.set_toast("Inserted");
                tokio::spawn(async move {
                    let _ = drv.terminal_input(&sid, &bytes).await;
                });
            }
            PaletteAction::SpawnWithPrompt(body) => self.open_spawn_with_prompt(body),
            PaletteAction::Command(cmd) => self.run_command(&cmd),
        }
    }

    // ── model / handoff picker ────────────────────────────────────────────

    /// Open the live model-switch picker for the target session. Managed
    /// sessions (codex/opencode/pi this TUI spawned) fetch their launchable
    /// models from the daemon; claude/unknown sessions get a free-text field
    /// (their model switches via a `/model` slash command on the message path).
    pub(super) fn open_model_picker(&mut self) {
        let Some(sid) = self.target_session() else {
            self.set_toast("no agent selected");
            return;
        };
        let provider = self.provider_for(&sid);
        let managed = provider != "claude";
        let cwd = self
            .target_agent()
            .and_then(|a| a.cwd.clone())
            .unwrap_or_default();
        self.picker = Some(Picker {
            title: format!("model · {provider}"),
            kind: PickerKind::Model {
                provider: provider.clone(),
                effort: None,
            },
            session_id: sid.clone(),
            query: String::new(),
            items: Vec::new(),
            matched: Vec::new(),
            selected: 0,
            pending: managed,
            allow_free_text: true,
        });
        if managed {
            let cm = self.claudemon.clone();
            let tx = self.tx.clone();
            tokio::spawn(async move {
                let models = cm
                    .provider_models(&provider, &cwd)
                    .await
                    .unwrap_or_default();
                let _ = tx.send(AppMsg::PickerModels {
                    session_id: sid,
                    models,
                });
            });
        }
    }

    /// Open the handoff provider chooser for the target session: pick who takes
    /// over, then build a brief and spawn that provider primed to read it.
    pub(super) fn open_handoff_picker(&mut self) {
        let Some(sid) = self.target_session() else {
            self.set_toast("no agent selected");
            return;
        };
        let Some(cwd) = self
            .target_agent()
            .and_then(|a| a.cwd.clone())
            .filter(|c| !c.is_empty())
        else {
            self.set_toast("no working directory for a handoff");
            return;
        };
        let items: Vec<PickerItem> = ["claude", "codex", "opencode", "pi"]
            .iter()
            .map(|p| PickerItem {
                id: (*p).to_string(),
                label: (*p).to_string(),
            })
            .collect();
        let mut picker = Picker {
            title: "hand off to".into(),
            kind: PickerKind::Handoff { cwd },
            session_id: sid,
            query: String::new(),
            items,
            matched: Vec::new(),
            selected: 0,
            pending: false,
            allow_free_text: false,
        };
        picker.rematch();
        self.picker = Some(picker);
    }

    /// Cycle the target session's permission mode one step and push it to the
    /// daemon. Managed sessions cycle ask⇄yolo; PTY (claude) sessions cycle
    /// default→acceptEdits→plan. A capability cliff (yolo→ask when spawned in
    /// bypass, opencode/pi) surfaces as a toast rather than crashing.
    pub(super) fn cycle_permission_mode(&mut self) {
        let Some(sid) = self.target_session() else {
            self.set_toast("no agent selected");
            return;
        };
        let managed = self.provider_for(&sid) != "claude";
        let cycle: &[&str] = if managed {
            &["ask", "yolo"]
        } else {
            &["default", "acceptEdits", "plan"]
        };
        let cur = self
            .perm_modes
            .get(&sid)
            .map(String::as_str)
            .unwrap_or(cycle[0]);
        let idx = cycle.iter().position(|m| *m == cur).unwrap_or(0);
        let next = cycle[(idx + 1) % cycle.len()].to_string();
        let drv = self.driver();
        let tx = self.tx.clone();
        let sid2 = sid.clone();
        tokio::spawn(async move {
            match drv.set_permission_mode(&sid2, &next).await {
                Ok(mode) => {
                    let _ = tx.send(AppMsg::PermissionMode {
                        session_id: sid2,
                        mode,
                    });
                }
                Err(e) => {
                    let _ = tx.send(AppMsg::Toast(format!("mode: {e}")));
                }
            }
        });
    }

    fn handle_picker_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => self.picker = None,
            KeyCode::Enter => self.submit_picker(),
            KeyCode::Down => {
                if let Some(p) = self.picker.as_mut() {
                    if !p.matched.is_empty() {
                        p.selected = (p.selected + 1).min(p.matched.len() - 1);
                    }
                }
            }
            KeyCode::Up => {
                if let Some(p) = self.picker.as_mut() {
                    p.selected = p.selected.saturating_sub(1);
                }
            }
            KeyCode::Backspace => {
                if let Some(p) = self.picker.as_mut() {
                    p.query.pop();
                    p.rematch();
                }
            }
            KeyCode::Char(c) => {
                if let Some(p) = self.picker.as_mut() {
                    p.query.push(c);
                    p.rematch();
                }
            }
            _ => {}
        }
    }

    /// Apply the picker's selection: switch the model, or run the handoff.
    pub(super) fn submit_picker(&mut self) {
        let Some(p) = self.picker.take() else { return };
        // A highlighted list row wins; free text (the model picker) is the fallback.
        let chosen_id = p.chosen().map(|it| it.id.clone());
        let Picker {
            kind,
            session_id,
            query,
            ..
        } = p;
        match kind {
            PickerKind::Model { provider, effort } => {
                let model = chosen_id.or_else(|| {
                    let q = query.trim();
                    (!q.is_empty()).then(|| q.to_string())
                });
                let Some(model) = model else {
                    self.set_toast("no model chosen");
                    return;
                };
                self.apply_model_switch(session_id, provider, effort, model);
            }
            PickerKind::Handoff { cwd } => {
                let Some(target) = chosen_id else { return };
                self.do_handoff(session_id, cwd, target);
            }
        }
    }

    /// Push a model switch: managed providers hit `POST /model`; claude/unknown
    /// sessions send a `/model <id>` slash command on the message path (their PTY
    /// 409s the endpoint).
    fn apply_model_switch(
        &mut self,
        sid: String,
        provider: String,
        effort: Option<String>,
        model: String,
    ) {
        let drv = self.driver();
        if provider == "claude" {
            let msg = format!("/model {model}");
            self.dispatch(
                "Model switch sent",
                async move { drv.message(&sid, &msg).await },
            );
        } else {
            self.dispatch("Model switched", async move {
                drv.set_model(&sid, Some(&model), effort.as_deref()).await
            });
        }
    }

    /// Build a handoff brief from `sid`, then spawn `target` (in `cwd`) primed to
    /// read it and continue — any harness → any harness. A claude successor seeds
    /// the brief into its composer (pasted, unsent) like the library-spawn flow;
    /// a managed successor receives it as its first message.
    fn do_handoff(&mut self, sid: String, cwd: String, target: String) {
        let drv = self.driver();
        let tx = self.tx.clone();
        let default_profile = self
            .profiles
            .iter()
            .find(|p| p.is_default)
            .or_else(|| self.profiles.first())
            .cloned();
        self.set_toast("Building handoff brief…");
        tokio::spawn(async move {
            let brief = match drv.handoff(&sid).await {
                Ok(b) => b,
                Err(e) => {
                    let _ = tx.send(AppMsg::Toast(format!("Handoff failed: {e}")));
                    return;
                }
            };
            let path = brief.path.unwrap_or_default();
            let prompt = format!(
                "You are taking over an in-progress session from another AI coding agent. \
                 First read the handoff brief at {path}, then continue the work from where it \
                 left off — don't start over or redo completed steps. Reply with a one-paragraph \
                 summary of the state and your next step."
            );
            if target == "claude" {
                let Some(profile) = default_profile else {
                    let _ = tx.send(AppMsg::Toast("no claude profile to hand off to".into()));
                    return;
                };
                match drv.spawn(cwd, &profile, None).await {
                    Ok(new_sid) => {
                        let _ = tx.send(AppMsg::Toast(format!("Handed off → {target}")));
                        seed_prompt(&drv.claudemon, &tx, &new_sid, &prompt).await;
                    }
                    Err(e) => {
                        let _ = tx.send(AppMsg::Toast(format!("Successor spawn failed: {e}")));
                    }
                }
            } else {
                match drv.spawn_managed(&target, &cwd, None, None, false).await {
                    Ok(new_sid) => {
                        let _ = tx.send(AppMsg::ManagedSpawned {
                            session_id: new_sid.clone(),
                            provider: target.clone(),
                        });
                        let _ = tx.send(AppMsg::Toast(format!("Handed off → {target}")));
                        // Managed adapters boot asynchronously; the message
                        // pipeline queues until the agent is ready, so this lands.
                        let _ = drv.message(&new_sid, &prompt).await;
                        fetch_agents(&drv.claudemon, &tx).await;
                    }
                    Err(e) => {
                        let _ = tx.send(AppMsg::Toast(format!("Successor spawn failed: {e}")));
                    }
                }
            }
        });
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
        let cur = if self.selected == 0 {
            n - 1
        } else {
            self.selected - 1
        };
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
        let Some(agent) = self.selected_agent() else {
            return;
        };
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
        self.workspaces
            .entry(id.clone())
            .or_insert_with(|| Workspace {
                tabs: vec![Tab {
                    title: "claude".into(),
                    session_id: id.clone(),
                    kind: TabKind::Claude,
                }],
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

    /// Pin or unpin the target agent — by its cwd, so the pin survives restarts
    /// (see `crate::pins`). Persists and rebuilds the live harpoon.
    pub(super) fn harpoon_toggle(&mut self) {
        let Some(cwd) = self.target_agent().map(|a| a.cwd_str().to_string()) else {
            self.set_toast("no agent to pin");
            return;
        };
        if cwd.is_empty() {
            self.set_toast("no working directory to pin");
            return;
        }
        if let Some(pos) = self.pinned_cwds.iter().position(|c| c == &cwd) {
            self.pinned_cwds.remove(pos);
            self.set_toast("Unpinned");
        } else {
            self.pinned_cwds.push(cwd);
            self.set_toast(format!("Pinned #{}", self.pinned_cwds.len()));
        }
        crate::pins::save(&self.pinned_cwds);
        self.rebuild_harpoon();
    }

    /// Teleport to the 1-based harpoon slot, if it's filled.
    pub(super) fn harpoon_jump(&mut self, slot: usize) {
        let Some(sid) = slot
            .checked_sub(1)
            .and_then(|i| self.harpoon.get(i))
            .cloned()
        else {
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
            self.set_toast(if delta < 0 {
                "start of jumps"
            } else {
                "end of jumps"
            });
            return;
        }
        self.jump_idx = target as usize;
        let id = self.jumplist[self.jump_idx].clone();
        self.open_single(id, false);
    }

    // ── window splits (panes) ─────────────────────────────────────────────

    /// First agent not already tiled, for bringing another agent into view.
    /// Uses the full set so splits can pull in agents the `/` filter hides, but
    /// skips shells (they aren't standalone agents).
    fn next_untiled_agent(&self) -> Option<String> {
        self.all_agents
            .iter()
            .map(|a| &a.session_id)
            .find(|sid| !self.tiles.contains(sid) && !self.is_shell_session(sid))
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
        self.pending_echo = None;
        self.invalidate_transcript_cache();
        let Some(tab) = self.active_tab().cloned() else {
            return;
        };
        // Headless stream sessions and known no-PTY sessions are proactively
        // transcript-only — never warm a PTY stream that can't exist.
        let transcript_only = tab.kind == TabKind::Claude
            && (self.no_terminal.contains(&tab.session_id)
                || self.is_stream_session(&tab.session_id));
        if transcript_only {
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
        let Some(id) = self.open_agent_id().map(|s| s.to_string()) else {
            return;
        };
        let cwd = self
            .chat_agent()
            .map(|a| a.cwd_str().to_string())
            .filter(|c| !c.is_empty())
            .or_else(|| {
                std::env::current_dir()
                    .ok()
                    .map(|p| p.display().to_string())
            })
            .unwrap_or_else(|| "/".into());
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".into());
        let cm = self.claudemon.clone();
        let tx = self.tx.clone();
        self.set_toast("Opening terminal…");
        tokio::spawn(async move {
            match cm.spawn(vec![shell], cwd, serde_json::Map::new(), "").await {
                Ok(sid) => {
                    let _ = tx.send(AppMsg::ShellSpawned {
                        agent_id: id,
                        session_id: sid,
                    });
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
        let Some(tab) = ws.active_tab().cloned() else {
            return;
        };
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
            provider_idx: 0,
            completions: Vec::new(),
            initial_prompt,
        });
    }

    /// Spawn a Claude session in the chosen cwd with the chosen profile, via
    /// claudemon's REST API. The new agent surfaces in the sidebar on the next
    /// state-change event (claudemon emits one once Claude starts up).
    pub(super) fn submit_spawn(&mut self) {
        let Some(form) = self.spawn_form.clone() else {
            return;
        };
        let cwd = profiles::normalize_cwd(&form.cwd);
        if cwd.is_empty() {
            self.set_toast("working directory required");
            return;
        }
        let provider = crate::app::SPAWN_PROVIDERS
            .get(form.provider_idx)
            .copied()
            .unwrap_or("claude");
        let initial_prompt = form.initial_prompt.clone();
        self.spawn_form = None;
        if provider == "claude" {
            let Some(profile) = self.profiles.get(form.profile_idx).cloned() else {
                self.set_toast("no profile selected");
                return;
            };
            self.spawn_agent_in(cwd, profile, initial_prompt, None);
        } else {
            self.spawn_managed_agent_in(provider, cwd, initial_prompt);
        }
    }

    /// Spawn a managed (Codex/OpenCode/Pi) agent via `/sessions/spawn-managed`,
    /// record its provider (so the model picker + permission-mode cycle pick the
    /// managed behaviour), and optionally seed a first prompt once it's up.
    pub(super) fn spawn_managed_agent_in(
        &self,
        provider: &str,
        cwd: String,
        initial_prompt: Option<String>,
    ) {
        let drv = self.driver();
        let tx = self.tx.clone();
        let provider = provider.to_string();
        tokio::spawn(async move {
            match drv.spawn_managed(&provider, &cwd, None, None, false).await {
                Ok(sid) => {
                    let _ = tx.send(AppMsg::ManagedSpawned {
                        session_id: sid.clone(),
                        provider: provider.clone(),
                    });
                    let _ = tx.send(AppMsg::Toast(format!("Spawned {provider} agent")));
                    fetch_agents(&drv.claudemon, &tx).await;
                    if let Some(prompt) = initial_prompt {
                        // Managed adapters boot async; the message pipeline queues
                        // until the agent is ready.
                        let _ = drv.message(&sid, &prompt).await;
                    }
                }
                Err(e) => {
                    let _ = tx.send(AppMsg::Toast(format!("Spawn failed: {e}")));
                }
            }
        });
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
        // The driver builds the argv (claudemon-direct) or hands the profile id to
        // the brain (bus mode); either way it pins/returns the session id.
        let resume = resume_session_id.is_some();
        let drv = self.driver();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let sid = match drv.spawn(cwd, &profile, resume_session_id).await {
                Ok(sid) => {
                    let verb = if resume { "Resumed" } else { "Spawned" };
                    let _ = tx.send(AppMsg::Toast(format!("{verb} agent")));
                    fetch_agents(&drv.claudemon, &tx).await;
                    sid
                }
                Err(e) => {
                    let _ = tx.send(AppMsg::Toast(format!("Spawn failed: {e}")));
                    return;
                }
            };
            if let Some(prompt) = initial_prompt {
                seed_prompt(&drv.claudemon, &tx, &sid, &prompt).await;
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
        let Some(agent) = self.target_agent() else {
            return;
        };
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
        self.pending_echo = None;
        self.turns.clear();
        self.invalidate_transcript_cache();
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
        let Some(agent) = self.target_agent() else {
            return;
        };
        if agent.approval().is_none() {
            return;
        }
        let sid = agent.session_id.clone();
        let drv = self.driver();
        let decision = decision.to_string();
        self.dispatch(ok, async move { drv.approve(&sid, &decision, None).await });
    }

    /// The target's pending questions as `(session id, questions)`, when any.
    fn target_questions(&self) -> Option<(String, Vec<crate::types::Question>)> {
        let agent = self.target_agent()?;
        let qs = agent.questions().filter(|q| !q.is_empty())?;
        Some((agent.session_id.clone(), qs.to_vec()))
    }

    /// The index of the question currently on screen (the flow's position, or
    /// the first question before any interaction). Only a flow that tracks
    /// this exact set counts — a stale flow for a superseded set must not
    /// position us on a question the user never stepped to.
    fn current_question_idx(&self, sid: &str, qs: &[crate::types::Question]) -> usize {
        self.question_flow
            .as_ref()
            .filter(|f| f.tracks(sid, qs))
            .map(|f| f.idx.min(qs.len() - 1))
            .unwrap_or(0)
    }

    /// Whether the question currently on screen is a multi-select (so Enter
    /// confirms toggles instead of falling through to the keymap).
    fn current_question_is_multiselect(&self) -> bool {
        let Some((sid, qs)) = self.target_questions() else {
            return false;
        };
        let idx = self.current_question_idx(&sid, &qs);
        qs[idx].multi_select
    }

    /// Whether Esc should step back to the previous question (mid-set only).
    fn question_can_step_back(&self) -> bool {
        let Some((sid, qs)) = self.target_questions() else {
            return false;
        };
        self.question_flow
            .as_ref()
            .is_some_and(|f| f.tracks(&sid, &qs) && f.idx > 0)
    }

    /// The stepper state for the pending set, created (or reset) on demand —
    /// including when the tracked set was superseded by a different one of
    /// the same length (its answers would otherwise leak into the new set).
    fn ensure_question_flow(
        &mut self,
        sid: &str,
        qs: &[crate::types::Question],
    ) -> &mut super::QuestionFlow {
        let stale = self
            .question_flow
            .as_ref()
            .is_none_or(|f| !f.tracks(sid, qs));
        if stale {
            self.question_flow = Some(super::QuestionFlow::new(sid.to_string(), qs));
        }
        self.question_flow.as_mut().expect("flow just ensured")
    }

    /// Record the current question's raw answer, then advance — or, after the
    /// last question, POST the whole set as `{answers: [raw…]}` (the daemon
    /// maps digit strings to labels on both transports and types them
    /// sequentially into the PTY picker).
    fn question_record_and_advance(
        &mut self,
        sid: &str,
        qs: &[crate::types::Question],
        raw: String,
    ) {
        let n = qs.len();
        let flow = self.ensure_question_flow(sid, qs);
        let idx = flow.idx.min(n - 1);
        flow.answers[idx] = Some(raw);
        if idx + 1 < n {
            flow.idx = idx + 1;
            return;
        }
        let answers: Vec<String> = flow
            .answers
            .iter()
            .map(|a| a.clone().unwrap_or_default())
            .collect();
        self.question_flow = None;
        let sid = sid.to_string();
        let drv = self.driver();
        self.dispatch(
            "Answered",
            async move { drv.answer_all(&sid, answers).await },
        );
    }

    /// Revisit the previous question (its recorded pick renders highlighted).
    fn question_back(&mut self) {
        if let Some(f) = self.question_flow.as_mut() {
            f.idx = f.idx.saturating_sub(1);
        }
    }

    /// Confirm the current multi-select question: the toggled labels, joined
    /// `", "`, become its raw answer. A single-question set answers
    /// immediately on the `{text}` fast path; mid-set it records and advances.
    fn question_confirm_multiselect(&mut self) {
        let Some((sid, qs)) = self.target_questions() else {
            return;
        };
        let n = qs.len();
        let idx = self.current_question_idx(&sid, &qs);
        let q = &qs[idx];
        if !q.multi_select {
            return;
        }
        let picks = self
            .question_flow
            .as_ref()
            .filter(|f| f.tracks(&sid, &qs))
            .map(|f| f.picks[idx].clone())
            .unwrap_or_default();
        let labels: Vec<String> = picks
            .iter()
            .filter_map(|&i| q.options.get(i))
            .map(|o| o.label.clone())
            .collect();
        if labels.is_empty() {
            self.set_toast("nothing selected — 1-9 toggle options");
            return;
        }
        let raw = labels.join(", ");
        if n == 1 {
            self.question_flow = None;
            let drv = self.driver();
            self.dispatch("Answered", async move { drv.answer_text(&sid, &raw).await });
            return;
        }
        self.question_record_and_advance(&sid, &qs, raw);
    }

    /// A digit key (1-9) against the pending question set: toggles an option
    /// of a multi-select, answers a single question immediately (the `{option}`
    /// fast path), or answers the current question of a multi-question set and
    /// advances the stepper.
    pub(super) fn answer_option(&mut self, c: char) {
        let Some((sid, qs)) = self.target_questions() else {
            return;
        };
        let n = qs.len();
        let digit = (c as u8 - b'1') as usize; // '1'..='9' → 0-based option
        let idx = self.current_question_idx(&sid, &qs);
        let q = &qs[idx];
        // In terminal mode no stepper renders (mirroring the Enter/Esc gate at
        // the top of handle_key), so digits must not drive an invisible flow —
        // recording hidden answers or toggling unseen checkboxes, then blind-
        // submitting the set. Only the single plain question keeps its
        // immediate `{option}` fast path: the daemon types that pick into the
        // PTY picker where the user can see it.
        if self.key_context() == Context::AgentTerminal && (n > 1 || q.multi_select) {
            self.set_toast("switch to transcript (t) to answer");
            return;
        }
        if q.multi_select {
            if digit < q.options.len() {
                let flow = self.ensure_question_flow(&sid, &qs);
                let set = &mut flow.picks[idx];
                if !set.remove(&digit) {
                    set.insert(digit);
                }
            }
            return;
        }
        // Out-of-range picks are ignored (when the option list is known).
        if !q.options.is_empty() && digit >= q.options.len() {
            return;
        }
        if n == 1 {
            // Single question keeps the immediate `{option}` fast path.
            self.question_flow = None;
            let option = (digit + 1) as u64;
            let drv = self.driver();
            self.dispatch(
                "Answered",
                async move { drv.answer_option(&sid, option).await },
            );
            return;
        }
        self.question_record_and_advance(&sid, &qs, (digit + 1).to_string());
    }

    pub(super) fn signal(&mut self, signal: &str, ok: &str) {
        let Some(sid) = self.target_session() else {
            return;
        };
        let drv = self.driver();
        let signal = signal.to_string();
        self.dispatch(ok, async move { drv.signal(&sid, &signal).await });
    }

    /// Send the composer's contents — as an answer if the agent is on a
    /// question, otherwise as a chat message. Mirrors the `/remote` heuristic.
    ///
    /// Free text against a multi-question set answers the CURRENT question and
    /// advances the stepper (submission happens after the last); a chat message
    /// echoes optimistically into the transcript until the refold carries it.
    pub(super) fn send_input(&mut self) {
        let text = self.input.trim().to_string();
        if text.is_empty() {
            return;
        }
        if let Some((sid, qs)) = self.target_questions() {
            self.input.clear();
            if qs.len() > 1 {
                self.question_record_and_advance(&sid, &qs, text);
                return;
            }
            self.question_flow = None;
            let drv = self.driver();
            self.dispatch(
                "Answered",
                async move { drv.answer_text(&sid, &text).await },
            );
            return;
        }
        let Some(agent) = self.target_agent() else {
            return;
        };
        let sid = agent.session_id.clone();
        self.input.clear();
        // Optimistic echo: render the message as a pending user turn now; the
        // refold that includes it (or a failure) retires it. Slash commands
        // don't echo — the daemon records them as filtered meta
        // (<command-name>…), so no refold could ever retire theirs.
        if !text.starts_with('/') {
            self.pending_echo = Some(text.clone());
            self.invalidate_transcript_cache();
        }
        let drv = self.driver();
        let cm = self.claudemon.clone();
        let tx = self.tx.clone();
        let transport = self.transport_for(&sid);
        tokio::spawn(async move {
            match drv.message(&sid, &text).await {
                Ok(_) => {
                    let _ = tx.send(AppMsg::Toast("Sent".into()));
                    fetch_agents(&cm, &tx).await;
                    super::tasks::fetch_transcript(&cm, &tx, sid, transport).await;
                }
                Err(e) => {
                    let _ = tx.send(AppMsg::SendFailed {
                        text,
                        error: e.to_string(),
                    });
                }
            }
        });
    }
}

/// Ctrl-C, the universal escape hatch — honored even while the help overlay is
/// up so the user can always quit.
fn is_ctrl_c(key: &KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c')
}

#[cfg(test)]
mod tests {
    //! Dispatch-seam tests for the top-level key handler. These drive real
    //! `KeyEvent`s through [`App::handle_key`] and assert on the synchronous
    //! state it leaves behind — normal-mode action resolution (counts, leader
    //! chords), the insert/filter/cmdline text modes, question-mode digit
    //! gating, and that open modals swallow keys instead of leaking them into
    //! global actions. Async effects (network sends) are fire-and-forget against
    //! a dead port, so we only assert the local state each path mutates.
    use super::*;
    use crate::app::ChatMode;
    use crate::claudemon::Claudemon;
    use crate::config::Config;
    use crate::types::Agent;

    fn test_app() -> App {
        // Redirect the config dir to a per-process temp dir: harpoon/rename/notes
        // dispatch persists to disk, and tests must never touch the real files.
        use std::sync::Once;
        static ONCE: Once = Once::new();
        ONCE.call_once(|| {
            let dir =
                std::env::temp_dir().join(format!("wks-tui-input-test-{}", std::process::id()));
            let _ = std::fs::create_dir_all(&dir);
            std::env::set_var("XDG_CONFIG_HOME", &dir);
        });
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let (ptx, _prx) = tokio::sync::mpsc::unbounded_channel();
        // An unused port: the background stream tasks fail and retry harmlessly.
        let cm = Claudemon::new("http://127.0.0.1:59999".into());
        App::new(cm, Vec::new(), Vec::new(), Config::default(), tx, ptx)
    }

    fn agent(id: &str, mode: &str) -> Agent {
        serde_json::from_value(serde_json::json!({
            "session_id": id, "cwd": format!("/work/{id}"), "mode": mode
        }))
        .unwrap()
    }

    /// An agent parked on a structured question (so `has_question()` is true).
    fn agent_asking(id: &str) -> Agent {
        serde_json::from_value(serde_json::json!({
            "session_id": id, "cwd": format!("/work/{id}"), "mode": "question",
            "pending": {"kind": "question", "questions": [
                {"question": "Which?", "options": [{"label": "A"}, {"label": "B"}]}
            ]}
        }))
        .unwrap()
    }

    /// An agent parked on a single multi-select question.
    fn agent_asking_multiselect(id: &str) -> Agent {
        serde_json::from_value(serde_json::json!({
            "session_id": id, "cwd": format!("/work/{id}"), "mode": "question",
            "pending": {"kind": "question", "questions": [
                {"question": "Choose", "multi_select": true,
                 "options": [{"label": "X"}, {"label": "Y"}]}
            ]}
        }))
        .unwrap()
    }

    /// An agent on a three-question set: a pick, a multi-select, and free text.
    fn agent_asking_many(id: &str) -> Agent {
        serde_json::from_value(serde_json::json!({
            "session_id": id, "cwd": format!("/work/{id}"), "mode": "question",
            "pending": {"kind": "question", "questions": [
                {"question": "Pick one",
                 "options": [{"label": "A"}, {"label": "B"}]},
                {"question": "Choose tools", "multi_select": true,
                 "options": [{"label": "X"}, {"label": "Y"}, {"label": "Z"}]},
                {"question": "Anything else?", "options": []}
            ]}
        }))
        .unwrap()
    }

    fn ch(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }
    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }
    fn code(k: KeyCode) -> KeyEvent {
        KeyEvent::new(k, KeyModifiers::NONE)
    }
    /// Feed each character of `s` as a separate key press.
    fn feed(app: &mut App, s: &str) {
        for c in s.chars() {
            app.handle_key(ch(c));
        }
    }
    /// A list app with `n` agents and the first one selected (row 1).
    fn app_with_agents(n: usize) -> App {
        let mut app = test_app();
        app.set_agents(
            (1..=n)
                .map(|i| agent(&format!("s{i}"), "responding"))
                .collect(),
        );
        app.selected = 1;
        app
    }

    // ── normal-mode key → Action resolution ─────────────────────────────────

    #[test]
    fn normal_q_quits() {
        let mut app = app_with_agents(2);
        app.handle_key(ch('q'));
        assert!(app.should_quit);
    }

    #[test]
    fn normal_j_k_navigate_the_sidebar() {
        let mut app = app_with_agents(3);
        app.selected = 0;
        app.handle_key(ch('j'));
        app.handle_key(ch('j'));
        assert_eq!(app.selected, 2, "two j's step down two rows");
        app.handle_key(ch('k'));
        assert_eq!(app.selected, 1, "k steps back up");
    }

    #[test]
    fn normal_g_and_shift_g_jump_first_and_last() {
        let mut app = app_with_agents(3);
        app.handle_key(ch('G'));
        assert_eq!(app.selected, 3, "G jumps to the last agent row");
        app.handle_key(ch('g'));
        assert_eq!(app.selected, 0, "g jumps to the first (dashboard) row");
    }

    #[test]
    fn ctrl_k_opens_palette_and_question_help_opens_help() {
        let mut app = app_with_agents(1);
        app.handle_key(ctrl('k'));
        assert!(app.palette.is_some(), "ctrl+k opens the command palette");
        app.palette = None;
        app.handle_key(ch('?'));
        assert!(app.help, "? opens the help overlay");
    }

    // ── vim counts ──────────────────────────────────────────────────────────

    #[test]
    fn count_prefix_repeats_a_motion_then_clears() {
        let mut app = app_with_agents(3);
        app.selected = 0;
        app.handle_key(ch('3'));
        assert_eq!(app.count, Some(3), "a leading digit accumulates a count");
        app.handle_key(ch('j'));
        assert_eq!(app.selected, 3, "3j steps down three rows");
        assert_eq!(app.count, None, "the count is consumed by the motion");
    }

    #[test]
    fn count_then_shift_g_jumps_to_that_agent() {
        let mut app = app_with_agents(3);
        feed(&mut app, "2");
        app.handle_key(ch('G'));
        assert_eq!(app.selected, 2, "2G jumps to agent #2");
    }

    #[test]
    fn leading_zero_does_not_start_a_count() {
        let mut app = app_with_agents(3);
        app.selected = 1;
        app.handle_key(ch('0'));
        assert_eq!(app.count, None, "a bare 0 never starts a count");
        assert_eq!(app.selected, 1, "and resolves to nothing here");
    }

    #[test]
    fn esc_abandons_a_pending_count() {
        let mut app = app_with_agents(2);
        app.handle_key(ch('3'));
        assert_eq!(app.count, Some(3));
        app.handle_key(code(KeyCode::Esc));
        assert_eq!(app.count, None, "esc clears the pending count");
        assert!(!app.should_quit, "and doesn't also fire Esc's own binding");
    }

    // ── leader / which-key chords ───────────────────────────────────────────

    #[test]
    fn leader_alone_is_pending_and_fires_nothing() {
        let mut app = app_with_agents(1);
        app.handle_key(ch(' '));
        assert_eq!(app.pending_keys.len(), 1, "leader is held pending");
        assert!(
            app.spawn_form.is_none() && !app.should_quit,
            "nothing fired yet"
        );
    }

    #[test]
    fn leader_a_opens_the_spawn_form() {
        let mut app = app_with_agents(1);
        feed(&mut app, " a");
        assert!(
            app.spawn_form.is_some(),
            "<leader> a is the new-agent chord"
        );
        assert!(
            app.pending_keys.is_empty(),
            "the chord resolved and cleared"
        );
    }

    #[tokio::test]
    async fn leader_slash_opens_cross_agent_search() {
        let mut app = app_with_agents(1);
        feed(&mut app, " /");
        assert!(app.search.is_some(), "<leader> / opens content search");
    }

    #[test]
    fn leader_digit_is_a_positional_harpoon_jump() {
        let mut app = app_with_agents(1);
        feed(&mut app, " 1");
        // No pins, so the jump only toasts — but the chord must be consumed and
        // must not fall through to a count or a global action.
        assert!(
            app.pending_keys.is_empty(),
            "the leader+digit chord is consumed"
        );
        assert_eq!(app.count, None);
        assert!(!app.should_quit);
    }

    #[test]
    fn count_before_harpoon_jump_does_not_leak_into_the_next_motion() {
        let mut app = app_with_agents(3);
        app.selected = 0;
        // Type a count, then a <leader><digit> harpoon teleport.
        app.handle_key(ch('3'));
        assert_eq!(app.count, Some(3), "the leading digit accumulates a count");
        feed(&mut app, " 2"); // <leader>2 — jump to harpoon slot 2 (no pins: toasts)
        assert!(
            app.pending_keys.is_empty(),
            "the leader+digit chord is consumed"
        );
        assert_eq!(
            app.count, None,
            "the harpoon jump must consume the pending count, not leak it \
             into the next motion"
        );
        // Prove the leak concretely: a following `j` must move exactly one row.
        app.handle_key(ch('j'));
        assert_eq!(
            app.selected, 1,
            "j after the jump moves one row, not the stale count of 3"
        );
    }

    #[test]
    fn esc_abandons_a_pending_leader_chord() {
        let mut app = app_with_agents(1);
        app.handle_key(ch(' '));
        assert_eq!(app.pending_keys.len(), 1);
        app.handle_key(code(KeyCode::Esc));
        assert!(
            app.pending_keys.is_empty(),
            "esc drops the half-typed chord"
        );
    }

    #[test]
    fn unknown_normal_key_is_a_harmless_noop() {
        let mut app = app_with_agents(2);
        app.handle_key(ch('z'));
        assert!(app.pending_keys.is_empty() && !app.should_quit && app.count.is_none());
    }

    // ── insert / compose mode routing ───────────────────────────────────────

    #[tokio::test]
    async fn i_enters_insert_mode_in_the_transcript() {
        let mut app = app_with_agents(1);
        app.open_agent();
        app.chat_mode = ChatMode::Transcript; // 'i' = Attach in terminal mode
        app.handle_key(ch('i'));
        assert!(
            app.insert_mode,
            "i enters compose mode in the transcript context"
        );
    }

    #[tokio::test]
    async fn insert_mode_typing_appends_to_the_composer() {
        let mut app = app_with_agents(1);
        app.open_agent();
        app.insert_mode = true;
        feed(&mut app, "hi");
        assert_eq!(
            app.input, "hi",
            "characters land in the composer, not the keymap"
        );
    }

    #[tokio::test]
    async fn insert_mode_esc_exits_to_normal() {
        let mut app = app_with_agents(1);
        app.open_agent();
        app.insert_mode = true;
        app.handle_key(code(KeyCode::Esc));
        assert!(!app.insert_mode, "esc leaves compose mode");
    }

    #[tokio::test]
    async fn insert_mode_enter_sends_and_clears_the_composer() {
        let mut app = app_with_agents(1);
        app.open_agent();
        app.insert_mode = true;
        feed(&mut app, "ship it");
        app.handle_key(code(KeyCode::Enter));
        assert!(
            app.input.is_empty(),
            "enter dispatches the message and clears the buffer"
        );
    }

    // ── `/` sidebar filter entry / exit ─────────────────────────────────────

    #[test]
    fn slash_opens_the_filter_and_typing_edits_it() {
        let mut app = app_with_agents(2);
        app.handle_key(ch('/'));
        assert!(app.filter_editing, "/ starts editing the sidebar filter");
        feed(&mut app, "s1");
        assert_eq!(
            app.filter.as_deref(),
            Some("s1"),
            "characters extend the query"
        );
        app.handle_key(code(KeyCode::Backspace));
        assert_eq!(app.filter.as_deref(), Some("s"), "backspace trims it");
    }

    #[test]
    fn filter_enter_keeps_it_and_esc_clears_it() {
        let mut app = app_with_agents(2);
        app.handle_key(ch('/'));
        feed(&mut app, "s1");
        app.handle_key(code(KeyCode::Enter));
        assert!(!app.filter_editing, "enter stops editing");
        assert_eq!(app.filter.as_deref(), Some("s1"), "but keeps the filter");

        app.handle_key(ch('/'));
        app.handle_key(code(KeyCode::Esc));
        assert!(
            !app.filter_editing && app.filter.is_none(),
            "esc clears the filter entirely"
        );
    }

    // ── `:` ex-command line entry / exit ────────────────────────────────────

    #[test]
    fn colon_opens_the_cmdline() {
        let mut app = app_with_agents(1);
        app.handle_key(ch(':'));
        assert_eq!(
            app.cmdline.as_deref(),
            Some(""),
            ": opens an empty command line"
        );
    }

    #[test]
    fn cmdline_esc_cancels_without_running() {
        let mut app = app_with_agents(1);
        app.handle_key(ch(':'));
        feed(&mut app, "q");
        app.handle_key(code(KeyCode::Esc));
        assert!(app.cmdline.is_none(), "esc closes the command line");
        assert!(!app.should_quit, "and the typed command never ran");
    }

    #[test]
    fn cmdline_enter_runs_the_command() {
        let mut app = app_with_agents(1);
        app.handle_key(ch(':'));
        feed(&mut app, "q");
        app.handle_key(code(KeyCode::Enter));
        assert!(app.cmdline.is_none(), "enter closes the command line");
        assert!(app.should_quit, ":q quits");
    }

    // ── question-mode digit gating ──────────────────────────────────────────

    #[tokio::test]
    async fn digit_answers_a_pending_question_instead_of_starting_a_count() {
        let mut app = test_app();
        app.set_agents(vec![agent_asking("s1")]);
        app.selected = 1;
        app.handle_key(ch('3'));
        // With a question up, 1-9 answer it positionally — no count is started.
        assert_eq!(
            app.count, None,
            "a digit does not accumulate a count while a question is up"
        );
        assert!(app.pending_keys.is_empty());
    }

    #[test]
    fn digit_starts_a_count_when_no_question_is_pending() {
        let mut app = app_with_agents(2);
        app.handle_key(ch('3'));
        assert_eq!(
            app.count,
            Some(3),
            "without a question, the same digit starts a count"
        );
    }

    // ── question stepper: multi-question + multi-select routing ─────────────

    #[tokio::test]
    async fn multi_question_digits_answer_and_advance() {
        let mut app = test_app();
        app.set_agents(vec![agent_asking_many("s1")]);
        app.selected = 1;

        // Q1 (single pick): a digit records the 1-indexed raw and advances.
        app.handle_key(ch('2'));
        let flow = app.question_flow.as_ref().expect("flow started");
        assert_eq!(flow.idx, 1, "advanced to Q2");
        assert_eq!(flow.answers[0].as_deref(), Some("2"));
        assert_eq!(app.count, None, "digit answered, never a vim count");
    }

    #[tokio::test]
    async fn multiselect_digits_toggle_and_enter_confirms() {
        let mut app = test_app();
        app.set_agents(vec![agent_asking_many("s1")]);
        app.selected = 1;
        app.handle_key(ch('1')); // Q1 answered → Q2 (multi-select)

        // Digits TOGGLE options — no advance.
        app.handle_key(ch('1'));
        app.handle_key(ch('3'));
        {
            let flow = app.question_flow.as_ref().unwrap();
            assert_eq!(flow.idx, 1, "toggles don't advance");
            assert!(flow.picks[1].contains(&0) && flow.picks[1].contains(&2));
        }
        app.handle_key(ch('3')); // toggle Z back off
        assert!(!app.question_flow.as_ref().unwrap().picks[1].contains(&2));

        // Enter confirms: the chosen labels joined ", " become the raw answer.
        app.handle_key(ch('2')); // also pick Y → X + Y
        app.handle_key(code(KeyCode::Enter));
        let flow = app.question_flow.as_ref().unwrap();
        assert_eq!(flow.idx, 2, "confirmed and advanced to Q3");
        assert_eq!(flow.answers[1].as_deref(), Some("X, Y"));
    }

    #[tokio::test]
    async fn multiselect_enter_with_nothing_selected_toasts() {
        let mut app = test_app();
        app.set_agents(vec![agent_asking_many("s1")]);
        app.selected = 1;
        app.handle_key(ch('1')); // → Q2 (multi-select), nothing toggled
        app.handle_key(code(KeyCode::Enter));
        let flow = app.question_flow.as_ref().unwrap();
        assert_eq!(flow.idx, 1, "empty confirm doesn't advance");
        assert_eq!(app.toast(), Some("nothing selected — 1-9 toggle options"));
    }

    #[tokio::test]
    async fn esc_revisits_the_previous_question_mid_set() {
        let mut app = test_app();
        app.set_agents(vec![agent_asking_many("s1")]);
        app.selected = 1;
        app.handle_key(ch('1')); // Q1 → Q2
        app.handle_key(code(KeyCode::Esc));
        let flow = app.question_flow.as_ref().unwrap();
        assert_eq!(flow.idx, 0, "esc steps back");
        assert_eq!(
            flow.answers[0].as_deref(),
            Some("1"),
            "the recorded pick is kept (renders highlighted)"
        );
        // Re-answering overwrites and advances again.
        app.handle_key(ch('2'));
        let flow = app.question_flow.as_ref().unwrap();
        assert_eq!(flow.idx, 1);
        assert_eq!(flow.answers[0].as_deref(), Some("2"));
    }

    #[tokio::test]
    async fn free_text_answers_the_current_question_and_last_answer_submits() {
        let mut app = test_app();
        app.set_agents(vec![agent_asking_many("s1")]);
        app.selected = 1;
        app.handle_key(ch('1')); // Q1 → Q2

        // Free text mid-set answers the CURRENT question and advances (it
        // must not submit the whole set yet).
        app.input = "custom tools".into();
        app.send_input();
        {
            let flow = app.question_flow.as_ref().unwrap();
            assert_eq!(flow.idx, 2, "advanced, not submitted");
            assert_eq!(flow.answers[1].as_deref(), Some("custom tools"));
        }

        // Answering the final question submits {answers:[…]} and clears the flow.
        app.input = "nothing else".into();
        app.send_input();
        assert!(
            app.question_flow.is_none(),
            "the whole set posted and the stepper reset"
        );
    }

    #[tokio::test]
    async fn single_question_keeps_the_immediate_fast_path() {
        let mut app = test_app();
        app.set_agents(vec![agent_asking("s1")]);
        app.selected = 1;
        app.handle_key(ch('1'));
        assert!(
            app.question_flow.is_none(),
            "a single pick answers immediately — no stepper survives"
        );
    }

    #[tokio::test]
    async fn enter_still_opens_an_agent_on_a_single_select_question() {
        // The Enter interception only applies to multi-select questions; a
        // plain question must not shadow Enter = OpenAgent in the list.
        let mut app = test_app();
        app.set_agents(vec![agent_asking("s1")]);
        app.selected = 1;
        app.handle_key(code(KeyCode::Enter));
        assert!(
            matches!(&app.view, crate::app::View::Agent { id } if id == "s1"),
            "enter opened the agent"
        );
    }

    #[tokio::test]
    async fn enter_still_attaches_in_terminal_mode_on_a_multiselect_question() {
        // No stepper renders in terminal chat mode (its footer advertises
        // "enter to attach") — Enter must attach, not confirm invisible
        // toggles. The list detail pane and the transcript keep the stepper.
        let mut app = test_app();
        app.set_agents(vec![agent_asking_multiselect("s1")]);
        app.selected = 1;
        app.handle_key(ch('l')); // open via 'l' (Enter confirms from the list)
        assert!(matches!(&app.view, crate::app::View::Agent { id } if id == "s1"));
        assert_eq!(app.chat_mode, crate::app::ChatMode::Terminal);

        app.handle_key(code(KeyCode::Enter));
        assert!(app.term_attached(), "enter attached to the terminal");
        assert_eq!(app.toast(), None, "no 'nothing selected' toast");
    }

    #[tokio::test]
    async fn terminal_mode_digits_do_not_drive_the_invisible_stepper() {
        // In terminal chat mode no stepper renders — a digit against a
        // multi-question set must not silently record an answer and advance
        // toward a blind submit. It toasts the way out instead.
        let mut app = test_app();
        app.set_agents(vec![agent_asking_many("s1")]);
        app.selected = 1;
        app.handle_key(ch('l')); // open via 'l' → ChatMode::Terminal, unattached
        assert!(matches!(&app.view, crate::app::View::Agent { id } if id == "s1"));
        assert_eq!(app.chat_mode, crate::app::ChatMode::Terminal);

        app.handle_key(ch('1'));
        assert!(
            app.question_flow.is_none(),
            "no hidden answer was recorded in terminal mode"
        );
        assert_eq!(app.toast(), Some("switch to transcript (t) to answer"));
    }

    #[tokio::test]
    async fn terminal_mode_digits_do_not_toggle_invisible_multiselect_boxes() {
        // A single multi-select question is just as invisible in terminal
        // mode, and Enter there means attach — so the toggles could never be
        // confirmed. Digits must not accumulate hidden picks.
        let mut app = test_app();
        app.set_agents(vec![agent_asking_multiselect("s1")]);
        app.selected = 1;
        app.handle_key(ch('l'));
        assert_eq!(app.chat_mode, crate::app::ChatMode::Terminal);

        app.handle_key(ch('1'));
        assert!(
            app.question_flow.is_none(),
            "no invisible checkbox was toggled"
        );
        assert_eq!(app.toast(), Some("switch to transcript (t) to answer"));
    }

    #[tokio::test]
    async fn terminal_mode_single_question_keeps_the_visible_fast_path() {
        // One plain question keeps the immediate `{option}` POST: the daemon
        // types the pick into the PTY picker, which the terminal pane shows.
        let mut app = test_app();
        app.set_agents(vec![agent_asking("s1")]);
        app.selected = 1;
        app.handle_key(ch('l'));
        assert_eq!(app.chat_mode, crate::app::ChatMode::Terminal);

        app.handle_key(ch('1'));
        assert!(app.question_flow.is_none(), "fast path never opens a flow");
        assert_ne!(
            app.toast(),
            Some("switch to transcript (t) to answer"),
            "the single-question answer is not blocked"
        );
    }

    #[tokio::test]
    async fn a_slash_command_send_does_not_leave_a_pending_echo() {
        // Slash commands are stored as filtered meta (<command-name>…), so a
        // refold could never retire their echo — they don't echo at all.
        let mut app = test_app();
        app.set_agents(vec![agent("s1", "responding")]);
        app.selected = 1;
        app.input = "/compact".into();
        app.send_input();
        assert!(app.pending_echo.is_none(), "no ghost '…sending' turn");
        assert!(app.input.is_empty(), "the composer still clears");
    }

    // ── open modals swallow keys (no leak to global actions) ────────────────

    #[test]
    fn keys_in_the_picker_modal_do_not_reach_global_actions() {
        let mut app = app_with_agents(1);
        app.open_model_picker();
        assert!(app.picker.is_some());
        app.handle_key(ch('q')); // would Quit in the List context
        assert!(!app.should_quit, "the picker captures the key");
        assert!(app.picker.is_some(), "and stays open");
        app.handle_key(code(KeyCode::Esc));
        assert!(app.picker.is_none(), "esc closes the picker");
    }

    #[tokio::test]
    async fn keys_in_the_search_modal_do_not_quit() {
        let mut app = app_with_agents(1);
        app.open_search();
        assert!(app.search.is_some());
        app.handle_key(ch('q'));
        assert!(
            !app.should_quit,
            "typing in search never triggers the global quit"
        );
        assert_eq!(
            app.search.as_ref().map(|s| s.query.as_str()),
            Some("q"),
            "the key lands in the search query instead"
        );
    }

    #[test]
    fn keys_in_the_spawn_form_do_not_leak_to_global_actions() {
        let mut app = app_with_agents(1);
        app.open_spawn();
        assert!(app.spawn_form.is_some());
        app.handle_key(ch('q'));
        assert!(!app.should_quit, "the spawn form captures keys");
        assert!(app.spawn_form.is_some());
    }
}
