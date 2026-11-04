//! Design v2's dock: the "waiting on you" cards (approval with number keys
//! and Details as facts, the merged plan and limit card, the question
//! stepper), the Send / Send now / Queue controls, and the first-run
//! welcome.
use super::*;
use serde_json::json;
use wks_native::appearance::Design;
use wks_native::queue::Op;

fn v2(workspace: &Entity<Workspace>, visual: &mut VisualTestContext) {
    visual.update(|_, cx| {
        workspace.update(cx, |this, _| {
            this.demo = false;
            this.force_design(Some(Design::V2));
        })
    });
}

fn show(workspace: &Entity<Workspace>, visual: &mut VisualTestContext, view: View) {
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| this.update_view(Arc::new(view), window, cx))
    });
    visual.run_until_parked();
    visual.update(|window, _| window.refresh());
    visual.run_until_parked();
}

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

/// Session "a" (Claude) with `row` merged into its snapshot.
fn with(row: serde_json::Value) -> View {
    let mut view = state("a");
    let session = &mut Arc::make_mut(&mut view.sessions)[0];
    session.provider = "claude".into();
    session.cwd = "/home/dev/workspacer".into();
    session.merge(&row);
    view
}

fn bash_approval() -> View {
    with(
        json!({"pendingRequestId":"req-1","pendingApproval":{"toolName":"Bash",
        "toolInput":{"command":"cargo test","description":"Run the tests"},"requestId":"req-1",
        "alwaysOptions":[{"id":"req-1:0","label":"Always allow cargo test","scope":"session","kind":"addRules"}]}}),
    )
}

fn focus_root(workspace: &Entity<Workspace>, visual: &mut VisualTestContext) {
    visual.update(|window, cx| workspace.update(cx, |this, _| this.focus.focus(window)));
    visual.run_until_parked();
}

fn draft(workspace: &Entity<Workspace>, visual: &mut VisualTestContext, text: &str) {
    let text = text.to_owned();
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.composer.update(cx, |input, cx| {
                input.set_value(text, window, cx);
                input.focus(window, cx);
            })
        })
    });
    visual.run_until_parked();
}

fn inside(visual: &mut VisualTestContext, inner: &'static str, outer: &'static str) -> bool {
    let (Some(a), Some(b)) = (visual.debug_bounds(inner), visual.debug_bounds(outer)) else {
        return false;
    };
    a.top() >= b.top() && a.bottom() <= b.bottom() && a.left() >= b.left() && a.right() <= b.right()
}

/// The approval is the "waiting on you" card above the composer (r2): a
/// question headline and where/why line, its buttons keep their selectors
/// and card binding, the number keys pick them, and the composer yields.
#[gpui::test]
fn the_approval_waits_above_the_composer_and_takes_number_keys(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.simulate_resize(size(px(1200.), px(800.)));
    v2(&workspace, &mut visual);
    show(&workspace, &mut visual, bash_approval());
    effects(&mut commands);
    assert!(visual.debug_bounds("approval-card").is_some());
    assert!(!inside(&mut visual, "approval-card", "chat-composer"));
    assert!(inside(&mut visual, "approve", "approval-card"));
    assert!(inside(&mut visual, "headline-code", "approval-card"));
    assert!(inside(&mut visual, "waiting-context", "approval-card"));
    workspace.read_with(&visual, |this, _| {
        assert_eq!(
            this.composer_ui.placeholder,
            Some(super::super::composer_v2::composer_placeholder(Some(
                super::super::composer_v2::Decision::Approval
            )))
        )
    });
    // No v1 hint row or notch; the card says what is waited on.
    assert!(visual.debug_bounds("composer-send-hint").is_none());
    assert!(visual.debug_bounds("composer-notch").is_none());

    tap(&mut visual, "approve");
    assert!(matches!(acts(&mut commands).as_slice(),
        [Action::Approve(true, Some(card))] if card == "req-1"));
    tap(&mut visual, "always-option-0");
    assert!(matches!(acts(&mut commands).as_slice(),
        [Action::AlwaysAllow { option, card, .. }] if option == "req-1:0" && card == "req-1"));

    // 1 Allow, 2 Always allow, 3 Deny, 4 Deny with note.
    focus_root(&workspace, &mut visual);
    visual.simulate_keystrokes("1");
    assert!(matches!(
        acts(&mut commands).as_slice(),
        [Action::Approve(true, _)]
    ));
    visual.simulate_keystrokes("2");
    assert!(matches!(
        acts(&mut commands).as_slice(),
        [Action::AlwaysAllow { .. }]
    ));
    visual.simulate_keystrokes("3");
    assert!(matches!(
        acts(&mut commands).as_slice(),
        [Action::Approve(false, _)]
    ));
    visual.simulate_keystrokes("9");
    assert!(acts(&mut commands).is_empty(), "no fifth action");
    // An empty composer passes the digit to the strip; a draft keeps it.
    draft(&workspace, &mut visual, "");
    visual.simulate_keystrokes("1");
    assert!(matches!(
        acts(&mut commands).as_slice(),
        [Action::Approve(true, _)]
    ));
    draft(&workspace, &mut visual, "step ");
    visual.simulate_keystrokes("1");
    assert!(acts(&mut commands).is_empty());
    workspace.read_with(&visual, |this, cx| {
        // (set_value leaves the cursor at the start.)
        assert_eq!(this.composer.read(cx).value().as_ref(), "1step ")
    });
    // 4 opens the note; sending it denies with the note.
    focus_root(&workspace, &mut visual);
    visual.simulate_keystrokes("4");
    visual.update(|window, _| window.refresh());
    visual.run_until_parked();
    assert!(visual.debug_bounds("deny-note").is_some());
    visual.simulate_input("only the session tests");
    tap(&mut visual, "deny-note-send");
    assert!(matches!(acts(&mut commands).as_slice(),
        [Action::DenyWithNote(note, Some(card))] if note == "only the session tests" && card == "req-1"));
}

