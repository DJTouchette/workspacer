//! History backed by the hub's permanent store (docs/session-roster.md §5):
//! every session, hidden ones included and badged, paged with "Load older
//! sessions", searched by the hub once typing pauses, and deleted for good
//! only after a confirming second press. An older hub's `sessions.recent`
//! keeps the page working without those.
//!
//! GPUI's test `debug_bounds` keeps a selector once it has been drawn, so
//! bounds prove something appeared (and give click targets); that it left is
//! checked through the workspace's own state.
use super::*;
use serde_json::{Value, json};
use std::time::Duration;
use wks_native::features::Request;

fn row(id: &str, name: &str, at: i64, hidden: bool) -> Value {
    json!({"sessionId": id, "name": name, "cwd": "/work/app", "provider": "claude",
           "mode": "stopped", "transport": "stream", "updatedAt": at, "model": "",
           "hidden": hidden, "onSidebar": false, "transcriptCopy": true})
}

/// `sessions.history`'s first page as the controller holds it.
fn history_page() -> Value {
    json!({"history": true, "query": "", "cursor": null, "next": "700:live",
    "sessions": [
        row("h1", "Deploy fix", 900, false),
        row("h2", "Old spike", 800, true),
        row("live", "Live one", 700, false),
    ]})
}

fn history_fixture(
    cx: &mut TestAppContext,
    value: Value,
) -> (
    Entity<Workspace>,
    VisualTestContext,
    tokio::sync::mpsc::Receiver<Command>,
) {
    let (workspace, mut visual, commands, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            let view = View {
                connected: true,
                sessions: Arc::new(vec![Session {
                    id: "live".into(),
                    label: "Live one".into(),
                    cwd: "/work/app".into(),
                    state: "input".into(),
                    provider: "claude".into(),
                    ..Default::default()
                }]),
                selected: Some("live".into()),
                ..Default::default()
            };
            this.update_view(Arc::new(view), window, cx);
            this.open_feature(Screen::Recent, window, cx);
        })
    });
    request_state(&workspace, &mut visual, Request::Recent, 1, value);
    (workspace, visual, commands)
}

fn ids(workspace: &Entity<Workspace>, visual: &VisualTestContext) -> Vec<String> {
    workspace.read_with(visual, |this, cx| {
        this.recent_page(cx)
            .groups
            .iter()
            .flat_map(|g| g.entries.iter().map(|e| e.session.id.clone()))
            .collect()
    })
}

fn requests(commands: &mut tokio::sync::mpsc::Receiver<Command>) -> Vec<Request> {
    effects(commands)
        .into_iter()
        .filter_map(|c| match c {
            Command::Request(r) => Some(r),
            _ => None,
        })
        .collect()
}

#[gpui::test]
fn hidden_sessions_stay_in_history_with_a_badge_and_their_own_filter(cx: &mut TestAppContext) {
    let (workspace, mut visual, _) = history_fixture(cx, history_page());
    // All sessions really is all of them: the hidden spike is listed too.
    assert_eq!(ids(&workspace, &visual), ["h1", "h2", "live"]);
    bounds_of(&mut visual, "recent-hidden-1");
    // A history row reads stopped; the live fleet says this one is running.
    let live = workspace.read_with(&visual, |this, cx| {
        this.recent_page(cx).groups[0].entries[2].session.stopped()
    });
    assert!(!live, "the fleet's state wins over the store's");
    visual.update(|_, cx| {
        workspace.update(cx, |this, cx| {
            this.extras.show_archived = true;
            cx.notify();
        })
    });
    visual.run_until_parked();
    assert_eq!(ids(&workspace, &visual), ["h2"]);
    // Once the shared archive is known it decides, including an unhide.
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            let mut view = (*this.view).clone();
            view.session_archive = Some(Arc::new(json!({"version":2,"archived":{"h1":5}})));
            this.update_view(Arc::new(view), window, cx);
        })
    });
    visual.run_until_parked();
    assert_eq!(ids(&workspace, &visual), ["h1"]);
}

