//! Scrolling the conversation: wheel glide, reading history, the tail.
use super::*;

#[gpui::test]
fn wheel_notches_glide_instead_of_jumping(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    let items: Vec<Item> = (0..40)
        .map(|i| Item {
            kind: "assistant_text".into(),
            text: format!("Message {i}\n\nA paragraph to scroll past smoothly."),
            ..Default::default()
        })
        .collect();
    let mut view = state("a");
    view.transcript.snapshot(ConversationSnapshot {
        seq: 40,
        first_seq: 1,
        items,
    });
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| this.update_view(Arc::new(view), window, cx))
    });
    visual.run_until_parked();
    let top = |visual: &mut gpui::VisualTestContext| {
        visual.debug_bounds("last-transcript-row").unwrap().top()
    };
    let start = top(&mut visual);
    let wheel = |visual: &mut gpui::VisualTestContext, lines: f32| {
        visual.simulate_event(gpui::ScrollWheelEvent {
            position: gpui::point(px(600.), px(300.)),
            delta: gpui::ScrollDelta::Lines(gpui::point(0., lines)),
            ..Default::default()
        });
    };
    // One notch toward older messages: no instant jump…
    wheel(&mut visual, 3.);
    visual.run_until_parked();
    assert_eq!(
        top(&mut visual),
        start,
        "the list does not jump on the event"
    );
    // …a partial glide after a frame or two…
    visual
        .executor()
        .advance_clock(std::time::Duration::from_millis(20));
    visual.run_until_parked();
    let partway = top(&mut visual) - start;
    assert!(
        partway > px(0.) && partway < px(96.),
        "partway: {partway:?}"
    );
    // …and it settles on exactly three lines.
    visual
        .executor()
        .advance_clock(std::time::Duration::from_millis(600));
    visual.run_until_parked();
    assert!((f32::from(top(&mut visual) - start) - 96.).abs() < 0.01);
    workspace.read_with(&visual, |this, _| assert!(!this.follow, "reading history"));
    // Scrolling back down past the end re-pins the newest message.
    wheel(&mut visual, -30.);
    visual
        .executor()
        .advance_clock(std::time::Duration::from_millis(800));
    visual.run_until_parked();
    workspace.read_with(&visual, |this, _| assert!(this.follow));
    // Touchpad pixel deltas stay immediate.
    visual.simulate_event(gpui::ScrollWheelEvent {
        position: gpui::point(px(600.), px(300.)),
        delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(50.))),
        ..Default::default()
    });
    visual.run_until_parked();
    workspace.read_with(&visual, |this, _| assert!(!this.follow));
}

#[gpui::test]
fn wheel_over_a_scrollable_panel_stays_with_the_panel(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    let mut items: Vec<Item> = (0..30)
        .map(|i| Item {
            kind: "assistant_text".into(),
            text: format!("Message {i}"),
            ..Default::default()
        })
        .collect();
    items.push(Item {
        kind: "tool_use".into(),
        id: "fail-1".into(),
        name: "Bash".into(),
        input: serde_json::json!({"command":"cargo test"}),
        ..Default::default()
    });
    items.push(Item {
        kind: "tool_result".into(),
        tool_use_id: "fail-1".into(),
        is_error: true,
        content: (0..200).map(|n| format!("line {n}\n")).collect(),
        ..Default::default()
    });
    let mut view = state("a");
    view.transcript.snapshot(ConversationSnapshot {
        seq: items.len() as u64,
        first_seq: 1,
        items,
    });
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| this.update_view(Arc::new(view), window, cx))
    });
    visual.run_until_parked();
    // Failed calls open their details; the panel scrolls itself.
    let panel = visual.debug_bounds("tool-details").unwrap();
    visual.simulate_event(gpui::ScrollWheelEvent {
        position: panel.center(),
        delta: gpui::ScrollDelta::Lines(gpui::point(0., 3.)),
        ..Default::default()
    });
    workspace.read_with(&visual, |this, _| assert!(!this.smooth_scroll.gliding()));
    // Outside it, the same notch glides the conversation.
    visual.simulate_event(gpui::ScrollWheelEvent {
        position: gpui::point(
            workspace.read_with(&visual, |this, _| this.list.viewport_bounds().left()) + px(8.),
            panel.center().y,
        ),
        delta: gpui::ScrollDelta::Lines(gpui::point(0., 3.)),
        ..Default::default()
    });
    workspace.read_with(&visual, |this, _| assert!(this.smooth_scroll.gliding()));
    visual
        .executor()
        .advance_clock(std::time::Duration::from_millis(800));
    visual.run_until_parked();
}

