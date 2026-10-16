//! Card-bound decisions, "Always allow" options, MCP elicitations, thinking
//! and compaction rows, per-message history controls, the Session page's
//! context/Codex/MCP cards and the title capsule's status chips.
use super::*;
use serde_json::json;
use wks_native::codex_extras::{GoalChange, ReviewTarget};

const UUID: &str = "6f1c2e1a-0b5e-4bd8-9f7e-2d6a8a1b9c3d";

/// Session "a" of `provider` on `transport`, with `items` as its chat and
/// `row` merged into its snapshot.
fn chat(provider: &str, transport: &str, items: serde_json::Value, row: serde_json::Value) -> View {
    let mut view = state("a");
    let session = &mut Arc::make_mut(&mut view.sessions)[0];
    session.provider = provider.into();
    session.transport = transport.into();
    session.merge(&row);
    let items: Vec<Item> = serde_json::from_value(items).unwrap();
    view.transcript.snapshot(ConversationSnapshot {
        seq: items.len() as u64,
        first_seq: 1,
        items,
    });
    view
}

fn show(workspace: &Entity<Workspace>, visual: &mut VisualTestContext, view: View) {
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| this.update_view(Arc::new(view), window, cx))
    });
    visual.run_until_parked();
    visual.update(|window, _| window.refresh());
    visual.run_until_parked();
}

/// A click, then a fresh frame for the next lookup.
fn tap(visual: &mut VisualTestContext, selector: &'static str) {
    click(visual, selector);
    visual.update(|window, _| window.refresh());
    visual.run_until_parked();
}

fn acts(commands: &mut tokio::sync::mpsc::Receiver<Command>) -> Vec<Action> {
    effects(commands)
        .into_iter()
        .filter_map(|c| match c {
            Command::Act { action, .. } => Some(action),
            _ => None,
        })
        .collect()
}

fn requests(commands: &mut tokio::sync::mpsc::Receiver<Command>) -> Vec<Request> {
    effects(commands)
        .into_iter()
        .filter_map(|c| match c {
            Command::Request(request) => Some(request),
            _ => None,
        })
        .collect()
}

#[gpui::test]
fn approvals_offer_each_always_option_in_its_own_words_bound_to_the_card(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.simulate_resize(size(px(1200.), px(800.)));
    let view = chat(
        "claude",
        "stream",
        json!([]),
        json!({"pendingRequestId":"req-1","pendingApproval":{"toolName":"Bash",
            "toolInput":{"command":"npm test"},"requestId":"req-1","alwaysOptions":[
                {"id":"req-1:0","label":"Always allow Bash(npm test:*) this session","scope":"session","kind":"addRules"},
                {"id":"req-1:1","label":"Allow all edits this session","scope":"session","kind":"setMode"}]}}),
    );
    show(&workspace, &mut visual, view);
    effects(&mut commands);
    assert!(visual.debug_bounds("always-option-0").is_some());
    assert!(visual.debug_bounds("always-option-1").is_some());
    assert!(visual.debug_bounds("always-option-2").is_none());
    tap(&mut visual, "always-option-0");
    match acts(&mut commands).as_slice() {
        [
            Action::AlwaysAllow {
                option,
                label,
                card,
            },
        ] => {
            assert_eq!(option, "req-1:0");
            assert_eq!(card, "req-1");
            assert_eq!(label, "Always allow Bash(npm test:*) this session");
            let (method, params) = Action::AlwaysAllow {
                option: option.clone(),
                label: label.clone(),
                card: card.clone(),
            }
            .wire("a");
            assert_eq!(method, "claude.approve");
            assert_eq!(
                params,
                json!({"sessionId":"a","decision":"yes","alwaysOption":"req-1:0","requestId":"req-1"})
            );
        }
        other => panic!("expected one always-allow, got {other:?}"),
    }
    tap(&mut visual, "approve");
    assert!(matches!(acts(&mut commands).as_slice(),
        [Action::Approve(true, Some(card))] if card == "req-1"));
    tap(&mut visual, "deny");
    assert!(matches!(acts(&mut commands).as_slice(),
        [Action::Approve(false, Some(card))] if card == "req-1"));
    // Allow once never carries a grant; legacy "always" is never sent.
    let (_, once) = Action::Approve(true, Some("req-1".into())).wire("a");
    assert_eq!(
        once,
        json!({"sessionId":"a","decision":"yes","requestId":"req-1"})
    );
}

