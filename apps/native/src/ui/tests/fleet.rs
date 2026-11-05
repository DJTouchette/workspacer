//! r3-fleet: the Fleet view (R3.7). Tiles in the lexicon, most urgent
//! first; arrows move the focus across the grid; the focused tile's number
//! keys and buttons answer its own ask; v1 never opens it.
use super::*;
use serde_json::json;
use wks_native::appearance::Design;

fn row(id: &str, label: &str, state: &str, extra: serde_json::Value) -> Session {
    let mut session = Session {
        id: id.into(),
        label: label.into(),
        state: state.into(),
        provider: "claude".into(),
        cwd: "/home/dev/workspacer".into(),
        ..Default::default()
    };
    session.merge(&extra);
    session
}

/// An idle session (open), an approval, a working session and a question,
/// in the hub's order.
fn fleet_view() -> View {
    View {
        connected: true,
        selected: Some("idle".into()),
        sessions: Arc::new(vec![
            row("idle", "Tidy the notes", "input", json!({})),
            row(
                "ask",
                "Restore sessions",
                "input",
                json!({"pendingRequestId":"req-9","pendingApproval":{"toolName":"Bash",
                    "toolInput":{"command":"cargo test","description":"Run the tests"},
                    "requestId":"req-9",
                    "alwaysOptions":[{"id":"req-9:0","label":"Always allow Bash(cargo test:*) this session",
                        "scope":"session","kind":"addRules"}]}}),
            ),
            row(
                "busy",
                "Bus backpressure",
                "responding",
                json!({"lastToolName":"Read","lastToolSummary":"src/bus.rs"}),
            ),
            row(
                "q",
                "Release checklist",
                "question",
                json!({"pendingQuestions":[{"header":"Name",
                    "question":"What should I call the branch?","multiSelect":false,"options":[]}]}),
            ),
        ]),
        ..Default::default()
    }
}

/// A 1600×1000 window (three columns beside the sidebar), design v2, the
/// fleet's view, the keys on the shell, then Ctrl+Shift+F.
fn open_fleet(
    cx: &mut TestAppContext,
) -> (
    Entity<Workspace>,
    VisualTestContext,
    tokio::sync::mpsc::Receiver<Command>,
) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.simulate_resize(size(px(1600.), px(1000.)));
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.demo = false;
            this.force_design(Some(Design::V2));
            this.update_view(Arc::new(fleet_view()), window, cx);
            this.focus.focus(window);
        })
    });
    visual.run_until_parked();
    visual.simulate_keystrokes("ctrl-shift-f");
    refresh(&mut visual);
    effects(&mut commands);
    (workspace, visual, commands)
}

fn refresh(visual: &mut VisualTestContext) {
    visual.update(|window, _| window.refresh());
    visual.run_until_parked();
}

fn focused(workspace: &Entity<Workspace>, visual: &VisualTestContext) -> Option<String> {
    workspace.read_with(visual, |this, _| this.fleet.focus.clone())
}