#[gpui::test]
fn reading_history_stays_anchored_through_updates_and_scroll_stop(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    let mut items: Vec<Item> = (0..40)
        .map(|i| Item {
            kind: "assistant_text".into(),
            text: format!("Message {i}\n\nA paragraph to read without the view jumping."),
            ..Default::default()
        })
        .collect();
    let mut view = state("a");
    view.transcript.snapshot(ConversationSnapshot {
        seq: 40,
        first_seq: 1,
        items: items.clone(),
    });
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| this.update_view(Arc::new(view), window, cx))
    });
    visual.run_until_parked();
    let dock_height = workspace.read_with(&visual, |this, _| this.composer_dock_bounds.size.height);
    visual.simulate_event(gpui::ScrollWheelEvent {
        position: gpui::point(px(600.), px(300.)),
        delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(350.))),
        ..Default::default()
    });
    visual.run_until_parked();
    let (tracked_row, y) = workspace.read_with(&visual, |this, _| {
        assert!(!this.follow);
        assert_eq!(this.composer_dock_bounds.size.height, dock_height);
        let anchor = this.list.logical_scroll_top();
        // Header notices can change which row intersects the logical top.
        // Track content fully below the header, whose screen position must
        // remain fixed even when that first logical row advances.
        let tracked_row = (anchor.item_ix + 2).min(this.list.item_count() - 1);
        (
            tracked_row,
            this.list.bounds_for_item(tracked_row).unwrap().top()
                + this.header_bounds.size.height
                + px(16.),
        )
    });
    for round in 0..3 {
        items[0].text.push_str(" Changed earlier content.");
        items.push(Item {
            kind: "assistant_text".into(),
            text: format!("New tail {round}"),
            ..Default::default()
        });
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                let mut next = (*this.view).clone();
                next.notice = if round % 2 == 0 {
                    "Refreshing conversation".into()
                } else {
                    String::new()
                };
                next.transcript.snapshot(ConversationSnapshot {
                    seq: items.len() as u64,
                    first_seq: 1,
                    items: items.clone(),
                });
                this.update_view(Arc::new(next), window, cx);
            })
        });
        visual.run_until_parked();
        visual.simulate_event(gpui::ScrollWheelEvent {
            position: gpui::point(px(600.), px(300.)),
            touch_phase: gpui::TouchPhase::Ended,
            ..Default::default()
        });
        visual.run_until_parked();
        workspace.read_with(&visual, |this, _| {
            assert!(!this.follow);
            assert!(
                (this.list.bounds_for_item(tracked_row).unwrap().top()
                    + this.header_bounds.size.height
                    + px(16.)
                    - y)
                    .abs()
                    < px(1.)
            );
            assert_eq!(this.composer_dock_bounds.size.height, dock_height);
        });
    }
}

#[gpui::test]
fn scrolling_to_the_end_hides_jump_and_resumes_following(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    let mut view = state("a");
    view.transcript.snapshot(ConversationSnapshot {
        seq: 40,
        first_seq: 1,
        items: (0..40)
            .map(|i| Item {
                kind: "assistant_text".into(),
                text: format!("Message {i}\n\nA paragraph."),
                ..Default::default()
            })
            .collect(),
    });
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| this.update_view(Arc::new(view), window, cx))
    });
    visual.run_until_parked();
    for delta in [350., -100_000.] {
        visual.simulate_event(gpui::ScrollWheelEvent {
            position: gpui::point(px(600.), px(300.)),
            delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(delta))),
            ..Default::default()
        });
        visual.run_until_parked();
        workspace.read_with(&visual, |this, _| {
            assert_eq!(
                this.follow,
                delta < 0.,
                "delta={delta}, viewport={:?}, dock={:?}, tail={:?}, offset={:?}",
                this.list.viewport_bounds(),
                this.composer_dock_bounds,
                this.list.bounds_for_item(39),
                this.list.logical_scroll_top()
            )
        });
        assert_eq!(visual.debug_bounds("jump-latest").is_none(), delta < 0.);
    }
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            let mut next = (*this.view).clone();
            next.transcript.delta(
                wks_native::model::Delta {
                    seq: 41,
                    items: vec![Item {
                        kind: "assistant_text".into(),
                        text: "New reply".into(),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                false,
            );
            this.update_view(Arc::new(next), window, cx);
        })
    });
    visual.run_until_parked();
    workspace.read_with(&visual, |this, _| {
        assert!(this.follow);
        assert_eq!(
            this.list.logical_scroll_top().item_ix,
            this.list.item_count()
        );
    });
}