#[gpui::test]
fn codex_approvals_take_a_deny_note_too(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.simulate_resize(size(px(1200.), px(800.)));
    let view = chat(
        "codex",
        "stream",
        json!([]),
        json!({"pendingRequestId":"7","pendingApproval":{"toolName":"exec",
            "toolInput":{"command":"rm -rf build"},"requestId":"7"}}),
    );
    show(&workspace, &mut visual, view);
    effects(&mut commands);
    assert!(visual.debug_bounds("always-option-0").is_none());
    tap(&mut visual, "deny-with-note");
    visual.simulate_input("use cargo clean");
    tap(&mut visual, "deny-note-send");
    assert!(matches!(acts(&mut commands).as_slice(),
        [Action::DenyWithNote(note, Some(card))] if note == "use cargo clean" && card == "7"));
}

#[gpui::test]
fn thinking_starts_collapsed_and_compaction_reads_as_a_divider(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    visual.simulate_resize(size(px(1200.), px(800.)));
    let view = chat(
        "claude",
        "stream",
        json!([
            {"kind":"user_message","text":"Fix the parser"},
            {"kind":"reasoning","item_id":"m1:0","text":"Look at the lexer first.\nThen the tests."},
            {"kind":"assistant_text","text":"Fixed."},
            {"kind":"compaction","trigger":"auto","pre_tokens":25000,"post_tokens":2800}
        ]),
        json!({}),
    );
    show(&workspace, &mut visual, view);
    assert!(visual.debug_bounds("thinking-row").is_some());
    assert!(visual.debug_bounds("thinking-body").is_none());
    assert!(visual.debug_bounds("compaction-row").is_some());
    workspace.read_with(&visual, |this, _| {
        let rows = &this.view.transcript.rows;
        assert_eq!(rows[1].role, "Thinking");
        assert_eq!(rows[2].text, "Fixed.", "reasoning never joins the reply");
        assert_eq!(rows[3].text, "Context compacted (auto) · 25k → 2.8k tokens");
    });
    tap(&mut visual, "thinking-toggle");
    assert!(visual.debug_bounds("thinking-body").is_some());
}

#[gpui::test]
fn rewind_files_previews_first_and_only_then_restores(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.simulate_resize(size(px(1200.), px(800.)));
    let view = chat(
        "claude",
        "stream",
        json!([
            {"kind":"user_message","text":"Write the file","uuid":UUID},
            {"kind":"assistant_text","text":"Done."},
            {"kind":"user_message","text":"steered, no uuid"}
        ]),
        json!({}),
    );
    show(&workspace, &mut visual, view);
    effects(&mut commands);
    assert!(
        visual.debug_bounds("undo-from-here").is_none(),
        "Codex's undo is not offered on Claude"
    );
    tap(&mut visual, "rewind-files");
    let preview = Request::RewindFiles {
        session: "a".into(),
        message: UUID.into(),
        preview: true,
    };
    assert!(matches!(requests(&mut commands).as_slice(),
        [Request::RewindFiles { message, preview: true, .. }] if message == UUID));
    assert!(visual.debug_bounds("message-action-panel").is_some());
    assert!(visual.debug_bounds("rewind-files-confirm").is_none());
    request_state(
        &workspace,
        &mut visual,
        preview,
        1,
        json!({"ok":true,"result":{"canRewind":true,"filesChanged":["/w/a.rs"],"insertions":0,"deletions":3}}),
    );
    tap(&mut visual, "rewind-files-confirm");
    assert!(matches!(requests(&mut commands).as_slice(),
        [Request::RewindFiles { message, preview: false, .. }] if message == UUID));
    // A refusal is shown in the CLI's words.
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            let mut view = (*this.view).clone();
            view.requests.insert(
                "rewind-files",
                wks_native::features::RequestState {
                    request: Request::RewindFiles {
                        session: "a".into(),
                        message: UUID.into(),
                        preview: false,
                    },
                    number: 2,
                    loading: false,
                    error: Some("daemon returned 502 Bad Gateway: {\"ok\":false,\"error\":\"No file checkpoint found for this message.\"}".into()),
                    value: Arc::new(serde_json::Value::Null),
                },
            );
            this.update_view(Arc::new(view), window, cx);
        })
    });
    visual.run_until_parked();
    visual.update(|window, _| window.refresh());
    visual.run_until_parked();
    assert!(visual.debug_bounds("message-action-status").is_some());
    tap(&mut visual, "message-action-cancel");
    workspace.read_with(&visual, |this, _| {
        assert!(this.tools.message_action.is_none())
    });
}

