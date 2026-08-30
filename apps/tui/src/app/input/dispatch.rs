//! The top of the key funnel: which context a key belongs to, the key →
//! action resolution, and the two paths a keystroke can leave by — a signal or
//! a message send.

use super::*;

impl App {
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
        // The remote-nodes overlay captures keys while it's open, so `w` means
        // wake here and nothing else — and so the confirmation step can own
        // EVERY key while it is armed.
        if self.nodes_view.is_some() {
            self.handle_nodes_key(key);
            return;
        }
        // A docked pane owns the keys while it holds focus. Ctrl-w h/l moves focus
        // back to the chat, so the binding that got you here also gets you out.
        if self.side_focused() {
            // Ctrl-w alone hands focus back — a docked pane is one window, so
            // there is no direction to pick, and the pane's own keys stay free.
            if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('w') {
                self.focus_side(false);
                return;
            }
            if let Some(crate::app::SideKind::Changes) = self.side.as_ref().map(|p| p.kind) {
                self.handle_changes_key(key);
                return;
            }
        }
        // The runs overlay is read-only — it only needs a way out.
        if self.runs_open.is_some() {
            match key.code {
                KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('h') | KeyCode::Char('W') => {
                    self.close_runs();
                }
                // A manual nudge, for when you don't want to wait for the tick.
                KeyCode::Char('r') => self.refresh_runs(),
                _ => {}
            }
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
        let ctxs = [Context::Global, self.key_context()];
        // Leader + a digit teleports to a harpoon slot — positional, like the
        // answer keys, so it isn't nine separate keymap entries. A user-defined
        // `<leader> <digit>` override still wins, though: only intercept when no
        // explicit keymap binding claims the sequence.
        if self.pending_keys.len() == 2
            && self.pending_keys[0] == self.keymap.leader()
            && matches!(
                self.keymap.resolve(&ctxs, &self.pending_keys),
                KeyMatch::None
            )
        {
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
                // Abandon any pending vim count too, so it can't leak into the
                // next motion (mirrors dispatch_action's `self.count.take()`
                // and the harpoon-jump reset above).
                self.count = None;
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
    pub(in crate::app) fn dispatch_action(&mut self, action: Action) {
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
                // `select_next` saturates at `agents.len()`, so cap the repeat
                // count to the number of rows — a huge typed count must not
                // spin a billions-of-iterations busy loop on the UI thread.
                for _ in 0..n.min(self.agents.len()) {
                    self.select_next();
                }
            }
            SelectPrev => {
                // `select_prev` saturates at 0, so at most `selected` steps do
                // any work; clamp the loop to that.
                for _ in 0..n.min(self.selected) {
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
            OpenRuns => self.open_runs(),
            OpenChanges => self.toggle_side(crate::app::SideKind::Changes),
            OpenNotes => self.open_notes(),
            RemoteNodes => self.open_nodes(),
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

    pub(in crate::app) fn handle_insert_key(&mut self, key: KeyEvent) {
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

    pub(in crate::app) fn signal(&mut self, signal: &str, ok: &str) {
        let Some(sid) = self.target_session() else {
            return;
        };
        if self.blocked_by_offline_hub(&sid) {
            return;
        }
        // `driver_for`: remote sessions signal via `hub:<peer>/claude.signal`.
        let drv = self.driver_for(&sid);
        let signal = signal.to_string();
        self.dispatch(ok, async move { drv.signal(&sid, &signal).await });
    }

    /// Send the composer's contents — as an answer if the agent is on a
    /// question, otherwise as a chat message. Mirrors the `/remote` heuristic.
    ///
    /// Free text against a multi-question set answers the CURRENT question and
    /// advances the stepper (submission happens after the last); a chat message
    /// echoes optimistically into the transcript until the refold carries it.
    pub(in crate::app) fn send_input(&mut self) {
        let text = self.input.trim().to_string();
        if text.is_empty() {
            return;
        }
        if let Some((sid, qs)) = self.target_questions() {
            if self.blocked_by_offline_hub(&sid) {
                return;
            }
            self.input.clear();
            if qs.len() > 1 {
                self.question_record_and_advance(&sid, &qs, text);
                return;
            }
            self.question_flow = None;
            // `driver_for`: remotely, answer text rides the message path.
            let drv = self.driver_for(&sid);
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
        if self.blocked_by_offline_hub(&sid) {
            return;
        }
        self.input.clear();
        // Optimistic echo: render the message as a pending user turn now; the
        // refold that includes it (or a failure) retires it. Slash commands
        // don't echo — the daemon records them as filtered meta
        // (<command-name>…), so no refold could ever retire theirs.
        if !text.starts_with('/') {
            self.pending_echo = Some(text.clone());
            self.invalidate_transcript_cache();
        }
        // `driver_for`: a remote chat sends via `hub:<peer>/agents.sendMessage`.
        let drv = self.driver_for(&sid);
        let cm = self.claudemon.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let remote = drv.hub.is_some();
            match drv.message(&sid, &text).await {
                Ok(_) => {
                    let _ = tx.send(AppMsg::Toast("Sent".into()));
                    fetch_agents(&cm, &tx).await;
                    // The delta feed carries the echo and the reply; this just
                    // closes the gap if the send landed before we subscribed.
                    // Remotely there is no delta feed at all: the round trip
                    // through the federation link IS the refresh.
                    if remote {
                        if let (Some(bus), Some(hub)) = (drv.bus.clone(), drv.hub.clone()) {
                            crate::federation::fetch_remote_conversation(bus, hub, sid, tx).await;
                        }
                    } else {
                        crate::app::tasks::fetch_transcript(&cm, &tx, sid).await;
                    }
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
pub(super) fn is_ctrl_c(key: &KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::input::testutil::*;
    use crossterm::event::KeyCode;

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

    #[test]
    fn huge_count_motion_is_bounded_not_a_busy_loop() {
        use std::time::Instant;
        let mut app = app_with_agents(3);
        app.selected = 0;
        // A pathologically large vim count — as if the user held the digit
        // prefix on auto-repeat before a `j` motion. `select_next` already
        // saturates at `agents.len()`, so the repeat loop must clamp its
        // iteration count to the number of rows instead of literally spinning
        // billions of times on the UI thread.
        app.count = Some(5_000_000_000);
        let start = Instant::now();
        app.handle_key(ch('j'));
        let elapsed = start.elapsed();
        assert_eq!(app.selected, 3, "selection still clamps to the last row");
        assert!(
            elapsed.as_secs() < 1,
            "a huge count must not spin a busy loop (took {elapsed:?})"
        );
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
    fn user_leader_digit_override_beats_the_harpoon_jump() {
        let mut app = app_with_agents(1);
        // A user override: `<leader> 5` -> quit, stored in the Global table.
        assert!(app.keymap.set(Context::Global, "<leader> 5", "quit"));
        feed(&mut app, " 5");
        assert!(
            app.should_quit,
            "a configured <leader> 5 binding must fire, not the positional harpoon jump"
        );
        assert!(
            app.pending_keys.is_empty(),
            "the chord resolved and cleared"
        );
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
    fn count_before_a_dead_end_key_does_not_leak_into_the_next_motion() {
        let mut app = app_with_agents(3);
        app.selected = 0;
        // Type a count, then an unbound / dead-end key (`z` is not a default
        // binding in Global or List), which resolves to KeyMatch::None.
        app.handle_key(ch('3'));
        assert_eq!(app.count, Some(3), "the leading digit accumulates a count");
        app.handle_key(ch('z')); // dead end — clears pending_keys
        assert!(
            app.pending_keys.is_empty(),
            "the dead-end key clears the pending sequence"
        );
        assert_eq!(
            app.count, None,
            "an interrupting dead-end key must abandon the pending count, \
             not leak it into the next motion"
        );
        // Prove the leak concretely: a following `j` must move exactly one row,
        // not the stale count of 3.
        app.handle_key(ch('j'));
        assert_eq!(
            app.selected, 1,
            "j after the dead-end key moves one row, not the stale count of 3"
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

    // ── window splits ───────────────────────────────────────────────────────
    //
    // The dispatch tests above drive keys; these call the pane and navigation
    // mutators directly, because their interesting cases (the tile cap, the
    // wrap, the last-pane-closes-the-view rule) are reached through state that
    // is fiddly to set up a keystroke at a time.
}