#[gpui::test]
fn reaching_the_tail_after_layout_resumes_follow_without_an_extra_wheel_event(
    cx: &mut TestAppContext,
) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    let mut view = state("a");
    view.transcript.snapshot(ConversationSnapshot {
        seq: 60,
        first_seq: 1,
        items: (0..60)
            .map(|i| Item {
                kind: "assistant_text".into(),
                text: format!("Message {i}\n\nSome content."),
                ..Default::default()
            })
            .collect(),
    });
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| this.update_view(Arc::new(view), window, cx))
    });
    visual.run_until_parked();
    visual.simulate_keystrokes("ctrl-u");
    visual.run_until_parked();
    assert!(visual.debug_bounds("jump-latest").is_some());
    for _ in 0..3 {
        visual.simulate_keystrokes("ctrl-d");
        visual.run_until_parked();
    }
    let tail = visual.debug_bounds("last-transcript-row").unwrap();
    workspace.read_with(&visual, |this, _| {
        assert!(tail.bottom() <= this.composer_dock_bounds.top());
        assert!(
            this.follow,
            "visible bottom must resume follow even when no wheel callback runs"
        );
    });
    // GPUI 0.2.2 retains debug_bounds entries across reused frames, so an
    // old selector's presence cannot establish whether a button was drawn.
}

#[gpui::test]
fn page_up_leaves_the_tail_and_repeated_pages_keep_moving(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    let mut view = state("a");
    view.transcript.snapshot(ConversationSnapshot {
        seq: 100,
        first_seq: 1,
        items: (0..100)
            .map(|i| Item {
                kind: "assistant_text".into(),
                text: format!("Message {i}\n\nSome content."),
                ..Default::default()
            })
            .collect(),
    });
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| this.update_view(Arc::new(view), window, cx))
    });
    visual.run_until_parked();
    visual.simulate_keystrokes("ctrl-u");
    visual.run_until_parked();
    let first = workspace.read_with(&visual, |this, _| {
        assert!(!this.follow);
        this.list.logical_scroll_top().item_ix
    });
    assert!(first < 99);
    visual.simulate_keystrokes("ctrl-u");
    visual.run_until_parked();
    workspace.read_with(&visual, |this, _| {
        assert!(this.list.logical_scroll_top().item_ix < first)
    });
    let jump = visual.debug_bounds("jump-latest").unwrap();
    visual.simulate_click(jump.center(), gpui::Modifiers::default());
    visual.run_until_parked();
    workspace.read_with(&visual, |this, _| {
        assert!(this.follow);
        assert_eq!(
            this.list.logical_scroll_top().item_ix,
            this.list.item_count()
        );
    });
}

#[gpui::test]
fn switching_back_lands_on_the_latest_message(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    visual.update(|window, _| window.activate_window());
    visual.run_until_parked();
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            let mut a = state("a");
            a.transcript.snapshot(ConversationSnapshot {
                seq: 50,
                first_seq: 1,
                items: (0..50)
                    .map(|i| Item {
                        kind: "user_message".into(),
                        text: format!("Message {i}"),
                        ..Default::default()
                    })
                    .collect(),
            });
            this.update_view(Arc::new(a.clone()), window, cx);
            assert!(this.follow);
            // Scroll up, leave, and come back after new activity.
            this.follow = false;
            this.list.scroll_to(ListOffset {
                item_ix: 12,
                offset_in_item: px(7.),
            });
            this.update_view(Arc::new(state("b")), window, cx);
            let mut loading = state("a");
            loading.loading = true;
            this.update_view(Arc::new(loading), window, cx);
            a.transcript.delta(
                wks_native::model::Delta {
                    seq: 51,
                    items: vec![Item {
                        kind: "assistant_text".into(),
                        text: "New activity".into(),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                true,
            );
            this.update_view(Arc::new(a), window, cx);
            assert!(this.follow);
        });
    });
    visual.run_until_parked();
    workspace.read_with(&visual, |this, _| {
        assert!(this.follow);
        assert_ne!(this.list.logical_scroll_top().item_ix, 12);
    });
}
