//! The hub's cleanup prompt (docs/session-roster.md §6): shown from the open
//! document's `cleanup` block, its actions are hub calls, and a cleanup offers
//! an Undo that restores exactly the ids the hub reported hiding.
//!
//! GPUI's test `debug_bounds` keeps a selector once it has been drawn, so
//! bounds prove a banner appeared (and give click targets); that it left is
//! checked through the workspace's own state.
use super::*;
use serde_json::Value;
use wks_native::features::{Request, RequestState};

fn open_doc(cleanup: Option<Value>) -> Arc<Value> {
    let mut doc = serde_json::json!({"version":3,"open":{"a":1},"ended":{"a":2}});
    if let Some(cleanup) = cleanup {
        doc["cleanup"] = cleanup;
    }
    Arc::new(doc)
}

fn base(cleanup: Option<Value>) -> View {
    View {
        connected: true,
        sessions: Arc::new(vec![Session {
            id: "a".into(),
            label: "A".into(),
            state: "stopped".into(),
            provider: "claude".into(),
            ..Default::default()
        }]),
        selected: Some("a".into()),
        session_open: Some(open_doc(cleanup)),
        ..Default::default()
    }
}

fn suggested(ended: u64) -> Option<Value> {
    Some(serde_json::json!({"ended":ended,"threshold":20,"suggested":true,"snoozedAt":null}))
}

fn show(workspace: &Entity<Workspace>, visual: &mut VisualTestContext, view: View) {
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.demo = false;
            this.update_view(Arc::new(view), window, cx)
        })
    });
    visual.run_until_parked();
}

fn prompt(workspace: &Entity<Workspace>, visual: &VisualTestContext) -> Option<u64> {
    workspace.read_with(visual, |this, _| this.cleanup_prompt())
}

fn undo_ids(workspace: &Entity<Workspace>, visual: &VisualTestContext) -> Option<Vec<String>> {
    workspace.read_with(visual, |this, _| {
        this.cleanup_undo().map(|undo| undo.ids.clone())
    })
}

fn click(visual: &mut VisualTestContext, selector: &str) {
    let at = bounds_of(visual, selector).center();
    visual.simulate_click(at, gpui::Modifiers::default());
    visual.run_until_parked();
}

fn requests(commands: &mut tokio::sync::mpsc::Receiver<Command>) -> Vec<Request> {
    std::iter::from_fn(|| commands.try_recv().ok())
        .filter_map(|c| match c {
            Command::Request(request) => Some(request),
            _ => None,
        })
        .collect()
}

fn reply(view: &mut View, key: &'static str, number: u64, request: Request, value: Value) {
    view.requests.insert(
        key,
        RequestState {
            number,
            request,
            loading: false,
            value: Arc::new(value),
            error: None,
        },
    );
}

#[gpui::test]
fn the_banner_follows_the_hub_document(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    // No `cleanup` block (an older hub), one that does not suggest it, or
    // one without a count.
    for cleanup in [
        None,
        Some(serde_json::json!({"ended":25,"threshold":20,"suggested":false,"snoozedAt":25})),
        Some(serde_json::json!({"suggested":true})),
    ] {
        show(&workspace, &mut visual, base(cleanup));
        assert_eq!(prompt(&workspace, &visual), None);
        assert!(visual.debug_bounds("cleanup-banner").is_none());
    }
    // A hub without the open set at all.
    let mut view = base(None);
    view.session_open = None;
    show(&workspace, &mut visual, view);
    assert_eq!(prompt(&workspace, &visual), None);
    assert!(visual.debug_bounds("cleanup-banner").is_none());

    show(&workspace, &mut visual, base(suggested(23)));
    assert_eq!(prompt(&workspace, &visual), Some(23));
    assert!(visual.debug_bounds("cleanup-banner").is_some());

    // Offline, nothing to act on; back online, it returns.
    let mut offline = base(suggested(23));
    offline.connected = false;
    show(&workspace, &mut visual, offline);
    assert_eq!(prompt(&workspace, &visual), None);
    show(&workspace, &mut visual, base(suggested(24)));
    assert_eq!(prompt(&workspace, &visual), Some(24));
    // Snoozed or tidied on another client: the hub stops suggesting it.
    show(
        &workspace,
        &mut visual,
        base(Some(
            serde_json::json!({"ended":24,"threshold":20,"suggested":false,"snoozedAt":24}),
        )),
    );
    assert_eq!(prompt(&workspace, &visual), None);
}