/// Ctrl+Shift+F opens the Fleet: Needs you first (approval, question),
/// then Working; Idle waits behind "All". Each tile speaks the lexicon,
/// carries its last event, and the waiting ones their ask.
#[gpui::test]
fn tiles_render_with_their_statuses_most_urgent_first(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands) = open_fleet(cx);
    workspace.read_with(&visual, |this, _| assert!(this.screen == Screen::Fleet));
    assert!(visual.debug_bounds("fleet-page").is_some());
    assert!(visual.debug_bounds("fleet-status-0-NeedsYou").is_some());
    assert!(visual.debug_bounds("fleet-status-1-NeedsYou").is_some());
    assert!(visual.debug_bounds("fleet-status-2-Working").is_some());
    assert!(
        visual.debug_bounds("fleet-tile-3").is_none(),
        "idle waits behind All"
    );
    // The approval's ask with its keys; the question's Answer; the working
    // session's last tool. Only the waiting tiles carry the attention bar.
    assert!(visual.debug_bounds("fleet-allow-0").is_some());
    assert!(visual.debug_bounds("fleet-always-0").is_some());
    assert!(visual.debug_bounds("fleet-deny-0").is_some());
    // R5.4: the question is answered on its tile (a field and Send answer).
    assert!(visual.debug_bounds("fleet-answer-1").is_some());
    assert!(visual.debug_bounds("fleet-answer-send-1").is_some());
    assert!(visual.debug_bounds("fleet-line-2-0").is_some());
    assert!(visual.debug_bounds("fleet-ask-2").is_none());
    // Three tiles in one row of the 3-column grid, the first focused.
    let (a, c) = (
        visual.debug_bounds("fleet-tile-0").unwrap(),
        visual.debug_bounds("fleet-tile-2").unwrap(),
    );
    assert_eq!(a.top(), c.top());
    assert!(c.left() > a.right());
    // R6.7: the cards of a row share its height (the decision first in
    // each; the room left falls below it).
    assert_eq!(c.size.height, a.size.height);
    // Allow and Deny side by side, on one line.
    let (allow, deny) = (
        visual.debug_bounds("fleet-allow-0").unwrap(),
        visual.debug_bounds("fleet-deny-0").unwrap(),
    );
    assert_eq!(allow.top(), deny.top());
    assert!(deny.left() > allow.right());
    // The ask sits near the top, right under the title and status.
    let ask = visual.debug_bounds("fleet-ask-0").unwrap();
    let title = visual.debug_bounds("fleet-title-0").unwrap();
    assert!(ask.top() > title.bottom());
    assert!(ask.top() - title.top() < px(80.));
    // R4.3: the approval reads as in the conversation: where it runs, and
    // "Always allow" with its exact rule, never shortened.
    assert!(visual.debug_bounds("fleet-decision-0").is_some());
    workspace.read_with(&visual, |this, cx| {
        let tiles = this.fleet_tiles(cx);
        match &tiles[0].ask {
            Some(super::super::fleet::Ask::Approval {
                always,
                folder,
                host,
                ..
            }) => {
                assert_eq!(always[0].2, "Always allow cargo test commands this session");
                assert_eq!(folder, "/home/dev/workspacer");
                assert!(!host.is_empty());
            }
            _ => panic!("the first tile asks for approval"),
        }
    });
    assert!(visual.debug_bounds("fleet-focused-0").is_some());
    assert!(visual.debug_bounds("nav-fleet").is_some());

    click(&mut visual, "fleet-filter-all");
    refresh(&mut visual);
    assert!(visual.debug_bounds("fleet-status-3-Idle").is_some());

    // R4.7: "Needs you" shows only what waits on you, for a burst.
    click(&mut visual, "fleet-filter-needs");
    refresh(&mut visual);
    workspace.read_with(&visual, |this, cx| {
        let ids: Vec<String> = this.fleet_tiles(cx).into_iter().map(|t| t.id).collect();
        assert_eq!(ids, ["ask", "q"]);
    });
}

/// R4.3: Details on a tile's approval expands the full request in place
/// (tool, command, folder, host), not a menu; it closes again.
#[gpui::test]
fn details_expand_a_tiles_full_request(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands) = open_fleet(cx);
    click(&mut visual, "fleet-details-0");
    refresh(&mut visual);
    assert!(visual.debug_bounds("fleet-details").is_some());
    assert!(visual.debug_bounds("fleet-fact-0").is_some());
    workspace.read_with(&visual, |this, _| {
        assert_eq!(this.fleet.details.as_deref(), Some("ask"));
        assert!(
            this.screen == Screen::Fleet,
            "Details does not open the session"
        );
    });
    assert!(
        !effects(&mut commands)
            .iter()
            .any(|c| matches!(c, Command::Act { .. })),
        "Details decides nothing"
    );
    click(&mut visual, "fleet-details-0");
    workspace.read_with(&visual, |this, _| assert_eq!(this.fleet.details, None));
}

