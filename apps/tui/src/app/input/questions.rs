//! Answering a pending approval or question set: the target lookups, the
//! multi-question stepper, and multi-select confirmation.

use super::*;

impl App {
    /// The agent a key action targets: the chat's agent when in chat, else the
    /// selected sidebar agent.
    pub(in crate::app) fn target_session(&self) -> Option<String> {
        match &self.view {
            View::Agent { .. } => self.chat_session_id(),
            View::List => self.selected_agent().map(|a| a.session_id.clone()),
        }
    }

    pub(in crate::app) fn target_agent(&self) -> Option<&crate::types::Agent> {
        match &self.view {
            View::Agent { .. } => self.chat_agent(),
            View::List => self.selected_agent(),
        }
    }

    /// Whether the targeted agent has a pending question — if so, `1`-`9` answer
    /// it rather than starting a vim count.
    pub(in crate::app) fn target_has_question(&self) -> bool {
        self.target_agent().is_some_and(|a| a.has_question())
    }

    pub(in crate::app) fn approve(&mut self, decision: &str, ok: &str) {
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
    pub(in crate::app) fn target_questions(&self) -> Option<(String, Vec<crate::types::Question>)> {
        let agent = self.target_agent()?;
        let qs = agent.questions().filter(|q| !q.is_empty())?;
        Some((agent.session_id.clone(), qs.to_vec()))
    }

    /// The index of the question currently on screen (the flow's position, or
    /// the first question before any interaction). Only a flow that tracks
    /// this exact set counts — a stale flow for a superseded set must not
    /// position us on a question the user never stepped to.
    pub(in crate::app) fn current_question_idx(
        &self,
        sid: &str,
        qs: &[crate::types::Question],
    ) -> usize {
        self.question_flow
            .as_ref()
            .filter(|f| f.tracks(sid, qs))
            .map(|f| f.idx.min(qs.len() - 1))
            .unwrap_or(0)
    }

    /// Whether the question currently on screen is a multi-select (so Enter
    /// confirms toggles instead of falling through to the keymap).
    pub(in crate::app) fn current_question_is_multiselect(&self) -> bool {
        let Some((sid, qs)) = self.target_questions() else {
            return false;
        };
        let idx = self.current_question_idx(&sid, &qs);
        qs[idx].multi_select
    }

    /// Whether Esc should step back to the previous question (mid-set only).
    pub(in crate::app) fn question_can_step_back(&self) -> bool {
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
    pub(in crate::app) fn ensure_question_flow(
        &mut self,
        sid: &str,
        qs: &[crate::types::Question],
    ) -> &mut crate::app::QuestionFlow {
        let stale = self
            .question_flow
            .as_ref()
            .is_none_or(|f| !f.tracks(sid, qs));
        if stale {
            self.question_flow = Some(crate::app::QuestionFlow::new(sid.to_string(), qs));
        }
        self.question_flow.as_mut().expect("flow just ensured")
    }

    /// Record the current question's raw answer, then advance — or, after the
    /// last question, POST the whole set as `{answers: [raw…]}` (the daemon
    /// maps digit strings to labels on both transports and types them
    /// sequentially into the PTY picker).
    pub(in crate::app) fn question_record_and_advance(
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
    pub(in crate::app) fn question_back(&mut self) {
        if let Some(f) = self.question_flow.as_mut() {
            f.idx = f.idx.saturating_sub(1);
        }
    }

    /// Confirm the current multi-select question: the toggled labels, joined
    /// `", "`, become its raw answer. A single-question set answers
    /// immediately on the `{text}` fast path; mid-set it records and advances.
    pub(in crate::app) fn question_confirm_multiselect(&mut self) {
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
    pub(in crate::app) fn answer_option(&mut self, c: char) {
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
}

#[cfg(test)]
mod tests {
    use crate::app::input::testutil::*;
    use crossterm::event::KeyCode;

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
}