#[gpui::test]
fn older_sessions_load_from_the_page_cursor(cx: &mut TestAppContext) {
    let (_workspace, mut visual, mut commands) = history_fixture(cx, history_page());
    let _ = effects(&mut commands);
    click(&mut visual, "history-load-more");
    let sent = requests(&mut commands);
    assert!(
        matches!(sent.as_slice(), [Request::HistoryPage { query, cursor: Some(c) }]
            if query.is_empty() && c == "700:live"),
        "{sent:?}"
    );
}

#[gpui::test]
fn delete_asks_first_and_never_offers_a_live_session(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands) = history_fixture(cx, history_page());
    let _ = effects(&mut commands);
    click(&mut visual, "recent-delete-0");
    assert!(
        requests(&mut commands).is_empty(),
        "the first press only asks"
    );
    workspace.read_with(&visual, |this, _| {
        assert_eq!(this.extras.recent.confirm_delete.as_deref(), Some("h1"))
    });
    click(&mut visual, "recent-delete-confirm-0");
    let sent = requests(&mut commands);
    assert!(
        matches!(sent.as_slice(), [Request::ForgetSession { session }] if session == "h1"),
        "{sent:?}"
    );
    workspace.read_with(&visual, |this, _| {
        assert_eq!(this.extras.recent.confirm_delete, None)
    });
    // The running session's Delete is drawn disabled: pressing it does nothing.
    click(&mut visual, "recent-delete-2");
    workspace.read_with(&visual, |this, _| {
        assert_eq!(this.extras.recent.confirm_delete, None)
    });
    assert!(requests(&mut commands).is_empty());
}

#[gpui::test]
fn typing_asks_the_hub_to_search_and_lists_what_it_found(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands) = history_fixture(cx, history_page());
    let _ = effects(&mut commands);
    visual.simulate_input("zeta");
    visual.run_until_parked();
    assert!(
        !requests(&mut commands)
            .iter()
            .any(|r| matches!(r, Request::HistoryPage { .. })),
        "nothing until typing pauses"
    );
    visual.executor().advance_clock(Duration::from_millis(400));
    visual.run_until_parked();
    let sent = requests(&mut commands);
    assert!(
        matches!(sent.as_slice(), [Request::HistoryPage { query, cursor: None }] if query == "zeta"),
        "{sent:?}"
    );
    assert!(ids(&workspace, &visual).is_empty(), "no local match");
    // The hub matched a row by a field this page cannot see (its first
    // prompt), one the loaded page does not even reach: it is listed.
    request_state(
        &workspace,
        &mut visual,
        Request::HistoryPage {
            query: "zeta".into(),
            cursor: None,
        },
        2,
        json!({"history": true, "query": "zeta", "sessions": [row("z9", "Unrelated", 50, false)]}),
    );
    assert_eq!(ids(&workspace, &visual), ["z9"]);
}

#[gpui::test]
fn an_older_hub_keeps_local_search_without_paging_or_delete(cx: &mut TestAppContext) {
    let legacy = json!([row("h1", "Deploy fix", 900, false)]);
    let (workspace, mut visual, mut commands) = history_fixture(cx, legacy);
    workspace.read_with(&visual, |this, _| {
        assert!(!this.history_available());
        assert_eq!(this.history_next(), None);
    });
    assert!(visual.debug_bounds("recent-delete-0").is_none());
    assert!(visual.debug_bounds("history-load-more").is_none());
    let _ = effects(&mut commands);
    visual.simulate_input("deploy");
    visual.executor().advance_clock(Duration::from_millis(400));
    visual.run_until_parked();
    assert!(
        !requests(&mut commands)
            .iter()
            .any(|r| matches!(r, Request::HistoryPage { .. }))
    );
    assert_eq!(ids(&workspace, &visual), ["h1"], "local search still works");
}