#[gpui::test]
fn codex_messages_offer_undo_and_fork_only_with_a_uuid_and_access(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.simulate_resize(size(px(1200.), px(800.)));
    let items = json!([{"kind":"user_message","text":"Refactor it","uuid":"u-1"},
        {"kind":"assistant_text","text":"Done."}]);
    show(
        &workspace,
        &mut visual,
        chat("codex", "stream", items.clone(), json!({})),
    );
    effects(&mut commands);
    assert!(visual.debug_bounds("rewind-files").is_none());
    tap(&mut visual, "undo-from-here");
    tap(&mut visual, "undo-confirm");
    assert!(matches!(acts(&mut commands).as_slice(), [Action::Revert(uuid)] if uuid == "u-1"));
    tap(&mut visual, "fork-from-here");
    tap(&mut visual, "fork-confirm");
    assert!(matches!(requests(&mut commands).as_slice(),
        [Request::Fork { session, at: Some(at), label: None }] if session == "a" && at == "u-1"));
}

// Debug bounds outlive the frame that drew them, so each "not offered"
// check starts from a window that never showed the control.

#[gpui::test]
fn a_tier_without_the_calls_shows_no_history_controls(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    visual.simulate_resize(size(px(1200.), px(800.)));
    let mut limited = chat(
        "codex",
        "stream",
        json!([{"kind":"user_message","text":"Refactor it","uuid":"u-1"}]),
        json!({}),
    );
    limited.methods = Some(Arc::new(vec!["agents.sendMessage".into()]));
    show(&workspace, &mut visual, limited);
    assert!(visual.debug_bounds("message-receipt").is_none());
    assert!(visual.debug_bounds("undo-from-here").is_none());
    assert!(visual.debug_bounds("fork-from-here").is_none());
}

#[gpui::test]
fn a_steered_message_without_a_uuid_offers_nothing(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    visual.simulate_resize(size(px(1200.), px(800.)));
    show(
        &workspace,
        &mut visual,
        chat(
            "codex",
            "stream",
            json!([{"kind":"user_message","text":"steered"}]),
            json!({}),
        ),
    );
    assert!(visual.debug_bounds("undo-from-here").is_none());
    assert!(visual.debug_bounds("fork-from-here").is_none());
}

#[gpui::test]
fn a_delivery_receipt_shows_on_the_matching_bubble(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    visual.simulate_resize(size(px(1200.), px(800.)));
    show(
        &workspace,
        &mut visual,
        chat(
            "claude",
            "stream",
            json!([{"kind":"user_message","text":"Run it","uuid":UUID}]),
            json!({"messageReceipts":[{"uuid":UUID,"state":"started","at":1}]}),
        ),
    );
    assert!(visual.debug_bounds("message-receipt").is_some());
}

#[gpui::test]
fn session_page_compacts_forks_and_reconnects_mcp_servers(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.simulate_resize(size(px(1200.), px(1000.)));
    show(
        &workspace,
        &mut visual,
        chat(
            "claude",
            "stream",
            json!([]),
            json!({"compacting":false,"compaction_count":1,"mcpServers":[
                {"name":"github","status":"connected"},{"name":"db","status":"failed","error":"spawn ENOENT"}]}),
        ),
    );
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.open_feature(Screen::Session, window, cx)
        })
    });
    visual.run_until_parked();
    effects(&mut commands);
    assert!(visual.debug_bounds("context-card").is_some());
    assert!(visual.debug_bounds("codex-card").is_none());
    workspace.update_in(&mut visual, |this, window, cx| {
        this.tools.compact_note.update(cx, |input, cx| {
            input.set_value("keep the API notes", window, cx)
        })
    });
    tap(&mut visual, "compact-context");
    assert!(matches!(acts(&mut commands).as_slice(),
        [Action::Compact(note)] if note == "keep the API notes"));
    tap(&mut visual, "fork-session");
    assert!(matches!(
        requests(&mut commands).as_slice(),
        [Request::Fork { at: None, .. }]
    ));
    assert!(visual.debug_bounds("mcp-reconnect-0").is_none());
    tap(&mut visual, "mcp-reconnect-1");
    assert!(matches!(acts(&mut commands).as_slice(),
        [Action::McpReconnect(server)] if server == "db"));
}