/// Arrows and h j k l move the focus across the grid (no wrap); Enter
/// opens the focused session; Esc returns.
#[gpui::test]
fn keyboard_focus_moves_between_tiles(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands) = open_fleet(cx);
    click(&mut visual, "fleet-filter-all");
    refresh(&mut visual);
    // ask question busy / idle
    assert_eq!(focused(&workspace, &visual), None);
    visual.simulate_keystrokes("right");
    assert_eq!(focused(&workspace, &visual).as_deref(), Some("q"));
    refresh(&mut visual);
    assert!(visual.debug_bounds("fleet-focused-1").is_some());
    visual.simulate_keystrokes("down");
    assert_eq!(focused(&workspace, &visual).as_deref(), Some("idle"));
    visual.simulate_keystrokes("up");
    assert_eq!(focused(&workspace, &visual).as_deref(), Some("ask"));
    visual.simulate_keystrokes("l l l");
    assert_eq!(focused(&workspace, &visual).as_deref(), Some("idle"));
    visual.simulate_keystrokes("h");
    assert_eq!(focused(&workspace, &visual).as_deref(), Some("busy"));
    effects(&mut commands);
    visual.simulate_keystrokes("enter");
    workspace.read_with(&visual, |this, _| {
        assert!(this.screen == Screen::Conversation)
    });
    assert!(
        effects(&mut commands)
            .iter()
            .any(|c| matches!(c, Command::Select(id) if id == "busy"))
    );
    // Back on the Fleet, Esc returns to the conversation.
    visual.simulate_keystrokes("ctrl-shift-f");
    refresh(&mut visual);
    workspace.read_with(&visual, |this, _| assert!(this.screen == Screen::Fleet));
    visual.simulate_keystrokes("escape");
    workspace.read_with(&visual, |this, _| {
        assert!(this.screen == Screen::Conversation)
    });
}

/// 1 on the focused approval tile allows that tile's card: its session is
/// selected first (the hub acts on the selected session), then the card the
/// tile showed is approved. Allow's button does the same; in reading order
/// 2 denies (Deny sits beside Allow) and 3 grants the shown "Always allow"
/// (the row below); 1 on a question opens it.
#[gpui::test]
fn allow_on_a_focused_tile_resolves_that_tiles_approval(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands) = open_fleet(cx);
    visual.simulate_keystrokes("1");
    let sent = effects(&mut commands);
    assert!(
        matches!(sent.as_slice(),
            [Command::Select(id), Command::Act { session, action: Action::Approve(true, Some(card)) }]
                if id == "ask" && session == "ask" && card == "req-9"),
        "Select, then Approve on the tile's card"
    );
    // Decided, the focus moves on to the next ask (the question).
    workspace.read_with(&visual, |this, _| assert!(this.screen == Screen::Fleet));
    assert_eq!(focused(&workspace, &visual).as_deref(), Some("q"));

    // Allow's own button acts on its tile, wherever the focus is.
    click(&mut visual, "fleet-allow-0");
    assert!(effects(&mut commands).iter().any(|c| matches!(c,
        Command::Act { session, action: Action::Approve(true, Some(card)) }
            if session == "ask" && card == "req-9")));
    visual.simulate_keystrokes("left 2");
    assert!(effects(&mut commands).iter().any(|c| matches!(
        c,
        Command::Act {
            action: Action::Approve(false, _),
            ..
        }
    )));
    visual.simulate_keystrokes("left 3");
    assert!(effects(&mut commands).iter().any(|c| matches!(c,
        Command::Act { action: Action::AlwaysAllow { option, card, .. }, .. }
            if option == "req-9:0" && card == "req-9")));

    // r5-fleet: the grant is remembered for Session details, with the rule
    // it saves.
    workspace.read_with(&visual, |this, _| {
        let grants = this.session_grants("ask");
        assert_eq!(grants.len(), 1);
        assert_eq!(grants[0].1.rule, "cargo test *");
        assert_eq!(grants[0].1.exact, "Bash(cargo test:*)");
        assert!(!grants[0].1.revocable(), "the hub cannot remove a rule");
    });

    // The question tile (focused again after the decision): 1 puts the
    // keys in its answer field; Enter sends the answer from the Fleet.
    assert_eq!(focused(&workspace, &visual).as_deref(), Some("q"));
    visual.simulate_keystrokes("1");
    refresh(&mut visual);
    visual.simulate_input("release/0.9");
    visual.simulate_keystrokes("enter");
    workspace.read_with(&visual, |this, _| assert!(this.screen == Screen::Fleet));
    assert!(effects(&mut commands).iter().any(|c| matches!(c,
        Command::Act { session, action: Action::Answers(answers, _) }
            if session == "q" && answers == &["release/0.9".to_owned()])));
}

