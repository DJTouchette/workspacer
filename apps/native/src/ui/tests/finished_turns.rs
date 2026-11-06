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