/// Details lists where, who and why, not the raw tool input.
#[gpui::test]
fn approval_details_read_as_facts(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.simulate_resize(size(px(1200.), px(800.)));
    v2(&workspace, &mut visual);
    show(&workspace, &mut visual, bash_approval());
    effects(&mut commands);
    assert!(visual.debug_bounds("approval-details").is_none());
    tap(&mut visual, "approval-toggle");
    assert!(visual.debug_bounds("approval-details").is_some());
    // Command, Folder, Host, Agent, Reason.
    for ix in 0..5 {
        let selector: &'static str = Box::leak(format!("approval-fact-{ix}").into_boxed_str());
        assert!(visual.debug_bounds(selector).is_some(), "{selector}");
    }
    assert!(visual.debug_bounds("approval-fact-5").is_none());
}

/// Send when idle; while working, Queue and a split Send now whose menu
/// holds Steer (and Queue again), and Tab no longer queues from inside the
/// text.
#[gpui::test]
fn the_send_button_sends_now_while_working_and_its_menu_steers(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.simulate_resize(size(px(1200.), px(800.)));
    v2(&workspace, &mut visual);
    show(&workspace, &mut visual, state("a"));
    assert!(visual.debug_bounds("composer-send").is_some());
    assert!(visual.debug_bounds("composer-send-menu").is_none());
    let mut working = state("a");
    Arc::make_mut(&mut working.sessions)[0].state = "responding".into();
    show(&workspace, &mut visual, working);
    effects(&mut commands);
    workspace.read_with(&visual, |this, _| assert!(this.composer_busy()));
    assert!(visual.debug_bounds("composer-send-menu").is_some());
    assert!(visual.debug_bounds("composer-stop").is_some());
    draft(&workspace, &mut visual, "after this turn");
    visual.simulate_keystrokes("tab");
    assert!(
        acts(&mut commands).is_empty(),
        "Tab stays in the text under v2"
    );
    tap(&mut visual, "composer-send-menu");
    assert!(visual.debug_bounds("composer-send-menu-list").is_some());
    tap(&mut visual, "composer-queue");
    assert!(matches!(acts(&mut commands).as_slice(),
        [Action::Queue(Op::Add(text))] if text == "after this turn"));
    workspace.read_with(&visual, |this, _| assert!(!this.composer_ui.send_menu));
    draft(&workspace, &mut visual, "look now");
    tap(&mut visual, "composer-send");
    assert!(matches!(acts(&mut commands).as_slice(),
        [Action::SendNow(text)] if text == "look now"));
    draft(&workspace, &mut visual, "steer it");
    tap(&mut visual, "composer-send-menu");
    tap(&mut visual, "composer-steer");
    assert!(matches!(acts(&mut commands).as_slice(),
        [Action::Send(text)] if text == "steer it"));
    draft(&workspace, &mut visual, "later");
    tap(&mut visual, "composer-queue-button");
    assert!(matches!(acts(&mut commands).as_slice(),
        [Action::Queue(Op::Add(text))] if text == "later"));
}