#[gpui::test]
fn codex_sessions_set_goals_start_reviews_and_set_up_the_sandbox(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.simulate_resize(size(px(1200.), px(2000.)));
    show(
        &workspace,
        &mut visual,
        chat(
            "codex",
            "stream",
            json!([]),
            json!({"compacting":false,"goal":{"objective":"Ship the parser","status":"active",
                "tokenBudget":50000,"tokensUsed":12000},
                "windowsSandbox":{"ready":false,"status":"notConfigured"}}),
        ),
    );
    // The chat says the sandbox is not ready, with the fix beside it.
    assert!(visual.debug_bounds("sandbox-setup-notice").is_some());
    assert!(visual.debug_bounds("title-goal").is_some());
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.open_feature(Screen::Session, window, cx)
        })
    });
    visual.run_until_parked();
    effects(&mut commands);
    assert!(visual.debug_bounds("goal-current").is_some());
    tap(&mut visual, "goal-pause");
    assert!(matches!(acts(&mut commands).as_slice(),
        [Action::Goal(GoalChange::Status(s))] if s == "paused"));
    workspace.update_in(&mut visual, |this, window, cx| {
        this.tools
            .goal_objective
            .update(cx, |input, cx| input.set_value("Ship v2", window, cx));
        this.tools
            .goal_budget
            .update(cx, |input, cx| input.set_value("80k", window, cx));
    });
    tap(&mut visual, "goal-save");
    assert!(matches!(acts(&mut commands).as_slice(),
        [Action::Goal(GoalChange::Set { objective, token_budget: Some(80_000) })] if objective == "Ship v2"));
    tap(&mut visual, "goal-clear");
    assert!(matches!(
        acts(&mut commands).as_slice(),
        [Action::Goal(GoalChange::Clear)]
    ));
    tap(&mut visual, "review-start");
    assert!(matches!(
        acts(&mut commands).as_slice(),
        [Action::Review(ReviewTarget::Uncommitted)]
    ));
    tap(&mut visual, "review-kind-1");
    // An empty branch is caught here, not sent.
    tap(&mut visual, "review-start");
    assert!(acts(&mut commands).is_empty());
    workspace.update_in(&mut visual, |this, window, cx| {
        this.tools
            .review_text
            .update(cx, |input, cx| input.set_value("main", window, cx))
    });
    tap(&mut visual, "review-start");
    assert!(matches!(acts(&mut commands).as_slice(),
        [Action::Review(ReviewTarget::BaseBranch(branch))] if branch == "main"));
    tap(&mut visual, "sandbox-setup");
    assert!(matches!(
        acts(&mut commands).as_slice(),
        [Action::SandboxSetup]
    ));
}

#[gpui::test]
fn codex_plan_mode_sits_beside_approvals_and_shows_on_the_title(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.simulate_resize(size(px(1200.), px(900.)));
    let codex = |mode: &str| {
        chat(
            "codex",
            "stream",
            json!([]),
            json!({"compacting":false,"livePermissionMode":"yolo","collaborationMode":mode}),
        )
    };
    show(&workspace, &mut visual, codex("plan"));
    assert!(visual.debug_bounds("title-plan").is_some());
    assert!(
        visual.debug_bounds("title-access").is_some(),
        "full access still shows beside plan"
    );
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| this.open_feature(Screen::Model, window, cx))
    });
    visual.run_until_parked();
    effects(&mut commands);
    tap(&mut visual, "live-plan-mode");
    assert!(matches!(acts(&mut commands).as_slice(),
        [Action::SetPlanMode { plan: false, posture }] if posture == "yolo"));
    show(&workspace, &mut visual, codex("default"));
    tap(&mut visual, "live-plan-mode");
    let sent = acts(&mut commands);
    assert!(matches!(
        sent.as_slice(),
        [Action::SetPlanMode { plan: true, .. }]
    ));
    assert_eq!(sent[0].wire("a").1, json!({"sessionId":"a","mode":"plan"}));
}

