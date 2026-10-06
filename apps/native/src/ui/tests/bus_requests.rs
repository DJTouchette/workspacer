//! UI requests arriving over the bus: focus, spawn dialog, terminal, notices.
use super::*;

#[gpui::test]
fn terminal_bus_request_is_visible_unsupported_and_never_creates_hidden_work(
    cx: &mut TestAppContext,
) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.update(|window,cx|workspace.update(cx,|this,cx|{
        let data=serde_json::json!({"cwd":"/project","command":"echo requested","label":"Checks","parentSessionId":"manager"});
        let(intent,payload)=wks_native::ui_requests::parse("facade.openTerminal",&data).unwrap().unwrap();
        let mut next=state("a");next.ui_requests.push(wks_native::ui_requests::Request{number:1,intent,payload});
        this.update_view(Arc::new(next),window,cx);
        assert!(this.ui_bus.notice.contains("does not run hub terminal requests"));
        assert_eq!(this.screen,Screen::Conversation);
    }));
    assert!(matches!(
        commands.try_recv().unwrap(),
        Command::ConsumeUiRequest(1)
    ));
    assert!(commands.try_recv().is_err());
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