/// R5.4: the dense table, one row per session, remembered per device; Up
/// and Down move its focus row and the number keys answer as on a tile.
#[gpui::test]
fn the_table_view_lists_one_row_per_session(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands) = open_fleet(cx);
    click(&mut visual, "fleet-view-table");
    refresh(&mut visual);
    workspace.read_with(&visual, |this, _| assert!(this.settings.fleet_table));
    assert!(visual.debug_bounds("fleet-table").is_some());
    for row in ["fleet-row-0", "fleet-row-1", "fleet-row-2"] {
        assert!(visual.debug_bounds(row).is_some(), "{row}");
    }
    assert!(
        visual.debug_bounds("fleet-row-3").is_none(),
        "Active hides idle"
    );
    assert!(visual.debug_bounds("fleet-status-0-NeedsYou").is_some());
    assert!(visual.debug_bounds("fleet-row-allow-0").is_some());
    assert!(visual.debug_bounds("fleet-row-deny-0").is_some());
    visual.simulate_keystrokes("down");
    assert_eq!(focused(&workspace, &visual).as_deref(), Some("q"));
    visual.simulate_keystrokes("up");
    effects(&mut commands);
    click(&mut visual, "fleet-row-deny-0");
    assert!(effects(&mut commands).iter().any(|c| matches!(c,
        Command::Act { session, action: Action::Approve(false, _) } if session == "ask")));
    click(&mut visual, "fleet-view-grid");
    refresh(&mut visual);
    assert!(visual.debug_bounds("fleet-grid").is_some());
}

/// R5.4: an option question is answered on its tile: a list whose number
/// keys send the option.
#[gpui::test]
fn an_option_question_is_answered_on_its_tile(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands) = open_fleet(cx);
    let mut view = fleet_view();
    let mut sessions = (*view.sessions).clone();
    sessions[3] = row(
        "q",
        "Release checklist",
        "question",
        json!({"pendingRequestId":"req-q","pendingQuestions":[{"header":"Name",
            "question":"Which branch name?","multiSelect":false,
            "options":[{"label":"release/0.9","description":"The milestone"},{"label":"rc-0.9"}]}]}),
    );
    view.sessions = Arc::new(sessions);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| this.update_view(Arc::new(view), window, cx))
    });
    refresh(&mut visual);
    assert!(visual.debug_bounds("fleet-options-1").is_some());
    assert!(visual.debug_bounds("fleet-option-1-1").is_some());
    visual.simulate_keystrokes("right");
    effects(&mut commands);
    visual.simulate_keystrokes("2");
    assert!(effects(&mut commands).iter().any(|c| matches!(c,
        Command::Act { session, action: Action::Answers(answers, Some(card)) }
            if session == "q" && answers == &["rc-0.9".to_owned()] && card == "req-q")));
}

/// v1 has no Fleet: Ctrl+Shift+F does nothing there.
#[gpui::test]
fn v1_has_no_fleet(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.demo = false;
            this.force_design(Some(Design::V1));
            this.update_view(Arc::new(fleet_view()), window, cx);
            this.focus.focus(window);
        })
    });
    visual.run_until_parked();
    visual.simulate_keystrokes("ctrl-shift-f");
    refresh(&mut visual);
    workspace.read_with(&visual, |this, _| assert!(this.screen != Screen::Fleet));
    assert!(visual.debug_bounds("fleet-page").is_none());
    assert!(visual.debug_bounds("nav-fleet").is_none());
}

