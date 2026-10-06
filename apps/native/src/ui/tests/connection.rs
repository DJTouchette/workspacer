//! Connection states: loading, pauses and a lost hub.
use super::*;

#[gpui::test]
fn loading_and_empty_states_follow_the_real_request_state(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.run_until_parked();
    assert!(visual.debug_bounds("state-connection").is_some());
    assert!(visual.debug_bounds("welcome-new").is_none());
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(
                Arc::new(View {
                    connected: true,
                    sessions_loading: true,
                    ..Default::default()
                }),
                window,
                cx,
            );
        })
    });
    visual.run_until_parked();
    assert!(visual.debug_bounds("state-sessions-loading").is_some());
    assert!(visual.debug_bounds("state-no-sessions").is_none());
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(
                Arc::new(View {
                    connected: true,
                    ..Default::default()
                }),
                window,
                cx,
            );
        })
    });
    visual.run_until_parked();
    assert!(visual.debug_bounds("state-no-sessions").is_some());
    assert!(visual.debug_bounds("welcome-setup").is_some());
    assert!(commands.try_recv().is_err());
}

#[gpui::test]
fn connection_pause_banner_requires_a_click_and_keeps_scrollback(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    let mut next = state("a");
    next.transcript.snapshot(ConversationSnapshot {
        seq: 1,
        first_seq: 1,
        items: vec![Item {
            kind: "assistant_text".into(),
            text: "Saved conversation".into(),
            ..Default::default()
        }],
    });
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(next.clone()), window, cx)
        })
    });
    next.connected = false;
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(next.clone()), window, cx)
        })
    });
    visual.run_until_parked();
    workspace.read_with(&visual, |this, _| {
        assert_eq!(this.connection_copy().label, "Reconnecting…");
        assert!(this.connection_copy().animated);
    });
    assert!(visual.debug_bounds("last-transcript-row").is_some());
    assert!(visual.debug_bounds("wake-workspace").is_none());
    next.power_paused = true;
    next.can_resume_power_pause = true;
    next.power_pause_generation = 42;
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(next), window, cx);
            this.composer
                .update(cx, |input, cx| input.set_value("Offline draft", window, cx));
        })
    });
    visual.run_until_parked();
    assert!(visual.debug_bounds("last-transcript-row").is_some());
    assert!(visual.debug_bounds("connection-banner").is_some());
    workspace.read_with(&visual, |this, _| assert!(!this.connection_copy().animated));
    assert!(commands.try_recv().is_err());
    let wake = visual.debug_bounds("wake-workspace").unwrap();
    visual.simulate_click(wake.center(), gpui::Modifiers::default());
    assert!(matches!(
        commands.try_recv().unwrap(),
        Command::ResumePowerPause(42)
    ));
    workspace.read_with(&visual, |this, cx| {
        assert_eq!(this.composer.read(cx).value().as_str(), "Offline draft")
    });
    assert!(commands.try_recv().is_err());
}

#[gpui::test]
fn power_pause_waits_for_an_explicit_connection_control(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            let mut next = state("a");
            next.connected = false;
            next.power_paused = true;
            next.power_pause_generation = 42;
            next.can_resume_power_pause = true;
            next.notice = "Server requested a reconnect pause.".into();
            this.update_view(Arc::new(next), window, cx);
            assert!(this.view.power_paused);
        })
    });
    assert!(commands.try_recv().is_err());
    visual.simulate_keystrokes("ctrl-r");
    assert!(matches!(
        commands.try_recv().unwrap(),
        Command::ResumePowerPause(42)
    ));
    assert!(commands.try_recv().is_err());
}

// GPUI never clears `debug_bounds` between frames (vendor/gpui Frame::
// clear), so an element that disappeared still reports its old bounds.
// Absence is asserted on the state that decides it, presence on bounds.
#[gpui::test]
fn sidebar_footer_omits_a_healthy_connection_and_flags_a_lost_one(cx: &mut TestAppContext) {
    let (workspace, mut visual, _, _) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.demo = false;
            this.update_view(Arc::new(state("a")), window, cx);
            assert_eq!(this.footer_connection_label(), None);
            let mut next = state("a");
            next.connected = false;
            this.update_view(Arc::new(next), window, cx);
            assert!(this.footer_connection_label().is_some());
            this.demo = true;
            assert_eq!(this.footer_connection_label(), Some("Demo"));
            this.demo = false;
        })
    });
    visual.run_until_parked();
    let status = visual.debug_bounds("sidebar-connection-status").unwrap();
    let sidebar = visual.debug_bounds("session-sidebar").unwrap();
    assert!(sidebar.contains(&status.center()));
}