/// A plan waiting while the account is out of its window: ONE card (r2)
/// whose primary approves, with "Approve · run with Codex" as secondary.
#[gpui::test]
fn a_plan_beside_a_spent_limit_offers_approve_and_continue(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.simulate_resize(size(px(1200.), px(900.)));
    v2(&workspace, &mut visual);
    let reset = wks_native::timing::now_ms() / 1000 + 3600;
    let view = with(json!({"pendingRequestId":"plan-1","pendingApproval":{
            "toolName":"ExitPlanMode","toolInput":{"plan":"## Plan\n\n1. Do it"},"requestId":"plan-1"},
        "statusLine":{"fiveHourPct":100.0,"fiveHourResetsAt":reset}}));
    show(&workspace, &mut visual, view);
    effects(&mut commands);
    assert!(visual.debug_bounds("plan-card").is_some());
    assert!(visual.debug_bounds("limit-card").is_none());
    assert!(inside(&mut visual, "plan-approve-handoff", "plan-card"));
    assert!(visual.debug_bounds("waiting-position").is_none());
    // The plan is not a strip on the composer.
    assert!(!inside(&mut visual, "plan-card", "chat-composer"));
    tap(&mut visual, "plan-approve");
    assert!(matches!(acts(&mut commands).as_slice(),
        [Action::Approve(true, Some(card))] if card == "plan-1"));
}

/// No sessions: the welcome with the wordmark, promise, task composer,
/// starters, projects and the agents' setup state. A starter fills the task.
#[gpui::test]
fn first_run_shows_the_welcome_and_starters_fill_the_task(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    visual.simulate_resize(size(px(1200.), px(900.)));
    v2(&workspace, &mut visual);
    let mut view = View {
        connected: true,
        ..Default::default()
    };
    view.installed.insert("claude".into(), true);
    view.installed.insert("codex".into(), false);
    show(&workspace, &mut visual, view.clone());
    for selector in [
        "state-no-sessions",
        "welcome-headline",
        "welcome-composer",
        "welcome-start",
        "welcome-starters",
        "welcome-projects",
        "welcome-setup-checklist",
        "welcome-agent-status-claude",
        "welcome-agent-status-codex",
        "welcome-command-codex",
    ] {
        assert!(visual.debug_bounds(selector).is_some(), "{selector}");
    }
    // r2: one question heading, no wordmark, no stray capsule; Claude is
    // ready, so it offers no command.
    assert!(visual.debug_bounds("welcome-wordmark").is_none());
    assert!(visual.debug_bounds("welcome-command-claude").is_none());
    tap(&mut visual, "welcome-starter-1");
    workspace.read_with(&visual, |this, cx| {
        assert_eq!(
            this.prompt.read(cx).value().as_ref(),
            "Find a failing test and fix it"
        )
    });
    // v1 keeps its own empty state.
    visual.update(|_, cx| workspace.update(cx, |this, _| this.force_design(Some(Design::V1))));
    show(&workspace, &mut visual, view);
    assert!(visual.debug_bounds("state-no-sessions").is_some());
    workspace.read_with(&visual, |this, _| assert!(!this.design().is_v2()));
}

/// r2: a question set is a stepper inside the waiting card: one question
/// at a time with "1/3", number keys answer (a single choice moves on),
/// Back returns, and Send appears only on the last step.
#[gpui::test]
fn questions_step_one_at_a_time_with_number_keys(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.simulate_resize(size(px(1200.), px(900.)));
    v2(&workspace, &mut visual);
    show(&workspace, &mut visual, question_state());
    effects(&mut commands);
    assert!(visual.debug_bounds("question-card").is_some());
    assert!(visual.debug_bounds("question-0-option-0").is_some());
    assert!(visual.debug_bounds("question-1-option-0").is_none());
    assert!(visual.debug_bounds("submit-answers").is_none());
    focus_root(&workspace, &mut visual);
    visual.simulate_keystrokes("2");
    visual.run_until_parked();
    workspace.read_with(&visual, |this, _| {
        assert!(this.extras.selected_options[0].contains(&1));
        assert_eq!(this.composer_ui.question_step, 1);
    });
    visual.update(|window, _| window.refresh());
    visual.run_until_parked();
    // (debug bounds accumulate across frames, so only presence is checked
    // after the step changes.)
    assert!(visual.debug_bounds("question-1-option-0").is_some());
    tap(&mut visual, "question-back");
    workspace.read_with(&visual, |this, _| {
        assert_eq!(this.composer_ui.question_step, 0)
    });
    tap(&mut visual, "question-next");
    tap(&mut visual, "question-1-option-0");
    tap(&mut visual, "question-next");
    assert!(visual.debug_bounds("submit-answers").is_some());
    assert!(acts(&mut commands).is_empty(), "nothing sent before Send");
}