/// r4-integration: one tile per tree, as the sidebar and the filter counts
/// have it. A working session nested under the approval lives on its
/// root's tile, never on a second tile of its own.
#[gpui::test]
fn a_nested_session_has_no_tile_of_its_own(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands) = open_fleet(cx);
    let mut view = fleet_view();
    let mut sessions = (*view.sessions).clone();
    sessions.push(row(
        "child",
        "Review session creation",
        "responding",
        json!({"parentSessionId":"ask","lastToolName":"Read","lastToolSummary":"src/launch.rs"}),
    ));
    view.sessions = Arc::new(sessions);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| this.update_view(Arc::new(view), window, cx))
    });
    refresh(&mut visual);
    workspace.read_with(&visual, |this, cx| {
        let nested = this
            .fleet_sessions(cx)
            .into_iter()
            .any(|(ix, depth, _)| depth > 0 && this.view.sessions[ix].id == "child");
        assert!(nested, "the child nests under its parent");
        let ids: Vec<String> = this.fleet_tiles(cx).into_iter().map(|t| t.id).collect();
        assert!(!ids.iter().any(|id| id == "child"), "{ids:?}");
        let counts = this.attention_counts(cx);
        use crate::ui::attention::Bucket;
        let active = counts[Bucket::NeedsYou.index()] + counts[Bucket::Working.index()];
        assert_eq!(ids.len(), active, "the Active count is the tiles shown");
    });
}

/// r5-integration: a job proposal's card takes the keyboard focus as a
/// tile does. The arrows reach it in grid order (after the tiles that wait
/// on you), it shows the focus ring and its keycaps, 1 approves, 2 rejects
/// and Enter opens its Details.
#[gpui::test]
fn job_proposals_take_the_keyboard_focus_like_tiles(cx: &mut TestAppContext) {
    use wks_native::features::{JobAction, Request, RequestState};
    let (workspace, mut visual, mut commands) = open_fleet(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            let mut view = (*this.view).clone();
            view.requests.insert(
                "jobs",
                RequestState {
                    request: Request::Jobs,
                    number: 1,
                    loading: false,
                    error: None,
                    value: Arc::new(json!({"jobs":[
                        {"id":"job","name":"Prune","enabled":true,
                         "trigger":{"kind":"interval","everyMinutes":240},
                         "action":{"kind":"shell","shell":{"command":"git worktree prune"}}},
                        {"id":"change","name":"Prune","enabled":false,"proposedBy":"helper",
                         "replaces":"job","trigger":{"kind":"daily","at":"03:00"},
                         "action":{"kind":"shell","shell":{"command":"git worktree prune"}}}
                    ]})),
                },
            );
            this.update_view(Arc::new(view), window, cx);
            this.focus.focus(window);
        })
    });
    refresh(&mut visual);
    effects(&mut commands);
    assert!(visual.debug_bounds("fleet-job-0").is_some());
    // Needs you: the approval, the question, then the proposal.
    visual.simulate_keystrokes("right right");
    assert_eq!(focused(&workspace, &visual).as_deref(), Some("job:change"));
    refresh(&mut visual);
    assert!(visual.debug_bounds("fleet-job-focused-0").is_some());
    workspace.read_with(&visual, |this, cx| {
        let tiles = this.fleet_tiles(cx);
        assert_eq!(this.fleet_focus(&tiles), usize::MAX, "no tile has it");
    });
    visual.simulate_keystrokes("1");
    assert!(
        effects(&mut commands)
            .iter()
            .any(|c| matches!(c, Command::Request(Request::JobAction(JobAction::Save(_)))))
    );
    visual.simulate_keystrokes("2");
    assert!(effects(&mut commands).iter().any(|c| matches!(c,
        Command::Request(Request::JobAction(JobAction::Remove(id))) if id == "change")));
    visual.simulate_keystrokes("enter");
    workspace.read_with(&visual, |this, _| {
        assert!(this.screen == Screen::Fleet, "Enter stays on the Fleet");
        assert_eq!(this.extras.job_expanded.as_deref(), Some("change"));
    });
    // Back on the tiles.
    visual.simulate_keystrokes("left");
    assert_eq!(focused(&workspace, &visual).as_deref(), Some("q"));
}