#[gpui::test]
fn the_title_reports_compacting_mcp_failures_and_fast_mode_but_not_hooks(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.simulate_resize(size(px(1400.), px(800.)));
    show(
        &workspace,
        &mut visual,
        chat(
            "claude",
            "stream",
            json!([]),
            json!({"compacting":true,"livePermissionMode":"default","observedPermissionMode":"acceptEdits",
                "activeHooks":[{"id":"h","name":"lint","event":"PostToolUse","started_at":1}],
                "mcpServers":[{"name":"db","status":"failed"}],"fastMode":"on"}),
        ),
    );
    effects(&mut commands);
    for chip in [
        "title-compacting",
        "title-mcp",
        "title-fast",
        "title-access",
    ] {
        assert!(visual.debug_bounds(chip).is_some(), "{chip}");
    }
    // A running hook is too brief to earn a chip.
    assert!(visual.debug_bounds("title-hook").is_none());
    tap(&mut visual, "title-fast");
    assert!(matches!(
        acts(&mut commands).as_slice(),
        [Action::SetFastMode(false)]
    ));
}

#[gpui::test]
fn nothing_reported_means_no_status_chips_and_no_plan_toggle(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    visual.simulate_resize(size(px(1400.), px(800.)));
    show(
        &workspace,
        &mut visual,
        chat("codex", "stream", json!([]), json!({"compacting":false})),
    );
    for chip in [
        "title-compacting",
        "title-hook",
        "title-mcp",
        "title-fast",
        "title-plan",
        "title-goal",
        "sandbox-setup-notice",
    ] {
        assert!(visual.debug_bounds(chip).is_none(), "{chip}");
    }
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| this.open_feature(Screen::Model, window, cx))
    });
    visual.run_until_parked();
    assert!(visual.debug_bounds("live-plan-mode").is_none());
}

/// Session "a" waiting on `questions` on card `card`, marked as an MCP
/// elicitation from `server` when one is given.
fn asking(card: &str, server: Option<&str>, questions: serde_json::Value) -> View {
    let mut view = state("a");
    let session = &mut Arc::make_mut(&mut view.sessions)[0];
    session.state = "question".into();
    let mut row = json!({"pendingRequestId":card,"pendingQuestions":questions,
        "pendingQuestionSource":"question"});
    if let Some(server) = server {
        row["pendingQuestionSource"] = json!("elicitation");
        row["elicitationServer"] = json!(server);
    }
    session.merge(&row);
    view
}

fn link_card() -> serde_json::Value {
    json!([{"header":"auth link",
        "question":"Sign in\n\nThe MCP server `auth` asks you to open:\nhttps://example.com/login",
        "options":[{"label":"Open","description":"https://example.com/login"},{"label":"Decline"}]}])
}

#[gpui::test]
fn an_mcp_link_opens_only_once_its_open_answer_is_accepted(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.simulate_resize(size(px(1200.), px(900.)));
    let view = asking("e-1", Some("auth"), link_card());
    show(&workspace, &mut visual, view.clone());
    effects(&mut commands);
    assert!(visual.debug_bounds("elicitation-url").is_some());
    tap(&mut visual, "elicitation-open");
    assert!(matches!(acts(&mut commands).as_slice(),
        [Action::Answers(answers, Some(card))] if answers == &["Open"] && card == "e-1"));
    assert!(
        cx.opened_url().is_none(),
        "nothing opens before the hub takes the answer"
    );
    // A refusal opens nothing.
    let mut refused = view.clone();
    refused.receipt = Some(Receipt {
        number: 1,
        session: "a".into(),
        action: Action::Answers(vec!["Open".into()], Some("e-1".into())),
        error: Some("Request refused".into()),
    });
    show(&workspace, &mut visual, refused);
    assert!(cx.opened_url().is_none());
    // Accepted: it opens, exactly as validated, once.
    tap(&mut visual, "elicitation-open");
    effects(&mut commands);
    let mut accepted = view;
    accepted.receipt = Some(Receipt {
        number: 2,
        session: "a".into(),
        action: Action::Answers(vec!["Open".into()], Some("e-1".into())),
        error: None,
    });
    show(&workspace, &mut visual, accepted);
    assert_eq!(
        cx.opened_url().as_deref(),
        Some("https://example.com/login")
    );
    workspace.read_with(&visual, |this, _| {
        assert!(this.extras.answers_sent);
        assert_eq!(
            this.tools.opened.as_ref(),
            Some(&this.extras.question_signature)
        );
    });
}

