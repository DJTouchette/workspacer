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
    assert!(visual.debug_bounds("fleet-open-1").is_some());
    assert!(visual.debug_bounds("fleet-line-2-0").is_some());
    assert!(visual.debug_bounds("fleet-ask-2").is_none());
    // Three tiles in one row of the 3-column grid, the first focused.
    let (a, c) = (
        visual.debug_bounds("fleet-tile-0").unwrap(),
        visual.debug_bounds("fleet-tile-2").unwrap(),
    );
    assert_eq!(a.top(), c.top());
    assert!(c.left() > a.right());
    // R4.7: tiles in one row share a height (the approval's is tallest).
    assert_eq!(a.size.height, c.size.height);
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
/// tile showed is approved. Allow's button does the same, 2 grants the
/// shown "Always allow", 3 denies, and 1 on a question opens it.
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
    assert!(effects(&mut commands).iter().any(|c| matches!(c,
        Command::Act { action: Action::AlwaysAllow { option, card, .. }, .. }
            if option == "req-9:0" && card == "req-9")));
    visual.simulate_keystrokes("left 3");
    assert!(effects(&mut commands).iter().any(|c| matches!(
        c,
        Command::Act {
            action: Action::Approve(false, _),
            ..
        }
    )));

    // The question tile (focused again after the decision): 1 opens the
    // session at its question.
    assert_eq!(focused(&workspace, &visual).as_deref(), Some("q"));
    visual.simulate_keystrokes("1");
    workspace.read_with(&visual, |this, _| {
        assert!(this.screen == Screen::Conversation)
    });
    assert!(
        effects(&mut commands)
            .iter()
            .any(|c| matches!(c, Command::Select(id) if id == "q"))
    );
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