/// `fleet_view` with `extra` sessions after it.
fn with_sessions(
    workspace: &Entity<Workspace>,
    visual: &mut VisualTestContext,
    extra: Vec<Session>,
) {
    let mut view = fleet_view();
    let mut sessions = (*view.sessions).clone();
    sessions.extend(extra);
    view.sessions = Arc::new(sessions);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| this.update_view(Arc::new(view), window, cx))
    });
    refresh(visual);
}

fn approval(id: &str, label: &str, command: &str) -> Session {
    row(
        id,
        label,
        "input",
        json!({"pendingRequestId":format!("req-{id}"),"pendingApproval":{"toolName":"Bash",
            "toolInput":{"command":command},"requestId":format!("req-{id}")}}),
    )
}

/// r6-decisions (R6.3): a request that deletes files shows its risk chip,
/// and Allow takes a second press: the first (key or click) only arms it
/// ("Press again to allow"); the second allows it.
#[gpui::test]
fn a_destructive_request_needs_a_second_press(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands) = open_fleet(cx);
    let mut view = fleet_view();
    let mut sessions = (*view.sessions).clone();
    sessions[1] = approval("ask", "Restore sessions", "rm -rf target");
    view.sessions = Arc::new(sessions);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| this.update_view(Arc::new(view), window, cx))
    });
    refresh(&mut visual);
    effects(&mut commands);
    assert!(
        visual.debug_bounds("decision-risk").is_some(),
        "the chip shows"
    );
    visual.simulate_keystrokes("1");
    assert!(
        !effects(&mut commands)
            .iter()
            .any(|c| matches!(c, Command::Act { .. })),
        "the first press only arms it"
    );
    workspace.read_with(&visual, |this, _| {
        assert!(this.armed(&crate::ui::decision::confirm_key("ask", Some("req-ask"))));
        assert_eq!(this.fleet.focus.as_deref(), Some("ask"), "the focus stays");
    });
    visual.simulate_keystrokes("1");
    assert!(effects(&mut commands).iter().any(|c| matches!(c,
        Command::Act { session, action: Action::Approve(true, Some(card)) }
            if session == "ask" && card == "req-ask")));
    // Deny needs no second press.
    workspace.update(&mut visual, |this, _| this.fleet.focus = Some("ask".into()));
    visual.simulate_keystrokes("2");
    assert!(effects(&mut commands).iter().any(|c| matches!(
        c,
        Command::Act {
            action: Action::Approve(false, _),
            ..
        }
    )));
}

/// r6-decisions (R6.7): identical requests (the same command in the same
/// folder) offer "Allow all N"; its key allows each on its own card. A
/// different command, or the same one elsewhere, is not in the batch.
#[gpui::test]
fn identical_requests_are_allowed_in_one_batch(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands) = open_fleet(cx);
    let mut elsewhere = approval("far", "Elsewhere", "cargo test");
    elsewhere.cwd = "/home/dev/other".into();
    with_sessions(
        &workspace,
        &mut visual,
        vec![
            approval("twin", "Twin", "cargo test"),
            approval("other", "Other", "cargo build"),
            elsewhere,
        ],
    );
    effects(&mut commands);
    let ids = workspace.read_with(&visual, |this, cx| {
        let tiles = this.fleet_tiles(cx);
        let first = tiles.iter().position(|t| t.id == "ask").unwrap();
        crate::ui::fleet::batch_of(&tiles, first)
            .into_iter()
            .map(|ix| tiles[ix].id.clone())
            .collect::<Vec<_>>()
    });
    assert_eq!(ids, ["ask", "twin"]);
    assert!(visual.debug_bounds("fleet-batch-0").is_some());
    assert_eq!(
        crate::ui::fleet::batch_label(2, "cargo test"),
        "Allow all 2 · cargo test"
    );
    // Allow 1 and Deny 2, then the batch beside them: 3 (Always allow is 4).
    visual.simulate_keystrokes("3");
    let approved: Vec<String> = effects(&mut commands)
        .into_iter()
        .filter_map(|c| match c {
            Command::Act {
                session,
                action: Action::Approve(true, _),
            } => Some(session),
            _ => None,
        })
        .collect();
    assert_eq!(approved, ["ask", "twin"]);
}

