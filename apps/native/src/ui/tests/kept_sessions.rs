//! Paused and Starting come from the hub. A stopped session the hub's shared
//! open set (`sessionOpen`) lists, without an `ended` mark, is paused and
//! picks back up from its composer. A row the hub marks `starting` (or still
//! in `unknown`) reads Starting. This device keeps no list of its own.
use super::*;

fn fleet(sessions: Vec<Session>, selected: &str) -> View {
    View {
        connected: true,
        selected: Some(selected.into()),
        sessions: Arc::new(sessions),
        session_open: Some(open_set(&["a"], &[])),
        ..Default::default()
    }
}

fn open_set(open: &[&str], ended: &[&str]) -> Arc<serde_json::Value> {
    let ids = |ids: &[&str]| {
        ids.iter()
            .map(|id| (id.to_string(), serde_json::json!(1)))
            .collect::<serde_json::Map<_, _>>()
    };
    Arc::new(serde_json::json!({"version":1,"open":ids(open),"ended":ids(ended)}))
}

fn session(id: &str, state: &str) -> Session {
    Session {
        id: id.into(),
        label: id.to_uppercase(),
        state: state.into(),
        provider: "claude".into(),
        cwd: "/work/project".into(),
        model: "sonnet".into(),
        permission_mode: "acceptEdits".into(),
        ..Default::default()
    }
}

fn show(workspace: &Entity<Workspace>, visual: &mut VisualTestContext, view: View) {
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| this.update_view(Arc::new(view), window, cx))
    });
    visual.run_until_parked();
}

#[gpui::test]
fn paused_and_ended_follow_the_hub_open_set(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    let sessions = vec![
        session("a", "stopped"),
        session("b", "stopped"),
        session("c", "stopped"),
        session("d", "input"),
    ];
    // `a` stopped while open (the app closed): Paused. `b` is open but the
    // hub saw it end on purpose: Ended, still listed. `c` is not open.
    let mut view = fleet(sessions.clone(), "a");
    view.session_open = Some(open_set(&["a", "b", "d"], &["b"]));
    show(&workspace, &mut visual, view);
    workspace.read_with(&visual, |this, _| {
        let p = this.palette();
        let [a, b, c, d] = &this.view.sessions[..] else {
            panic!("four sessions")
        };
        assert!(this.paused(a));
        assert_eq!(this.status_of(a, p).0, "Paused");
        assert!(!this.paused(b));
        assert_eq!(this.status_of(b, p).0, "Ended");
        assert!(!this.paused(c));
        assert_eq!(this.status_of(c, p).0, "Ended");
        assert!(!this.paused(d), "a live session is never paused");
        assert_eq!(this.status_of(d, p).0, "Idle");
    });

    // A hub without the shared open set (or before its first read) pauses
    // nothing; there is no local fallback list.
    let mut view = fleet(sessions, "a");
    view.session_open = None;
    show(&workspace, &mut visual, view);
    workspace.read_with(&visual, |this, _| {
        assert!(this.view.sessions.iter().all(|s| !this.paused(s)));
    });
}

#[gpui::test]
fn starting_rows_read_starting(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    let unknown = session("a", "unknown");
    let mut marked = session("b", "");
    marked.merge(&serde_json::json!({"sessionId":"b","mode":"unknown","starting":true}));
    let mut ready = session("c", "");
    ready.merge(&serde_json::json!({"sessionId":"c","mode":"unknown","starting":true}));
    ready.merge(&serde_json::json!({"sessionId":"c","mode":"input"}));
    show(
        &workspace,
        &mut visual,
        fleet(vec![unknown, marked, ready], "a"),
    );
    workspace.read_with(&visual, |this, _| {
        let p = this.palette();
        let [a, b, c] = &this.view.sessions[..] else {
            panic!("three sessions")
        };
        assert!(a.starting() && !a.stopped());
        assert_eq!(this.status_of(a, p).0, "Starting");
        assert!(b.starting());
        assert_eq!(this.status_of(b, p).0, "Starting");
        assert!(!c.starting(), "a state without the mark leaves starting");
        assert_eq!(this.status_of(c, p).0, "Idle");
    });
}

#[gpui::test]
fn sending_from_a_paused_session_resumes_it_with_the_message(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(
                Arc::new(fleet(vec![session("a", "stopped")], "a")),
                window,
                cx,
            );
            this.composer
                .update(cx, |input, cx| input.set_value("carry on", window, cx));
        })
    });
    visual.run_until_parked();
    visual
        .update(|window, cx| workspace.update(cx, |this, cx| this.send(&SendMessage, window, cx)));
    visual.run_until_parked();
    let request = std::iter::from_fn(|| commands.try_recv().ok())
        .find_map(|c| match c {
            Command::Create(request) => Some(request),
            Command::Act { .. } => panic!("a paused session must not be sent to as ended"),
            _ => None,
        })
        .expect("send resumes the session");
    assert_eq!(request.resume_session_id.as_deref(), Some("a"));
    assert_eq!(request.message, "carry on");
    assert_eq!(request.provider, "claude");
    assert_eq!(request.cwd, "/work/project");
    assert_eq!(request.model, "sonnet");
    assert_eq!(
        request.permission,
        wks_native::launch::Permission::AcceptEdits
    );
    // The draft stays until the resume is confirmed, then clears.
    workspace.read_with(&visual, |this, cx| {
        assert_eq!(this.resuming.as_deref(), Some("a"));
        assert_eq!(this.composer.read(cx).value(), "carry on");
    });
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            let mut view = fleet(vec![session("a", "responding")], "a");
            view.spawn_receipt = Some(wks_native::controller::SpawnReceipt {
                number: 1,
                session: Some("a".into()),
                error: None,
                unsent_message: None,
            });
            this.update_view(Arc::new(view), window, cx);
        })
    });
    visual.run_until_parked();
    workspace.read_with(&visual, |this, cx| {
        assert!(this.resuming.is_none());
        assert_eq!(this.composer.read(cx).value(), "");
        assert!(!this.paused(&this.view.sessions[0]));
    });
}

#[gpui::test]
fn a_failed_resume_keeps_the_draft_and_says_why(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(
                Arc::new(fleet(vec![session("a", "stopped")], "a")),
                window,
                cx,
            );
            this.composer
                .update(cx, |input, cx| input.set_value("carry on", window, cx));
            this.send(&SendMessage, window, cx);
            let mut view = fleet(vec![session("a", "stopped")], "a");
            view.spawn_receipt = Some(wks_native::controller::SpawnReceipt {
                number: 1,
                session: None,
                error: Some("provider unavailable".into()),
                unsent_message: None,
            });
            this.update_view(Arc::new(view), window, cx);
        })
    });
    visual.run_until_parked();
    workspace.read_with(&visual, |this, cx| {
        assert!(this.extras.notice.contains("provider unavailable"));
        assert_eq!(this.composer.read(cx).value(), "carry on");
        assert!(
            this.paused(&this.view.sessions[0]),
            "still paused, still resumable"
        );
    });
}