#[gpui::test]
fn a_look_alike_agent_question_is_never_an_mcp_elicitation(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.simulate_resize(size(px(1200.), px(900.)));
    // The same words and options an MCP link card uses, from the agent's own
    // AskUserQuestion: no hub mark, so an ordinary question.
    show(&workspace, &mut visual, asking("q-1", None, link_card()));
    effects(&mut commands);
    assert!(visual.debug_bounds("elicitation-url").is_none());
    assert!(visual.debug_bounds("elicitation-open").is_none());
    assert!(visual.debug_bounds("question-0-option-0").is_some());
    tap(&mut visual, "question-0-option-0");
    tap(&mut visual, "submit-answers");
    assert!(matches!(acts(&mut commands).as_slice(),
        [Action::Answers(answers, Some(card))] if answers == &["Open"] && card == "q-1"));
    let accepted = {
        let mut view = asking("q-1", None, link_card());
        view.receipt = Some(Receipt {
            number: 1,
            session: "a".into(),
            action: Action::Answers(vec!["Open".into()], Some("q-1".into())),
            error: None,
        });
        view
    };
    show(&workspace, &mut visual, accepted);
    assert!(
        cx.opened_url().is_none(),
        "an agent's question opens nothing"
    );
    // A look-alike consent form is no elicitation either.
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    show(
        &workspace,
        &mut visual,
        asking(
            "q-2",
            None,
            json!([{"header":"github asks","question":"Send?",
                "options":[{"label":"Submit"},{"label":"Decline"}]}]),
        ),
    );
    assert!(visual.debug_bounds("elicitation-consent").is_none());
}

#[gpui::test]
fn mcp_forms_need_only_required_fields_and_an_explicit_submit(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.simulate_resize(size(px(1200.), px(1000.)));
    show(
        &workspace,
        &mut visual,
        asking(
            "e-2",
            Some("github"),
            json!([
                {"header":"Repository","question":"Which repo? (required)","options":[]},
                {"header":"Note","question":"Anything else?","options":[]},
                {"header":"github asks","question":"Pick a repo\n\nSend these answers to the MCP server `github`?",
                 "options":[{"label":"Submit","description":"Send the answers to `github`"},
                            {"label":"Decline","description":"Send nothing"}]}
            ]),
        ),
    );
    effects(&mut commands);
    assert!(visual.debug_bounds("elicitation-consent").is_some());
    assert!(
        visual.debug_bounds("question-2-answer").is_none(),
        "consent is chosen, never typed"
    );
    tap(&mut visual, "question-2-option-0");
    workspace.read_with(&visual, |this, cx| {
        assert!(!this.answers_ready(cx), "the required field is still blank")
    });
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.extras.answers[0].update(cx, |input, cx| input.set_value("web", window, cx))
        })
    });
    visual.run_until_parked();
    workspace.read_with(&visual, |this, cx| {
        assert!(this.answers_ready(cx), "the optional note may stay blank")
    });
    tap(&mut visual, "submit-answers");
    assert!(matches!(acts(&mut commands).as_slice(),
        [Action::Answers(answers, Some(card))]
            if answers == &["web", "", "Submit"] && card == "e-2"));
}