/// r6-decisions (R6.3): in the table, a row's approve / deny is the
/// DecisionCard's answer row (Allow the filled Primary), with its keys;
/// the waiting time has its own column. R6.7: Waiting longest puts the
/// longest wait first.
#[gpui::test]
fn the_table_answers_through_the_decision_card(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands) = open_fleet(cx);
    click(&mut visual, "fleet-view-table");
    refresh(&mut visual);
    assert!(visual.debug_bounds("fleet-row-decision-0").is_some());
    assert!(visual.debug_bounds("fleet-row-allow-0").is_some());
    assert!(visual.debug_bounds("fleet-row-waiting-0").is_some());
    let now = timing::now_ms();
    let mut view = fleet_view();
    let mut sessions = (*view.sessions).clone();
    sessions[1].telemetry.last_activity_ms = Some(now - 60_000);
    sessions[3].telemetry.last_activity_ms = Some(now - 600_000);
    view.sessions = Arc::new(sessions);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| this.update_view(Arc::new(view), window, cx))
    });
    refresh(&mut visual);
    click(&mut visual, "fleet-sort-waiting");
    workspace.read_with(&visual, |this, cx| {
        assert_eq!(this.fleet.sort, crate::ui::fleet::Sort::Waiting);
        let ids: Vec<String> = this.fleet_tiles(cx).into_iter().map(|t| t.id).collect();
        assert_eq!(ids[..2], ["q", "ask"], "the longest wait first");
    });
}

/// r6-decisions (R6.7): the sidebar is its rail while the Fleet is on
/// screen, and comes back on the way out. Cards in a row share its height.
#[gpui::test]
fn the_fleet_takes_the_room_and_rows_share_a_height(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands) = open_fleet(cx);
    workspace.read_with(&visual, |this, _| assert!(this.sidebar_collapsed));
    let (a, b, c) = (
        visual.debug_bounds("fleet-tile-0").unwrap(),
        visual.debug_bounds("fleet-tile-1").unwrap(),
        visual.debug_bounds("fleet-tile-2").unwrap(),
    );
    assert_eq!(a.top(), c.top());
    assert_eq!(a.size.height, b.size.height);
    assert_eq!(a.size.height, c.size.height);
    visual.simulate_keystrokes("escape");
    refresh(&mut visual);
    workspace.read_with(&visual, |this, _| {
        assert!(this.screen == Screen::Conversation);
        assert!(!this.sidebar_collapsed, "the sidebar is back");
    });
}

/// r6-decisions (R6.2): a session whose subagent stopped answering needs
/// you: its tile asks with Nudge / Stop / View output on 1 2 3.
#[gpui::test]
fn a_stalled_subagent_asks_on_its_tile(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands) = open_fleet(cx);
    let now = timing::now_ms();
    let mut stuck = row(
        "stuck",
        "Bus fan-out",
        "responding",
        json!({"lastToolName":"Read","lastToolSummary":"src/bus.rs"}),
    );
    stuck.telemetry.last_activity_ms = Some(now - 1_000);
    stuck.subagents = Arc::new(json!([{"id":"s1","type":"Explore",
        "description":"Trace session restore paths","status":"running",
        "startedAt": now - 20 * 60_000, "lastActivity": now - 7 * 60_000 - 5_000}]));
    with_sessions(&workspace, &mut visual, vec![stuck]);
    let at = workspace.read_with(&visual, |this, cx| {
        let tiles = this.fleet_tiles(cx);
        let at = tiles.iter().position(|t| t.id == "stuck").unwrap();
        assert_eq!(tiles[at].kind, crate::ui::primitives::StatusKind::NeedsYou);
        assert!(matches!(
            tiles[at].ask,
            Some(crate::ui::fleet::Ask::Stalled(_))
        ));
        at
    });
    // Longest wait first: its known wait leads the unknown ones.
    assert_eq!(at, 0);
    assert!(visual.debug_bounds("fleet-stall-0-decision").is_some());
    assert!(visual.debug_bounds("fleet-stall-0-nudge").is_some());
    workspace.update(&mut visual, |this, _| {
        this.fleet.focus = Some("stuck".into())
    });
    effects(&mut commands);
    visual.simulate_keystrokes("1");
    assert!(effects(&mut commands).iter().any(|c| matches!(c,
        Command::Act { session, action: Action::SendNow(text) }
            if session == "stuck" && text.contains("Trace session restore paths"))));
    workspace.update(&mut visual, |this, _| {
        this.fleet.focus = Some("stuck".into())
    });
    visual.simulate_keystrokes("2");
    assert!(effects(&mut commands).iter().any(|c| matches!(c,
        Command::Act { session, action: Action::Stop } if session == "stuck")));
}

