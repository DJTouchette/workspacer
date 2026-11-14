//! UI requests arriving over the bus: focus, spawn dialog, terminal, notices.
use super::*;

/// Queue one bus request on a two-session view with `a` selected.
fn deliver(
    workspace: &Entity<Workspace>,
    visual: &mut VisualTestContext,
    number: u64,
    topic: &str,
    data: serde_json::Value,
) {
    let topic = topic.to_owned();
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            let (intent, payload) = wks_native::ui_requests::parse(&topic, &data)
                .unwrap()
                .unwrap();
            let mut next = state("a");
            Arc::make_mut(&mut next.sessions)[0].cwd = "/work/a".into();
            Arc::make_mut(&mut next.sessions)[1].cwd = "/work/b".into();
            next.ui_requests.push(wks_native::ui_requests::Request {
                number,
                intent,
                payload,
            });
            this.update_view(Arc::new(next), window, cx);
        })
    });
    visual.run_until_parked();
}

#[gpui::test]
fn a_terminal_request_is_offered_and_runs_only_when_the_person_says_so(cx: &mut TestAppContext) {
    use wks_native::terminal::Command as T;
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    deliver(
        &workspace,
        &mut visual,
        1,
        "facade.openTerminal",
        serde_json::json!({"cwd":"/work/a","command":"cargo test\n","label":"Checks","parentSessionId":"manager"}),
    );
    // Shown, with the command readable; nothing was started or typed.
    workspace.read_with(&visual, |this, _| {
        assert_eq!(
            this.ui_bus.notice,
            "The hub asks to run “Checks” in a terminal in /work/a: cargo test"
        );
        assert_eq!(this.screen, Screen::Conversation);
        assert!(!this.terminal.open);
    });
    assert!(matches!(
        effects(&mut commands).as_slice(),
        [Command::ConsumeUiRequest(1)]
    ));

    // The person runs it: the selected session's terminal opens (the
    // requesting session is not one of this hub's) and takes the command.
    click(&mut visual, "run-ui-request");
    let ran = effects(&mut commands);
    assert!(matches!(ran.as_slice(), [
            Command::Terminal(T::Open { agent, cwd, .. }),
            Command::Terminal(T::Input { agent: typed, bytes }),
        ] if agent == "a" && cwd == "/work/a" && typed == "a" && bytes == b"cargo test\r"));
    workspace.read_with(&visual, |this, _| {
        assert!(this.terminal.open);
        assert!(this.ui_bus.notice.is_empty(), "{}", this.ui_bus.notice);
    });
}

#[gpui::test]
fn a_command_for_another_session_waits_for_that_session_and_changes_folder_first(
    cx: &mut TestAppContext,
) {
    use wks_native::terminal::Command as T;
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    deliver(
        &workspace,
        &mut visual,
        1,
        "facade.openTerminal",
        serde_json::json!({"cwd":"/work/b/api","command":"make check\n","parentSessionId":"b"}),
    );
    let _ = effects(&mut commands);
    click(&mut visual, "run-ui-request");
    // Its session is selected first; nothing is typed into another shell.
    let asked = effects(&mut commands);
    assert!(matches!(asked.as_slice(), [Command::Select(id)] if id == "b"));
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            let mut view = (*this.view).clone();
            view.selected = Some("b".into());
            view.ui_requests.clear();
            this.update_view(Arc::new(view), window, cx)
        })
    });
    visual.run_until_parked();
    let ran = effects(&mut commands);
    assert!(matches!(ran.as_slice(), [
            Command::Terminal(T::Open { agent, .. }),
            Command::Terminal(T::Input { agent: typed, bytes }),
        ] if agent == "b" && typed == "b" && bytes == b"cd -- '/work/b/api' && make check\r"));
}

#[gpui::test]
fn pane_requests_open_their_native_counterpart_and_a_page_is_only_offered(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    deliver(
        &workspace,
        &mut visual,
        1,
        "command.open_pane",
        serde_json::json!({"paneType":"analytics"}),
    );
    workspace.read_with(&visual, |this, _| {
        assert!(this.usage_open);
        assert!(this.ui_bus.notice.is_empty());
    });
    deliver(
        &workspace,
        &mut visual,
        2,
        "command.open_pane",
        serde_json::json!({"paneType":"browser","url":"https://example.com/docs"}),
    );
    workspace.read_with(&visual, |this, _| {
        assert_eq!(
            this.ui_bus.notice,
            "The hub asks to show a web page: https://example.com/docs"
        );
    });
    assert!(visual.debug_bounds("open-ui-page").is_some());
    deliver(
        &workspace,
        &mut visual,
        3,
        "command.open_pane",
        serde_json::json!({"paneType":"terminal"}),
    );
    workspace.read_with(&visual, |this, _| assert!(this.terminal.open));
    let opened = effects(&mut commands);
    assert!(
        opened
            .iter()
            .any(|c| matches!(c, Command::Terminal(wks_native::terminal::Command::Open { agent, .. }) if agent == "a"))
    );
}

