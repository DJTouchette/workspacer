//! Finished turns nobody opened: the hub's `sessionSeen` marks put a session
//! in Needs you as Finished, and looking at it on a focused window clears the
//! mark once.
use super::*;
use wks_native::{features::Request, navigation::SessionSection};

fn unseen(version: i64, ids: &[&str]) -> Option<Arc<serde_json::Value>> {
    let unseen: serde_json::Map<_, _> = ids
        .iter()
        .map(|id| (id.to_string(), serde_json::json!(1000)))
        .collect();
    Some(Arc::new(
        serde_json::json!({"version":version,"unseen":unseen}),
    ))
}

fn marks(commands: &mut tokio::sync::mpsc::Receiver<Command>) -> Vec<String> {
    archive_effects(commands)
        .into_iter()
        .filter_map(|c| match c {
            Command::Request(Request::MarkSeen { session }) => Some(session),
            _ => None,
        })
        .collect()
}

#[gpui::test]
fn a_finished_turn_needs_you_until_it_is_looked_at_on_a_focused_window(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    let show = |view: View, visual: &mut VisualTestContext| {
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| this.update_view(Arc::new(view), window, cx))
        });
        visual.run_until_parked();
    };
    let mut view = state("a");
    view.session_unseen = unseen(1, &["a", "b"]);
    show(view.clone(), &mut visual);
    workspace.read_with(&visual, |this, cx| {
        let sections = this.session_sections(cx);
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].0, SessionSection::NeedsYou);
        let p = this.palette();
        assert_eq!(this.status_of(&this.view.sessions[1], p).0, "Finished");
    });
    // A background window is not looking at anything.
    assert!(marks(&mut commands).is_empty());
    visual.update(|window, _| window.activate_window());
    show(view.clone(), &mut visual);
    assert_eq!(marks(&mut commands), ["a"], "only the open session, once");
    show(view.clone(), &mut visual);
    assert!(
        marks(&mut commands).is_empty(),
        "not again for the same turn"
    );

    // The hub clears it: a, seen, is Idle and Inactive; b still needs you.
    view.session_unseen = unseen(2, &["b"]);
    show(view.clone(), &mut visual);
    workspace.read_with(&visual, |this, cx| {
        let p = this.palette();
        assert_eq!(this.status_of(&this.view.sessions[0], p).0, "Idle");
        let sections = this.session_sections(cx);
        assert_eq!(sections[0], (SessionSection::NeedsYou, vec![(1, 0)]));
        assert_eq!(sections[1], (SessionSection::Inactive, vec![(0, 0)]));
    });
    // A session that is working again is Working, whatever its old mark.
    let mut working = view.clone();
    Arc::make_mut(&mut working.sessions)[1].state = "responding".into();
    show(working, &mut visual);
    workspace.read_with(&visual, |this, cx| {
        assert_eq!(this.session_sections(cx)[0].0, SessionSection::Working);
    });
    assert!(marks(&mut commands).is_empty());
}

/// A session whose own turn ended while agents still do its work reads as
/// working, not Finished or Idle; a background shell is named without
/// hiding a finished turn.
#[gpui::test]
fn a_session_waiting_on_delegated_work_says_so(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    let mut view = state("a");
    view.session_unseen = unseen(1, &["a", "b"]);
    {
        let sessions = Arc::make_mut(&mut view.sessions);
        // b spawned a Workspacer session that is mid-turn.
        sessions.push(Session {
            id: "worker".into(),
            parent_session_id: "b".into(),
            state: "responding".into(),
            ..Default::default()
        });
        // a only runs a background shell.
        sessions[0].background_tasks = 1;
    }
    let show = |view: View, visual: &mut VisualTestContext| {
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| this.update_view(Arc::new(view), window, cx))
        });
        visual.run_until_parked();
    };
    show(view.clone(), &mut visual);
    workspace.read_with(&visual, |this, cx| {
        let p = this.palette();
        let s = &this.view.sessions;
        // Neither unopened turn is in Needs you while its work goes on.
        let sections = this.session_sections(cx);
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].0, SessionSection::Working);
        assert_eq!(this.status_of(&s[1], p), ("Agents working", p.busy));
        assert!(this.shows_working(&s[1]));
        assert_eq!(this.status_of(&s[0], p).0, "Finished");
        assert!(!this.shows_working(&s[0]));
    });
    // Seen, the shell's session names its task; the worker done, b is Finished.
    view.session_unseen = unseen(2, &["b"]);
    Arc::make_mut(&mut view.sessions)[2].state = "input".into();
    show(view.clone(), &mut visual);
    workspace.read_with(&visual, |this, cx| {
        let p = this.palette();
        let s = &this.view.sessions;
        assert_eq!(this.status_of(&s[0], p), ("Background task", p.busy));
        assert_eq!(this.status_of(&s[1], p).0, "Finished");
        assert!(!this.shows_working(&s[1]));
        assert_eq!(this.session_sections(cx)[0].0, SessionSection::NeedsYou);
    });
    // Offline, the last fleet seen proves nothing.
    view.connected = false;
    Arc::make_mut(&mut view.sessions)[2].state = "responding".into();
    show(view, &mut visual);
    workspace.read_with(&visual, |this, _| {
        assert!(!this.shows_working(&this.view.sessions[1]));
    });
}