#[gpui::test]
fn a_card_that_changes_before_the_click_gets_a_fresh_answer(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.simulate_resize(size(px(1200.), px(900.)));
    let questions = json!([{"header":"Pick","question":"Which?",
        "options":[{"label":"A"},{"label":"B"}]}]);
    show(
        &workspace,
        &mut visual,
        asking("c-1", None, questions.clone()),
    );
    tap(&mut visual, "question-0-option-1");
    // The same words on a new card: the pick is gone and Send binds c-2.
    show(&workspace, &mut visual, asking("c-2", None, questions));
    workspace.read_with(&visual, |this, cx| {
        assert_eq!(this.tools.question_card.as_deref(), Some("c-2"));
        assert!(!this.answers_ready(cx));
    });
    effects(&mut commands);
    tap(&mut visual, "question-0-option-0");
    tap(&mut visual, "submit-answers");
    assert!(matches!(acts(&mut commands).as_slice(),
        [Action::Answers(answers, Some(card))] if answers == &["A"] && card == "c-2"));
    // A deny note is bound to the card it was opened on, and closes when
    // the card changes under it.
    let approval = |card: &str| {
        let mut view = state("a");
        Arc::make_mut(&mut view.sessions)[0].merge(&json!({"provider":"claude",
            "pendingRequestId":card,"pendingApproval":{"toolName":"Bash","requestId":card}}));
        view
    };
    show(&workspace, &mut visual, approval("r-1"));
    tap(&mut visual, "deny-with-note");
    workspace.read_with(&visual, |this, _| {
        assert!(this.extras.deny_open);
        assert_eq!(this.tools.deny_card.as_deref(), Some("r-1"));
    });
    show(&workspace, &mut visual, approval("r-2"));
    workspace.read_with(&visual, |this, _| assert!(!this.extras.deny_open));
}

#[gpui::test]
fn a_new_rewind_preview_supersedes_and_a_late_one_offers_retry(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.simulate_resize(size(px(1200.), px(900.)));
    show(
        &workspace,
        &mut visual,
        chat(
            "claude",
            "stream",
            json!([{"kind":"user_message","text":"first","uuid":"u-1"},
                   {"kind":"assistant_text","text":"ok"}]),
            json!({}),
        ),
    );
    effects(&mut commands);
    tap(&mut visual, "rewind-files");
    assert!(matches!(requests(&mut commands).as_slice(),
        [Request::RewindFiles { message, preview: true, .. }] if message == "u-1"));
    // Still loading: the panel can always be left.
    request_state(
        &workspace,
        &mut visual,
        Request::RewindFiles {
            session: "a".into(),
            message: "u-1".into(),
            preview: true,
        },
        1,
        serde_json::Value::Null,
    );
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            let mut view = (*this.view).clone();
            view.requests.get_mut("rewind-files").unwrap().loading = true;
            if let Some(action) = this.tools.message_action.as_mut() {
                action.asked_at = std::time::Instant::now() - session_tools::REWIND_WAIT;
            }
            this.update_view(Arc::new(view), window, cx);
        })
    });
    visual.run_until_parked();
    visual.update(|window, _| window.refresh());
    visual.run_until_parked();
    assert!(visual.debug_bounds("message-action-cancel").is_some());
    // Late: Retry asks again (a fresh preview, which supersedes the old).
    tap(&mut visual, "rewind-retry");
    assert!(matches!(requests(&mut commands).as_slice(),
        [Request::RewindFiles { message, preview: true, .. }] if message == "u-1"));
    tap(&mut visual, "message-action-cancel");
    workspace.read_with(&visual, |this, _| {
        assert!(this.tools.message_action.is_none())
    });
}

#[gpui::test]
fn a_confirmed_delivery_is_shown_with_dismiss(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.simulate_resize(size(px(1200.), px(800.)));
    show(
        &workspace,
        &mut visual,
        chat(
            "claude",
            "stream",
            json!([]),
            json!({"queuedMessages":[],"queueConfirmed":{"id":"q-9","text":"run the tests",
                "messageUuid":"u","delivery":"started","at":1}}),
        ),
    );
    effects(&mut commands);
    assert!(visual.debug_bounds("queue-confirmed").is_some());
    tap(&mut visual, "queue-confirmed-dismiss");
    assert!(matches!(acts(&mut commands).as_slice(),
        [Action::Queue(wks_native::queue::Op::Remove(id))] if id == "q-9"));
}