#[gpui::test]
fn spawn_dialog_bus_request_only_prefills_and_decision_actions_are_refused(
    cx: &mut TestAppContext,
) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.demo = false;
            let mut next = state("a");
            next.ui_requests.push(wks_native::ui_requests::Request {
                number: 1,
                intent: wks_native::ui_requests::Intent::OpenSpawnDialog {
                    cwd: "/requested/project".into(),
                },
                payload: serde_json::json!({"cwd":"/requested/project"}),
            });
            this.update_view(Arc::new(next), window, cx);
            assert!(this.new_session);
            assert_eq!(this.projects.cwd.as_str(), "/requested/project");
            let mut next = state("a");
            next.ui_requests.push(wks_native::ui_requests::Request {
                number: 2,
                intent: wks_native::ui_requests::Intent::RunAction {
                    action: "fleet-approve-yes".into(),
                    digit: None,
                },
                payload: serde_json::json!({"action":"fleet-approve-yes"}),
            });
            this.update_view(Arc::new(next), window, cx);
            assert!(this.ui_bus.notice.contains("scoped controls"));
        })
    });
    while let Ok(command) = commands.try_recv() {
        assert!(matches!(command, Command::ConsumeUiRequest(_)) || project_read(&command));
    }
}

// An agent's `notify` (or a hub notice such as a failed job) is shown in the
// window, pinned or not, and never navigates. A quiet one posts no OS alert.
#[gpui::test]
fn a_notification_is_shown_in_a_pinned_window_and_a_quiet_one_posts_no_alert(
    cx: &mut TestAppContext,
) {
    features::POSTED_ALERTS.take();
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.requested_session = Some("a".into());
            let screen = this.screen;
            let (intent, payload) = wks_native::ui_requests::parse(
                "notify.post",
                &serde_json::json!({"title":"Job failed: nightly","body":"exit 2","sessionId":"b","inAppOnly":true}),
            )
            .unwrap()
            .unwrap();
            let mut next = state("a");
            next.ui_requests.push(wks_native::ui_requests::Request {
                number: 1,
                intent,
                payload,
            });
            this.update_view(Arc::new(next), window, cx);
            assert_eq!(this.ui_bus.notice, "Job failed: nightly: exit 2");
            assert_eq!(this.screen, screen);
            assert_eq!(this.view.selected.as_deref(), Some("a"));
        })
    });
    visual.run_until_parked();
    assert!(features::POSTED_ALERTS.take().is_empty());
}

#[gpui::test]
fn focus_bus_request_preserves_pinned_window_and_reviews_keep_explicit_cwd(
    cx: &mut TestAppContext,
) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.requested_session = Some("a".into());
            let mut next = state("a");
            next.ui_requests.push(wks_native::ui_requests::Request {
                number: 1,
                intent: wks_native::ui_requests::Intent::FocusAgent("b".into()),
                payload: serde_json::json!({"sessionId":"b"}),
            });
            this.update_view(Arc::new(next), window, cx);
            assert!(this.ui_bus.notice.contains("pinned"));
            this.requested_session = None;
            let mut next = state("a");
            next.ui_requests.push(wks_native::ui_requests::Request {
                number: 2,
                intent: wks_native::ui_requests::Intent::OpenPane {
                    pane_type: "review".into(),
                    cwd: "/different/project".into(),
                    url: String::new(),
                },
                payload: serde_json::json!({"paneType":"review","cwd":"/different/project"}),
            });
            this.update_view(Arc::new(next), window, cx);
            assert_eq!(this.screen, Screen::Changes);
        })
    });
    assert!(matches!(
        commands.try_recv().unwrap(),
        Command::ConsumeUiRequest(1)
    ));
    assert!(matches!(
        commands.try_recv().unwrap(),
        Command::ConsumeUiRequest(2)
    ));
    assert!(
        matches!(commands.try_recv().unwrap(),Command::Request(wks_native::features::Request::Changes{cwd}) if cwd=="/different/project")
    );
    assert!(commands.try_recv().is_err());
}

#[gpui::test]
fn unavailable_request_notice_is_a_compact_card_with_actions(cx: &mut TestAppContext) {
    let (workspace, mut visual, _, _) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(state("a")), window, cx);
            this.ui_bus.notice = "Requested session 00d2709e is unavailable on this hub.".into();
            this.ui_bus.payload_for_test(
                serde_json::json!({"sessionId":"00d2709e-977f-4dea-af7d-e33ba01e90ec"}),
            );
            cx.notify();
        })
    });
    visual.run_until_parked();
    let sidebar = visual.debug_bounds("session-sidebar").unwrap();
    let notice = visual.debug_bounds("sidebar-ui-notice").unwrap();
    assert!(notice.left() >= sidebar.left() && notice.right() <= sidebar.right());
    let copy = visual.debug_bounds("copy-ui-request").unwrap();
    let dismiss = visual.debug_bounds("dismiss-ui-request").unwrap();
    assert!(notice.contains(&copy.center()) && notice.contains(&dismiss.center()));
    assert!(copy.size.height <= px(34.), "copy {:?}", copy.size);
    assert!(dismiss.size.height <= px(28.), "dismiss {:?}", dismiss.size);
    // Dismiss sits in the header row, not under the message.
    assert!(dismiss.top() < copy.top());
    visual.simulate_click(dismiss.center(), gpui::Modifiers::default());
    visual.run_until_parked();
    workspace.read_with(&visual, |this, _| assert!(this.ui_bus.notice.is_empty()));
}