/// r6-integration: a table row answers one ask. When a stalled subagent
/// waits behind the session's approval, the row says "+1 more" so the
/// second ask is not hidden; a row with one ask says nothing more.
#[gpui::test]
fn a_table_row_with_two_asks_says_so(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands) = open_fleet(cx);
    let now = timing::now_ms();
    let mut both = approval("both", "Bus fan-out", "cargo build");
    both.subagents = Arc::new(json!([{"id":"s1","type":"Explore",
        "description":"Trace session restore paths","status":"running",
        "startedAt": now - 20 * 60_000, "lastActivity": now - 7 * 60_000 - 5_000}]));
    with_sessions(&workspace, &mut visual, vec![both]);
    click(&mut visual, "fleet-view-table");
    refresh(&mut visual);
    let at = workspace.read_with(&visual, |this, cx| {
        let tiles = this.fleet_tiles(cx);
        let at = tiles.iter().position(|t| t.id == "both").unwrap();
        assert!(matches!(
            tiles[at].ask,
            Some(crate::ui::fleet::Ask::Approval { .. })
        ));
        assert!(tiles[at].stall.is_some());
        at
    });
    assert!(
        visual
            .debug_bounds(&*Box::leak(
                format!("fleet-row-allow-{at}").into_boxed_str()
            ))
            .is_some()
    );
    let more = visual
        .debug_bounds(&*Box::leak(format!("fleet-row-more-{at}").into_boxed_str()))
        .expect("+1 more on the row with two asks");
    let row = visual
        .debug_bounds(&*Box::leak(format!("fleet-row-{at}").into_boxed_str()))
        .unwrap();
    assert!(row.contains(&more.center()));
    let ask = workspace.read_with(&visual, |this, cx| {
        this.fleet_tiles(cx)
            .iter()
            .position(|t| t.id == "ask")
            .unwrap()
    });
    assert!(
        visual
            .debug_bounds(&*Box::leak(
                format!("fleet-row-more-{ask}").into_boxed_str()
            ))
            .is_none()
    );
}

/// r6-decisions (R6.3): Deny with note replaces the answer row: the note,
/// "Deny and tell it" and Cancel, never a second Allow / Deny under it.
#[gpui::test]
fn deny_with_note_replaces_the_answer_row(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    visual.simulate_resize(size(px(1200.), px(800.)));
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.demo = false;
            this.force_design(Some(Design::V2));
            let mut view = fleet_view();
            view.selected = Some("ask".into());
            this.update_view(Arc::new(view), window, cx);
            this.open_deny_note(Some("req-9".into()), window, cx);
        })
    });
    visual.run_until_parked();
    assert!(visual.debug_bounds("deny-note").is_some());
    assert!(visual.debug_bounds("deny-note-send").is_some());
    assert!(visual.debug_bounds("deny-note-cancel").is_some());
    assert!(visual.debug_bounds("approve").is_none(), "no second Allow");
    assert!(visual.debug_bounds("deny").is_none(), "no second Deny");
    assert!(visual.debug_bounds("always-option-0").is_none());
}