#[gpui::test]
fn banner_actions_send_the_hub_calls(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    show(&workspace, &mut visual, base(suggested(23)));
    requests(&mut commands);

    click(&mut visual, "cleanup-all");
    assert!(matches!(
        &requests(&mut commands)[..],
        [Request::CleanupSessions {
            older_than_ms: None
        }]
    ));
    click(&mut visual, "cleanup-older");
    assert!(matches!(
        &requests(&mut commands)[..],
        [Request::CleanupSessions {
            older_than_ms: Some(86_400_000)
        }]
    ));
    click(&mut visual, "cleanup-snooze");
    assert!(matches!(
        &requests(&mut commands)[..],
        [Request::SnoozeCleanup]
    ));

    // While an action is on its way the prompt steps aside.
    let mut view = base(suggested(23));
    view.requests.insert(
        "session-snooze",
        RequestState {
            number: 9,
            request: Request::SnoozeCleanup,
            loading: true,
            value: Arc::new(Value::Null),
            error: None,
        },
    );
    show(&workspace, &mut visual, view);
    assert_eq!(prompt(&workspace, &visual), None);
}

#[gpui::test]
fn undo_restores_exactly_the_ids_the_hub_hid(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    show(&workspace, &mut visual, base(suggested(23)));
    click(&mut visual, "cleanup-all");
    requests(&mut commands);

    // The hub answers with what it hid and stops suggesting a cleanup; the
    // Undo takes the prompt's place.
    let mut view = base(Some(
        serde_json::json!({"ended":0,"threshold":20,"suggested":false,"snoozedAt":null}),
    ));
    reply(
        &mut view,
        "session-cleanup",
        4,
        Request::CleanupSessions {
            older_than_ms: None,
        },
        serde_json::json!({"hidden":["x","y","z"]}),
    );
    show(&workspace, &mut visual, view.clone());
    assert_eq!(prompt(&workspace, &visual), None);
    assert_eq!(
        undo_ids(&workspace, &visual).as_deref(),
        Some(&["x".to_string(), "y".into(), "z".into()][..])
    );
    assert!(visual.debug_bounds("cleanup-undo").is_some());

    click(&mut visual, "cleanup-undo-button");
    match &requests(&mut commands)[..] {
        [Request::UndoCleanup { sessions }] => assert_eq!(sessions, &["x", "y", "z"]),
        other => panic!("expected one undo, got {other:?}"),
    }
    assert_eq!(undo_ids(&workspace, &visual), None);
    // The same reply seen again (any later view) does not offer it again.
    show(&workspace, &mut visual, view);
    assert_eq!(undo_ids(&workspace, &visual), None);
}

#[gpui::test]
fn the_undo_lapses_and_an_empty_cleanup_offers_none(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    let mut view = base(None);
    reply(
        &mut view,
        "session-cleanup",
        1,
        Request::CleanupSessions {
            older_than_ms: Some(86_400_000),
        },
        serde_json::json!({"hidden":[]}),
    );
    show(&workspace, &mut visual, view);
    assert_eq!(undo_ids(&workspace, &visual), None);
    assert!(visual.debug_bounds("cleanup-undo").is_none());

    let mut view = base(None);
    reply(
        &mut view,
        "session-cleanup",
        2,
        Request::CleanupSessions {
            older_than_ms: None,
        },
        serde_json::json!({"hidden":["x"]}),
    );
    show(&workspace, &mut visual, view);
    assert_eq!(undo_ids(&workspace, &visual), Some(vec!["x".to_string()]));
    // Past its window the offer is gone, even before the redraw timer.
    workspace.update(&mut visual, |this, _| {
        this.cleanup.undo.as_mut().unwrap().until = timing::now_ms() - 1;
    });
    assert_eq!(undo_ids(&workspace, &visual), None);
}

#[gpui::test]
fn a_failed_cleanup_says_so_and_keeps_the_prompt(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    // Connected first: the failure arrives on a running connection.
    show(&workspace, &mut visual, base(suggested(23)));
    let mut view = base(suggested(23));
    view.requests.insert(
        "session-cleanup",
        RequestState {
            number: 1,
            request: Request::CleanupSessions {
                older_than_ms: None,
            },
            loading: false,
            value: Arc::new(Value::Null),
            error: Some("hub unavailable".into()),
        },
    );
    show(&workspace, &mut visual, view);
    workspace.read_with(&visual, |this, _| {
        assert!(
            this.extras.notice.contains("hub unavailable"),
            "{}",
            this.extras.notice
        );
        assert!(this.cleanup_undo().is_none());
        assert_eq!(this.cleanup_prompt(), Some(23));
    });
}
