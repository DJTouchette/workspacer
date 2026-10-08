//! Sessions open when the app closed stay listed, paused, and pick back up
//! from their composer.
use super::*;

fn fleet(sessions: Vec<Session>, selected: &str) -> View {
    View {
        connected: true,
        selected: Some(selected.into()),
        sessions: Arc::new(sessions),
        ..Default::default()
    }
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

fn kept(workspace: &Entity<Workspace>, visual: &mut VisualTestContext) -> Vec<String> {
    workspace.read_with(visual, |this, _| {
        this.settings
            .kept_open
            .get(&this.project_scope)
            .map(|kept| kept.keys().cloned().collect())
            .unwrap_or_default()
    })
}

#[gpui::test]
fn open_sessions_are_kept_and_only_ones_ended_while_running_are_let_go(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    let show = |view: View, visual: &mut VisualTestContext| {
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| this.update_view(Arc::new(view), window, cx))
        });
        visual.run_until_parked();
    };
    show(
        fleet(vec![session("a", "input"), session("b", "responding")], "a"),
        &mut visual,
    );
    assert_eq!(kept(&workspace, &mut visual), ["a", "b"]);
    let keeps = |commands: &mut tokio::sync::mpsc::Receiver<Command>| {
        std::iter::from_fn(|| commands.try_recv().ok())
            .filter_map(|c| match c {
                Command::KeepSessions(ids) => Some(ids),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    // Everything kept is listed, so the controller has nothing extra to read.
    assert!(keeps(&mut commands).is_empty());
    // `b` ends while the app is running: ended on purpose, so it is let go.
    show(
        fleet(vec![session("a", "input"), session("b", "stopped")], "a"),
        &mut visual,
    );
    assert_eq!(kept(&workspace, &mut visual), ["a"]);

    // A fresh run (the app reopened) first sees `a` already stopped: the
    // close stopped it, so it stays, paused.
    workspace.update(&mut visual, |this, _| this.seen_open.clear());
    show(fleet(vec![session("a", "stopped")], "a"), &mut visual);
    assert_eq!(kept(&workspace, &mut visual), ["a"]);
    workspace.read_with(&visual, |this, _| {
        let a = &this.view.sessions[0];
        assert!(this.paused(a));
        assert_eq!(this.status_of(a, this.appearance.palette()).0, "Paused");
    });
    // A kept session the fleet list left out is handed over to be read by id.
    let mut later = fleet(vec![session("c", "input")], "c");
    Arc::make_mut(&mut later.sessions)[0].state = "input".into();
    show(later, &mut visual);
    assert_eq!(
        keeps(&mut commands),
        [vec!["a".to_string(), "c".to_string()]]
    );
}

#[gpui::test]
fn archiving_a_paused_session_lets_it_go(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.settings
                .kept_open
                .insert(this.project_scope.clone(), [("a".to_string(), 1)].into());
            this.settings
                .archived
                .insert(this.project_scope.clone(), vec!["a".into()]);
            this.update_view(
                Arc::new(fleet(vec![session("a", "stopped")], "a")),
                window,
                cx,
            );
        })
    });
    visual.run_until_parked();
    assert!(kept(&workspace, &mut visual).is_empty());
}

#[gpui::test]
fn sending_from_a_paused_session_resumes_it_with_the_message(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.settings
                .kept_open
                .insert(this.project_scope.clone(), [("a".to_string(), 1)].into());
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
            this.settings
                .kept_open
                .insert(this.project_scope.clone(), [("a".to_string(), 1)].into());
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
