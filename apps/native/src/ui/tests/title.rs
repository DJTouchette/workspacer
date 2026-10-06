//! The title capsule: its island of notices, revealed actions and context gauge.
use super::*;

/// The last frame's text-selection highlights.
fn painted_selection(visual: &mut VisualTestContext) -> Vec<gpui::Bounds<gpui::Pixels>> {
    visual.update(|window, cx| {
        let selection = gpui_component::ActiveTheme::theme(cx).selection;
        window
            .rendered_fills()
            .into_iter()
            .filter(|(_, color)| *color == selection)
            .map(|(bounds, _)| bounds)
            .collect()
    })
}

/// A press and its release delivered before the next frame, as a
/// touchpad tap or a synthetic click arrives.
fn tap(visual: &mut VisualTestContext, at: gpui::Point<gpui::Pixels>) {
    let (modifiers, button) = (gpui::Modifiers::default(), gpui::MouseButton::Left);
    visual.simulate_events_in_one_frame(vec![
        gpui::PlatformInput::MouseDown(gpui::MouseDownEvent {
            position: at,
            modifiers,
            button,
            click_count: 1,
            first_mouse: false,
        }),
        gpui::PlatformInput::MouseUp(gpui::MouseUpEvent {
            position: at,
            modifiers,
            button,
            click_count: 1,
        }),
    ]);
}

/// Chat history scrolls under the floating title island. A press on an
/// island control (here a notice's dismiss) belongs to that control: the
/// transcript under it must not start a text selection that the pointer
/// then drags on, and the press clears one already there, as any press
/// outside the text does.
#[gpui::test]
fn island_controls_do_not_select_the_transcript_under_them(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    let paragraphs = (0..40)
        .map(|i| {
            format!(
                "Paragraph {i}: a line long enough to reach under every island control and wrap."
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    let show = |visual: &mut VisualTestContext| {
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                let mut view = state("a");
                view.notice = "Model change accepted: claude-opus-5-5 · High effort".into();
                view.transcript.snapshot(ConversationSnapshot {
                    seq: 1,
                    first_seq: 1,
                    items: vec![Item {
                        kind: "assistant_text".into(),
                        text: paragraphs.clone(),
                        ..Default::default()
                    }],
                });
                this.extras.dismissed_notices.clear();
                this.update_view(Arc::new(view), window, cx);
            })
        });
        visual.run_until_parked();
    };
    let dismissed = |visual: &mut VisualTestContext| {
        workspace.read_with(visual, |this, _| this.extras.dismissed_notices.len())
    };
    show(&mut visual);
    let dismiss = visual.debug_bounds("dismiss-status-notice").unwrap();
    // The transcript's text really is under the dismiss button.
    let transcript = visual
        .debug_bounds("markdown-inline-live:a:0-0")
        .expect("assistant markdown");
    assert!(
        transcript.top() < dismiss.top() && transcript.right() > dismiss.right(),
        "{transcript:?} under {dismiss:?}"
    );
    assert!(painted_selection(&mut visual).is_empty());

    // A tap on the dismiss, then the pointer goes on its way over the
    // transcript.
    let away = gpui::point(transcript.left() + px(40.), dismiss.center().y + px(200.));
    tap(&mut visual, dismiss.center());
    assert_eq!(dismissed(&mut visual), 1);
    visual.simulate_mouse_move(away, None, gpui::Modifiers::default());
    visual.run_until_parked();
    let painted = painted_selection(&mut visual);
    assert!(painted.is_empty(), "tap left a selection: {painted:?}");

    // Nor does a held press dragged off the control onto the text.
    show(&mut visual);
    let at = dismiss.center();
    visual.simulate_mouse_down(at, gpui::MouseButton::Left, gpui::Modifiers::default());
    visual.simulate_mouse_move(
        at - gpui::point(px(200.), px(-100.)),
        Some(gpui::MouseButton::Left),
        gpui::Modifiers::default(),
    );
    let painted = painted_selection(&mut visual);
    assert!(painted.is_empty(), "press selected: {painted:?}");
    visual.simulate_mouse_up(at, gpui::MouseButton::Left, gpui::Modifiers::default());
    visual.run_until_parked();
    assert_eq!(dismissed(&mut visual), 1);

    // Text selection itself still works, and a press on the island
    // clears it, as a press anywhere outside the text does.
    show(&mut visual);
    let a = gpui::point(transcript.left() + px(10.), away.y);
    let b = gpui::point(transcript.left() + px(300.), away.y + px(40.));
    visual.simulate_mouse_down(a, gpui::MouseButton::Left, gpui::Modifiers::default());
    visual.simulate_mouse_move(b, Some(gpui::MouseButton::Left), gpui::Modifiers::default());
    visual.simulate_mouse_up(b, gpui::MouseButton::Left, gpui::Modifiers::default());
    visual.run_until_parked();
    assert!(!painted_selection(&mut visual).is_empty(), "drag selects");
    visual.simulate_click(dismiss.center(), gpui::Modifiers::default());
    visual.run_until_parked();
    assert_eq!(dismissed(&mut visual), 1);
    let painted = painted_selection(&mut visual);
    assert!(painted.is_empty(), "island press kept: {painted:?}");

    // The capsule itself (its title, its hover-revealed actions) is the
    // same occluding island.
    let bar = visual.debug_bounds("title-bar").unwrap();
    tap(&mut visual, bar.center());
    visual.simulate_mouse_move(away, None, gpui::Modifiers::default());
    visual.run_until_parked();
    let painted = painted_selection(&mut visual);
    assert!(
        painted.is_empty(),
        "capsule tap left a selection: {painted:?}"
    );

    // A tap on the text itself selects nothing, and the pointer moving on
    // afterwards does not drag a selection along.
    visual.simulate_click(a, gpui::Modifiers::default());
    visual.run_until_parked();
    tap(&mut visual, a);
    visual.simulate_mouse_move(b, None, gpui::Modifiers::default());
    visual.run_until_parked();
    let painted = painted_selection(&mut visual);
    assert!(painted.is_empty(), "text tap kept selecting: {painted:?}");
}

#[gpui::test]
fn title_notice_dismiss_supports_tab_enter_and_space_without_sending(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    for key in ["enter", "space"] {
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                let mut next = state("a");
                next.notice = format!("Session created {key}");
                this.update_view(Arc::new(next), window, cx);
                this.composer
                    .update(cx, |input, cx| input.set_value("KEEP_DRAFT", window, cx));
            })
        });
        visual.run_until_parked();
        let bounds = visual.debug_bounds("dismiss-status-notice").unwrap();
        // Focus by mouse-down, then release outside so no click activates.
        visual.simulate_mouse_down(
            bounds.center(),
            gpui::MouseButton::Left,
            gpui::Modifiers::default(),
        );
        visual.simulate_mouse_up(
            gpui::point(gpui::px(0.), gpui::px(0.)),
            gpui::MouseButton::Left,
            gpui::Modifiers::default(),
        );
        visual.run_until_parked();
        let focus = visual.update(|window, cx| window.focused(cx).unwrap());
        workspace.read_with(&visual, |this, _| {
            assert!(this.extras.dismissed_notices.is_empty())
        });
        visual.simulate_keystrokes("tab shift-tab");
        visual.run_until_parked();
        assert_eq!(
            visual.update(|window, cx| window.focused(cx).unwrap()),
            focus
        );
        visual.simulate_keystrokes(key);
        visual.simulate_event(gpui::KeyUpEvent {
            keystroke: gpui::Keystroke::parse(key).unwrap(),
        });
        visual.run_until_parked();
        workspace.read_with(&visual, |this, cx| {
            assert!(
                this.extras
                    .dismissed_notices
                    .iter()
                    .any(|(slot, text)| *slot == "status"
                        && text == &format!("Session created {key}"))
            );
            assert_eq!(this.composer.read(cx).value().as_str(), "KEEP_DRAFT");
        });
        assert!(commands.try_recv().is_err());
    }
}

/// Bug #27: conversation notices are part of the title capsule, an island
/// that grows beneath it with the capsule's surface, border, shadow and
/// curve; long text wraps at the capsule's width; dismissal is per text;
/// an error's retry stays with it. The capsule itself never moves.
#[gpui::test]
fn title_notices_grow_the_capsule_into_one_island(cx: &mut TestAppContext) {
    let _caption = CaptionPreview::new();
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    let with_notice = |notice: &str| {
        let mut view = state("a");
        view.notice = notice.into();
        view.transcript.snapshot(ConversationSnapshot {
            seq: 12,
            first_seq: 1,
            items: (0..12)
                .map(|i| Item {
                    kind: "assistant_text".into(),
                    text: format!("Message {i}"),
                    ..Default::default()
                })
                .collect(),
        });
        Arc::new(view)
    };
    let show = |visual: &mut VisualTestContext, notice: &str| {
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.update_view(with_notice(notice), window, cx)
            })
        });
        visual.run_until_parked();
    };
    let header = |visual: &mut VisualTestContext| {
        workspace.read_with(visual, |this, _| this.header_bounds.size.height)
    };
    for (width, height) in [(1000., 700.), (720., 480.), (1600., 900.)] {
        visual.simulate_resize(size(px(width), px(height)));
        show(&mut visual, "");
        // Measured with its actions showing: the island holds that room
        // whether or not they show (see the reveal test below).
        reveal_title(&workspace, &mut visual);
        let bare = header(&mut visual);
        let capsule = visual.debug_bounds("title-bar").unwrap();
        for notice in [
            "Change queued; the provider will apply it when ready",
            "Model change accepted: claude-opus-5-5 · High effort",
            "Model change refused: this provider cannot switch models while a turn is running, so the request was not applied and nothing changed in the session.",
        ] {
            show(&mut visual, notice);
            let case = format!("{width}x{height} {notice:?}");
            let island = visual.debug_bounds("title-island").unwrap();
            let tray = visual.debug_bounds("title-island-notices").unwrap();
            let bar = visual.debug_bounds("title-bar").unwrap();
            let row = visual.debug_bounds("island-notice-status").unwrap();
            // The capsule's outline stays put and only grows downward,
            // so a long message wraps instead of widening it.
            assert_eq!(island.origin, capsule.origin, "{case}");
            assert_eq!(island.size.width, capsule.size.width, "{case}");
            assert_eq!(bar.size.height, capsule.size.height - px(2.), "{case}");
            // Attached: the tray starts where the capsule ends, inside
            // the island, and holds the notice.
            assert!(
                (tray.top() - bar.bottom()).abs() < px(1.),
                "{case}: {tray:?} {bar:?}"
            );
            assert!(
                tray.left() >= island.left() && tray.right() <= island.right() + px(0.5),
                "{case}"
            );
            assert!(
                tray.bottom() <= island.bottom(),
                "{case}: {tray:?} {island:?}"
            );
            assert!(
                row.top() >= tray.top() && row.bottom() <= tray.bottom(),
                "{case}: {row:?} {tray:?}"
            );
            // Clear of the app-drawn caption buttons and the window.
            assert!(
                island.right() <= px(width - chrome::CAPTION_WIDTH),
                "{case}: {island:?}"
            );
            assert!(
                island.left() >= px(0.) && island.bottom() < px(height / 2.),
                "{case}: {island:?}"
            );
            // The header grows with it, so the transcript is pushed, not covered.
            assert!(
                header(&mut visual) >= bare + tray.size.height - px(1.),
                "{case}"
            );
        }
        // The long refusal wrapped onto several lines within the capsule width.
        let row = visual.debug_bounds("island-notice-status").unwrap();
        assert!(row.size.height > px(40.), "{width}x{height}: {row:?}");

        // Dismissing hides that text; the island collapses to the capsule.
        let dismiss = visual.debug_bounds("dismiss-status-notice").unwrap();
        visual.simulate_click(dismiss.center(), gpui::Modifiers::default());
        visual.run_until_parked();
        assert_eq!(header(&mut visual), bare, "{width}x{height}: dismissed");
        assert_eq!(visual.debug_bounds("title-bar").unwrap(), capsule);
        // The same words again after the slot moved on are news again.
        show(&mut visual, "");
        show(
            &mut visual,
            "Model change accepted: claude-opus-5-5 · High effort",
        );
        assert!(
            header(&mut visual) > bare,
            "{width}x{height}: repeated notice"
        );
    }

    // An unavailable conversation keeps its error and Retry attached,
    // without a dismiss that would drop the only retry.
    visual.simulate_resize(size(px(1000.), px(700.)));
    show(&mut visual, "Conversation unavailable: hub timed out");
    let island = visual.debug_bounds("title-island").unwrap();
    let retry = visual.debug_bounds("retry-conversation").unwrap();
    assert!(island.contains(&retry.center()), "{retry:?} in {island:?}");
    let before = header(&mut visual);
    let stale_dismiss = visual.debug_bounds("dismiss-status-notice").unwrap();
    visual.simulate_click(stale_dismiss.center(), gpui::Modifiers::default());
    visual.run_until_parked();
    assert_eq!(
        header(&mut visual),
        before,
        "no dismiss on an error with its retry"
    );
    workspace.read_with(&visual, |this, _| {
        assert!(this.extras.dismissed_notices.is_empty())
    });
    while commands.try_recv().is_ok() {}
    visual.simulate_click(retry.center(), gpui::Modifiers::default());
    assert!(matches!(commands.try_recv(), Ok(Command::Refresh)));

    // Local feature notices share the island, each with its own tone row.
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(with_notice(""), window, cx);
            this.extras.notice = "Attachment failed: too large".into();
            cx.notify();
        })
    });
    visual.run_until_parked();
    let island = visual.debug_bounds("title-island").unwrap();
    let feature = visual.debug_bounds("island-notice-feature").unwrap();
    assert!(
        island.contains(&feature.center()),
        "{feature:?} in {island:?}"
    );
    let dismiss = visual.debug_bounds("dismiss-feature-notice").unwrap();
    visual.simulate_click(dismiss.center(), gpui::Modifiers::default());
    visual.run_until_parked();
    workspace.read_with(&visual, |this, _| {
        // Dismissal hides it here; the slot's text is left to its owner.
        assert_eq!(this.extras.notice, "Attachment failed: too large");
        assert_eq!(
            this.extras.dismissed_notices,
            vec![("feature", "Attachment failed: too large".to_owned())]
        );
    });
}

/// With motion on, the island grows into its notices rather than
/// snapping: mid-way it is part-tall and part-wide around its center,
/// the transcript follows the measured header, and at rest it is exactly
/// the snapped island. A dismissed row keeps its words while the room
/// closes, then the capsule is exactly the resting capsule again, and
/// nothing keeps rendering once it settles.
#[gpui::test]
fn title_island_springs_into_notices_and_lets_go(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    let with_notice = |notice: &str| {
        let mut view = state("a");
        view.notice = notice.into();
        view.transcript.snapshot(ConversationSnapshot {
            seq: 12,
            first_seq: 1,
            items: (0..12)
                .map(|i| Item {
                    kind: "assistant_text".into(),
                    text: format!("Message {i}"),
                    ..Default::default()
                })
                .collect(),
        });
        Arc::new(view)
    };
    let show = |visual: &mut VisualTestContext, notice: &str| {
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.update_view(with_notice(notice), window, cx)
            })
        });
        visual.run_until_parked();
    };
    let island = |visual: &mut VisualTestContext, f: fn(&mut Workspace)| {
        visual.update(|_, cx| {
            workspace.update(cx, |this, cx| {
                f(this);
                cx.notify();
            })
        });
        visual.run_until_parked();
    };
    let header = |visual: &mut VisualTestContext| {
        workspace.read_with(visual, |this, _| this.header_bounds.size.height)
    };
    let notice = "Model change refused: this provider cannot switch models while a turn is running, so nothing changed.";

    // Where everything belongs, from the snapped (reduced motion) path.
    show(&mut visual, "");
    settle_title(&workspace, &mut visual);
    let capsule = visual.debug_bounds("title-bar").unwrap();
    let bare = header(&mut visual);
    show(&mut visual, notice);
    let snapped = visual.debug_bounds("title-island").unwrap();
    let snapped_tray = visual.debug_bounds("title-island-notices").unwrap();
    let snapped_header = header(&mut visual);
    show(&mut visual, "");
    assert_eq!(visual.debug_bounds("title-bar").unwrap(), capsule);

    visual.update(|_, cx| workspace.update(cx, |this, _| this.settings.reduce_motion = false));
    show(&mut visual, notice);
    assert!(workspace.read_with(&visual, |this, _| this.island_moving()));
    // Half-way: the island is part-grown down and out about its center,
    // and the transcript is pushed by just that much.
    island(&mut visual, |this| this.freeze_island(0.5));
    let half = visual.debug_bounds("title-island").unwrap();
    assert!(
        half.size.width > capsule.size.width + px(40.)
            && half.size.width < snapped.size.width - px(40.),
        "{half:?} between {capsule:?} and {snapped:?}"
    );
    assert!((half.center().x - snapped.center().x).abs() <= px(1.));
    assert_eq!(half.top(), snapped.top());
    assert!(
        half.size.height > capsule.size.height + px(4.)
            && half.size.height < snapped.size.height - px(4.),
        "{half:?}"
    );
    let tray = visual.debug_bounds("title-island-notices").unwrap();
    assert!(tray.right() <= half.right() && tray.left() >= half.left());
    let pushed = header(&mut visual);
    assert!(
        bare < pushed && pushed < snapped_header,
        "{bare:?} {pushed:?}"
    );

    // At rest it is exactly the snapped island, and stays put.
    island(&mut visual, |this| this.settle_island());
    assert_eq!(visual.debug_bounds("title-island").unwrap(), snapped);
    assert_eq!(
        visual.debug_bounds("title-island-notices").unwrap(),
        snapped_tray
    );
    assert_eq!(header(&mut visual), snapped_header);
    assert!(!workspace.read_with(&visual, |this, _| this.island_moving()));

    // Dismissed: the words stay a moment as a ghost, holding the room
    // and no longer clickable; then the capsule is back exactly.
    let dismiss = visual.debug_bounds("dismiss-status-notice").unwrap();
    visual.simulate_click(dismiss.center(), gpui::Modifiers::default());
    visual.run_until_parked();
    assert!(workspace.read_with(&visual, |this, _| this.island_moving()));
    let leaving = visual.debug_bounds("title-island").unwrap();
    assert_eq!(
        leaving.size.height, snapped.size.height,
        "content leaves first"
    );
    island(&mut visual, |this| this.freeze_island(0.4));
    let closing = visual.debug_bounds("title-island").unwrap();
    assert!(closing.size.height < snapped.size.height - px(4.));
    assert!(closing.size.height > capsule.size.height);
    island(&mut visual, |this| this.settle_island());
    assert_eq!(visual.debug_bounds("title-bar").unwrap(), capsule);
    assert_eq!(header(&mut visual), bare);
    assert!(!workspace.read_with(&visual, |this, _| this.island_moving()));

    // In real time it settles by itself and stops asking for frames.
    // (Once the slot moves on, the same words are news again.)
    show(&mut visual, "");
    show(&mut visual, notice);
    std::thread::sleep(std::time::Duration::from_millis(900));
    // (The next frame, as the animation frame it asked for would draw.)
    island(&mut visual, |_| {});
    assert!(!workspace.read_with(&visual, |this, _| this.island_moving()));
    assert_eq!(visual.debug_bounds("title-island").unwrap(), snapped);
    assert_eq!(header(&mut visual), snapped_header);
}

/// The title capsule rests compact, like a Dynamic Island: its secondary
/// actions show only while the pointer is on it, focus is inside it or a
/// tap pinned it. Hidden or half-shown actions take no pointer clicks,
/// the capsule grows about its center without moving the transcript,
/// and beside notices the island keeps the actions' room, so revealing
/// them never rewraps or moves a notice row.
#[gpui::test]
fn title_actions_show_only_on_hover_focus_or_tap(cx: &mut TestAppContext) {
    use wks_native::terminal::Command as T;
    const ACTIONS: [&str; 6] = [
        "open-changes",
        "open-editor",
        "open-terminal",
        "open-history",
        "open-session",
        "open-model",
    ];
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    let with_notice = |notice: &str| {
        let mut view = state("a");
        Arc::make_mut(&mut view.sessions)[0].model = "claude-opus-5-5".into();
        view.notice = notice.into();
        view.transcript.snapshot(ConversationSnapshot {
            seq: 12,
            first_seq: 1,
            items: (0..12)
                .map(|i| Item {
                    kind: "assistant_text".into(),
                    text: format!("Message {i}"),
                    ..Default::default()
                })
                .collect(),
        });
        Arc::new(view)
    };
    let show = |visual: &mut VisualTestContext, notice: &str| {
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.update_view(with_notice(notice), window, cx)
            })
        });
        visual.run_until_parked();
    };
    let header = |visual: &mut VisualTestContext| {
        workspace.read_with(visual, |this, _| this.header_bounds.size.height)
    };
    let shown = |visual: &mut VisualTestContext| {
        visual
            .debug_bounds("title-actions-shown")
            .unwrap()
            .size
            .width
    };
    // (pointer on the capsule, where the reveal is headed)
    let reveal = |visual: &mut VisualTestContext| {
        workspace.read_with(visual, |this, _| {
            (this.title_reveal.hovered, this.title_reveal_target())
        })
    };
    let point = |x: f32, y: f32| gpui::point(px(x), px(y));
    let move_to = |visual: &mut VisualTestContext, at: gpui::Point<gpui::Pixels>| {
        visual.simulate_mouse_move(at, None, gpui::Modifiers::default());
        visual.run_until_parked();
    };
    let untouched = |visual: &mut VisualTestContext,
                     commands: &mut tokio::sync::mpsc::Receiver<Command>,
                     case: &str| {
        assert!(effects(commands).is_empty(), "{case}");
        workspace.read_with(visual, |this, _| {
            assert_eq!(this.screen, Screen::Conversation, "{case}");
            assert!(!this.terminal.open, "{case}");
        });
    };

    for (width, height) in [(1000., 700.), (720., 480.), (1600., 900.)] {
        visual.simulate_resize(size(px(width), px(height)));
        show(&mut visual, "");
        let away = point(width * 0.6, height * 0.55);
        move_to(&mut visual, away);
        settle_title(&workspace, &mut visual);
        let case = format!("{width}x{height}");

        // At rest: compact, actions clipped to nothing, nothing clickable
        // where they would be.
        assert_eq!(reveal(&mut visual), (false, false), "{case}");
        assert_eq!(shown(&mut visual), px(0.), "{case}");
        let rest = visual.debug_bounds("title-bar").unwrap();
        let rest_header = header(&mut visual);
        for action in ACTIONS {
            let hidden = visual.debug_bounds(action).unwrap();
            visual.simulate_click(hidden.center(), gpui::Modifiers::default());
            visual.run_until_parked();
            untouched(
                &mut visual,
                &mut commands,
                &format!("{case} hidden {action}"),
            );
        }

        // A real pointer move onto the capsule heads it open.
        move_to(&mut visual, rest.center());
        assert_eq!(reveal(&mut visual), (true, true), "{case}");
        // Half-way, a click on a visible action still does nothing.
        visual.update(|_, cx| {
            workspace.update(cx, |this, cx| {
                this.freeze_title_reveal(0.5);
                cx.notify();
            })
        });
        visual.run_until_parked();
        let half = shown(&mut visual);
        assert!(half > px(40.), "{case}: {half:?}");
        let changes = visual.debug_bounds("open-changes").unwrap();
        let clip = visual.debug_bounds("title-actions-shown").unwrap();
        assert!(clip.contains(&changes.center()), "{case}: visible mid-way");
        visual.simulate_click(changes.center(), gpui::Modifiers::default());
        visual.run_until_parked();
        untouched(&mut visual, &mut commands, &format!("{case} mid-reveal"));

        settle_title(&workspace, &mut visual);
        let open = visual.debug_bounds("title-bar").unwrap();
        let full = shown(&mut visual);
        assert!(
            (full - half * 2.).abs() < px(1.),
            "{case}: {full:?} {half:?}"
        );
        // Grows about its center, same height; the transcript stays put.
        assert_eq!(open.top(), rest.top(), "{case}");
        assert_eq!(open.size.height, rest.size.height, "{case}");
        assert!(open.size.width > rest.size.width + px(150.), "{case}");
        assert!(
            (open.center().x - rest.center().x).abs() <= px(1.),
            "{case}: {open:?} {rest:?}"
        );
        assert_eq!(header(&mut visual), rest_header, "{case}");
        // Every control fits inside the capsule, inside the window.
        assert!(open.left() >= px(0.) && open.right() <= px(width), "{case}");
        // (Narrow windows drop the model chip; debug bounds outlive it.)
        let wide = width >= 900.;
        let lead = if wide { "title-model" } else { "title-text" };
        for control in ACTIONS.iter().chain([lead].iter()) {
            let bounds = visual.debug_bounds(control).unwrap();
            assert!(
                bounds.left() >= open.left() && bounds.right() <= open.right(),
                "{case}: {control} {bounds:?} in {open:?}"
            );
        }

        // Crossing every control, the gaps between them and the divider
        // keeps it open.
        let first = visual.debug_bounds("open-changes").unwrap();
        let lead = visual.debug_bounds(lead).unwrap();
        let mut stops = vec![
            point(f32::from(lead.right()) + 4., f32::from(open.center().y)),
            point(f32::from(first.left()) - 1., f32::from(open.center().y)),
        ];
        for action in ACTIONS {
            let bounds = visual.debug_bounds(action).unwrap();
            stops.push(bounds.center());
            stops.push(point(
                f32::from(bounds.right()) + 0.5,
                f32::from(bounds.center().y),
            ));
        }
        for at in stops {
            move_to(&mut visual, at);
            assert_eq!(reveal(&mut visual), (true, true), "{case} at {at:?}");
            assert_eq!(shown(&mut visual), full, "{case} at {at:?}");
        }

        // Leaving closes it again, back to exactly the resting capsule.
        move_to(&mut visual, away);
        assert_eq!(reveal(&mut visual), (false, false), "{case}");
        settle_title(&workspace, &mut visual);
        assert_eq!(visual.debug_bounds("title-bar").unwrap(), rest, "{case}");

        // Beside a notice the island holds the actions' room at rest, so
        // revealing them moves neither the island nor its rows.
        show(
            &mut visual,
            "Model change refused: this provider cannot switch models while a turn is running, so nothing changed.",
        );
        settle_title(&workspace, &mut visual);
        assert_eq!(shown(&mut visual), px(0.), "{case} notice at rest");
        let island = visual.debug_bounds("title-island").unwrap();
        let row = visual.debug_bounds("island-notice-status").unwrap();
        let tray = visual.debug_bounds("title-island-notices").unwrap();
        let noticed_header = header(&mut visual);
        assert_eq!(island.origin, open.origin, "{case}");
        assert_eq!(island.size.width, open.size.width, "{case}");
        let bar = visual.debug_bounds("title-bar").unwrap();
        move_to(&mut visual, bar.center());
        assert_eq!(reveal(&mut visual), (true, true), "{case}");
        settle_title(&workspace, &mut visual);
        assert_eq!(shown(&mut visual), full, "{case} notice revealed");
        assert_eq!(
            visual.debug_bounds("title-island").unwrap(),
            island,
            "{case}"
        );
        assert_eq!(
            visual.debug_bounds("island-notice-status").unwrap(),
            row,
            "{case}"
        );
        assert_eq!(
            visual.debug_bounds("title-island-notices").unwrap(),
            tray,
            "{case}"
        );
        assert_eq!(header(&mut visual), noticed_header, "{case}");
        move_to(&mut visual, away);
        settle_title(&workspace, &mut visual);
        assert_eq!(
            visual.debug_bounds("title-island").unwrap(),
            island,
            "{case}"
        );
        assert_eq!(
            visual.debug_bounds("island-notice-status").unwrap(),
            row,
            "{case}"
        );
        show(&mut visual, "");
        settle_title(&workspace, &mut visual);
    }

    // Focus a pointer press leaves on a control does not hold the
    // capsule open (above, the mid-reveal press focused an action, yet
    // leaving closed it). Keyboard focus does, with no pointer, through
    // the focus change's own repaint; Tab reaches the hidden actions and
    // Enter activates them.
    visual.simulate_resize(size(px(1000.), px(700.)));
    show(&mut visual, "");
    settle_title(&workspace, &mut visual);
    let chip = visual.debug_bounds("title-model").unwrap();
    visual.simulate_mouse_down(
        chip.center(),
        gpui::MouseButton::Left,
        gpui::Modifiers::default(),
    );
    visual.simulate_mouse_up(
        point(0., 0.),
        gpui::MouseButton::Left,
        gpui::Modifiers::default(),
    );
    visual.run_until_parked();
    let chip_focus = visual.update(|window, cx| window.focused(cx).unwrap());
    assert_eq!(reveal(&mut visual), (false, false), "pressed, not tabbed");
    visual.simulate_keystrokes("tab");
    visual.run_until_parked();
    assert_ne!(
        visual.update(|window, cx| window.focused(cx).unwrap()),
        chip_focus
    );
    assert_eq!(
        reveal(&mut visual),
        (false, true),
        "Tab into the actions opens it"
    );
    settle_title(&workspace, &mut visual);
    assert!(shown(&mut visual) > px(150.));
    visual.simulate_keystrokes("tab tab");
    visual.run_until_parked();
    assert_eq!(reveal(&mut visual), (false, true), "focus stays inside");
    untouched(&mut visual, &mut commands, "tabbing");
    visual.simulate_keystrokes("enter");
    visual.simulate_event(gpui::KeyUpEvent {
        keystroke: gpui::Keystroke::parse("enter").unwrap(),
    });
    visual.run_until_parked();
    assert!(matches!(effects(&mut commands).as_slice(),
        [Command::Terminal(T::Open { agent, .. })] if agent == "a"));
    // The terminal took focus: with no pointer, the capsule closes.
    assert_eq!(reveal(&mut visual), (false, false));

    // A pointer click works once fully shown (here it hides the panel
    // again; a focused terminal would take ctrl-` itself).
    reveal_title(&workspace, &mut visual);
    click(&mut visual, "open-terminal");
    assert!(matches!(effects(&mut commands).as_slice(),
        [Command::Terminal(T::Hide { agent })] if agent == "a"));
    assert!(!workspace.read_with(&visual, |this, _| this.terminal.open));

    // Tap: a click on the title pins a closed capsule open without
    // taking the composer's focus; a second tap releases it.
    move_to(&mut visual, point(600., 400.));
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.composer
                .update(cx, |input, cx| input.focus(window, cx))
        })
    });
    visual.run_until_parked();
    settle_title(&workspace, &mut visual);
    assert_eq!(reveal(&mut visual), (false, false));
    let title = visual.debug_bounds("title-text").unwrap();
    visual.simulate_click(title.center(), gpui::Modifiers::default());
    visual.run_until_parked();
    assert_eq!(reveal(&mut visual), (false, true), "tap pins it open");
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            assert!(this.composer.read(cx).focus_handle(cx).is_focused(window))
        })
    });
    settle_title(&workspace, &mut visual);
    move_to(&mut visual, point(500., 500.));
    assert_eq!(reveal(&mut visual), (false, true), "pinned through moves");
    let title = visual.debug_bounds("title-text").unwrap();
    visual.simulate_click(title.center(), gpui::Modifiers::default());
    visual.run_until_parked();
    assert_eq!(reveal(&mut visual), (false, false), "second tap releases");
    settle_title(&workspace, &mut visual);
    assert_eq!(shown(&mut visual), px(0.));
    untouched(&mut visual, &mut commands, "taps");
}

/// The view with session `a` reporting `pct` of a 200K context window
/// (`None`: no reading at all).
fn with_context(selected: &str, pct: Option<f64>) -> View {
    let mut next = state(selected);
    if let Some(pct) = pct {
        Arc::make_mut(&mut next.sessions)[0].merge(&serde_json::json!({
            "statusLine": {"contextUsedPct": pct, "contextWindowSize": 200000}
        }));
    }
    next
}

fn show_view(workspace: &Entity<Workspace>, visual: &mut VisualTestContext, view: View) {
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| this.update_view(Arc::new(view), window, cx))
    });
    visual.run_until_parked();
}

/// The context gauge rides the title capsule: a hairline along the
/// title row's bottom edge, filled to the share in use, with the exact
/// figures beside the revealed actions. The composer has no meter, and
/// with no reading nothing shows anywhere.
#[gpui::test]
fn title_capsule_carries_the_context_hairline_once_the_runtime_reports(cx: &mut TestAppContext) {
    let (workspace, mut visual, _, _updates) = fixture(cx);
    show_view(&workspace, &mut visual, with_context("a", None));
    reveal_title(&workspace, &mut visual);
    for selector in [
        "title-context",
        "title-context-detail",
        "context-meter",
        "island-notice-context",
    ] {
        assert!(visual.debug_bounds(selector).is_none(), "{selector}");
    }

    for (pct, tone) in [(30., None), (75., Some(chrome::Tone::Warning))] {
        show_view(&workspace, &mut visual, with_context("a", Some(pct)));
        assert!(visual.debug_bounds("context-meter").is_none(), "{pct}%");
        assert!(workspace.read_with(&visual, |this, _| this.hairline_drawn()));
        let bar = visual.debug_bounds("title-bar").unwrap();
        let line = visual.debug_bounds("title-context").unwrap();
        let fill = visual.debug_bounds("title-context-fill").unwrap();
        assert_eq!(line.size.height, px(2.));
        assert!(
            (line.bottom() - bar.bottom()).abs() <= px(1.5),
            "{line:?} on the bottom edge of {bar:?}"
        );
        assert!(line.left() > bar.left() + px(10.) && line.right() < bar.right() - px(10.));
        let share = fill.size.width / line.size.width;
        assert!((share - pct as f32 / 100.).abs() < 0.02, "{pct}%: {share}");
        // The figures show at the head of the revealed actions.
        let shown = visual.debug_bounds("title-actions-shown").unwrap();
        let detail = visual.debug_bounds("title-context-detail").unwrap();
        assert!(shown.size.width > px(0.) && shown.contains(&detail.center()));
        assert!(bar.contains(&detail.center()));
        // From 70% the capsule holds a quiet glow in the warning tone.
        let glow = workspace.read_with(&visual, |this, _| {
            this.gauge_motion.glow(std::time::Instant::now())
        });
        assert_eq!(glow.map(|(tone, _)| tone), tone, "{pct}%");
        assert!(workspace.read_with(&visual, |this, _| this.island_slots().is_empty()));
    }

    // At rest the figures are clipped away with the actions.
    visual.simulate_mouse_move(
        gpui::point(px(5.), px(600.)),
        None,
        gpui::Modifiers::default(),
    );
    visual.run_until_parked();
    settle_title(&workspace, &mut visual);
    let shown = visual.debug_bounds("title-actions-shown").unwrap();
    assert_eq!(shown.size.width, px(0.));
    assert!(workspace.read_with(&visual, |this, _| this.hairline_drawn()));

    // The reading goes away, and so does the gauge, everywhere.
    show_view(&workspace, &mut visual, with_context("a", None));
    workspace.read_with(&visual, |this, _| {
        assert!(!this.hairline_drawn());
        assert!(this.title_carries_context(), "no meter in the composer");
        assert!(this.gauge_motion.glow(std::time::Instant::now()).is_none());
    });
}

/// A window with no usage yet shows an empty, muted track.
#[gpui::test]
fn waiting_context_shows_an_empty_track(cx: &mut TestAppContext) {
    let (workspace, mut visual, _, _updates) = fixture(cx);
    let mut next = state("a");
    Arc::make_mut(&mut next.sessions)[0].merge(&serde_json::json!({
        "statusLine": {
            "contextWindowSize": 200000,
            "contextUsageState": "waitingForRuntimeUsage"
        }
    }));
    show_view(&workspace, &mut visual, next);
    assert!(visual.debug_bounds("title-context").is_some());
    assert!(visual.debug_bounds("title-context-fill").is_none());
    assert!(visual.debug_bounds("context-meter").is_none());
    reveal_title(&workspace, &mut visual);
    assert!(visual.debug_bounds("title-context-detail").is_some());
}

/// Notices grow the island below the title row; the hairline stays on
/// the title row, just above the seam.
#[gpui::test]
fn context_hairline_stays_on_the_title_row_as_notices_grow_the_island(cx: &mut TestAppContext) {
    let (workspace, mut visual, _, _updates) = fixture(cx);
    let mut next = with_context("a", Some(40.));
    next.notice = "Change queued".into();
    show_view(&workspace, &mut visual, next);
    let bar = visual.debug_bounds("title-bar").unwrap();
    let island = visual.debug_bounds("title-island").unwrap();
    let tray = visual.debug_bounds("title-island-notices").unwrap();
    let line = visual.debug_bounds("title-context").unwrap();
    assert!(
        (line.bottom() - bar.bottom()).abs() <= px(1.),
        "{line:?} {bar:?}"
    );
    assert!(line.bottom() <= tray.top() + px(1.), "above the seam");
    assert!(island.bottom() > line.bottom() + px(10.));
}

/// From 90% the island grows one notice row per threshold band. A
/// dismissed row stays dismissed while the share ticks within its band,
/// and across a chat switch, but a new band is news again.
#[gpui::test]
fn context_notice_shows_once_per_band_and_stays_dismissed_within_it(cx: &mut TestAppContext) {
    let (workspace, mut visual, _, _updates) = fixture(cx);
    let shows = |visual: &mut VisualTestContext| {
        workspace.read_with(visual, |this, _| this.island_slots().contains(&"context"))
    };
    show_view(&workspace, &mut visual, with_context("a", Some(89.)));
    assert!(!shows(&mut visual));
    show_view(&workspace, &mut visual, with_context("a", Some(92.)));
    assert!(shows(&mut visual));
    let row = visual.debug_bounds("island-notice-context").unwrap();
    assert!(
        visual
            .debug_bounds("title-island")
            .unwrap()
            .contains(&row.center())
    );
    click(&mut visual, "dismiss-context-notice");
    assert!(!shows(&mut visual));
    // Token ticks within the band, and a trip to another chat.
    show_view(&workspace, &mut visual, with_context("a", Some(93.)));
    assert!(!shows(&mut visual), "same band");
    show_view(&workspace, &mut visual, with_context("b", Some(93.)));
    assert!(!shows(&mut visual), "b has no reading");
    show_view(&workspace, &mut visual, with_context("a", Some(94.)));
    assert!(!shows(&mut visual), "still dismissed after the switch");
    // The next band shows again.
    show_view(&workspace, &mut visual, with_context("a", Some(96.)));
    assert!(shows(&mut visual), "95% is a new band");
    click(&mut visual, "dismiss-context-notice");
    assert!(!shows(&mut visual));
    // Below 90% the row and its dismissal go; climbing back is news.
    show_view(&workspace, &mut visual, with_context("a", Some(40.)));
    assert!(!shows(&mut visual));
    workspace.read_with(&visual, |this, _| {
        assert!(
            !this
                .extras
                .dismissed_notices
                .iter()
                .any(|(slot, _)| *slot == "context")
        )
    });
    show_view(&workspace, &mut visual, with_context("a", Some(91.)));
    assert!(shows(&mut visual));
}

/// The capsule never hides in compact or narrow layouts, but a short
/// title in a narrow window leaves it too slim for a readable hairline:
/// then the composer keeps its meter. The two never show at once.
#[gpui::test]
fn composer_keeps_the_context_meter_where_the_capsule_is_too_narrow(cx: &mut TestAppContext) {
    let (workspace, mut visual, _, _updates) = fixture(cx);
    show_view(&workspace, &mut visual, with_context("a", Some(92.)));
    for (width, height, in_title) in [
        (720., 480., false),
        (1000., 700., true),
        (720., 700., false),
        (1400., 900., true),
    ] {
        visual.simulate_resize(size(px(width), px(height)));
        visual.run_until_parked();
        let label = format!("{width}x{height}");
        workspace.read_with(&visual, |this, _| {
            // One decision places the meter, so it is never in both.
            assert_eq!(this.title_carries_context(), in_title, "{label}");
            assert_eq!(this.hairline_drawn(), in_title, "{label}");
            // A slim capsule does not escalate either: the meter's
            // color does.
            assert_eq!(
                this.island_slots().contains(&"context"),
                in_title,
                "{label}"
            );
        });
        if in_title {
            let bar = visual.debug_bounds("title-bar").unwrap();
            let line = visual.debug_bounds("title-context").unwrap();
            assert!(bar.contains(&line.center()), "{label}");
        } else {
            // Each fallback size moves the composer, so this is the
            // meter drawn now.
            let meter = visual.debug_bounds("context-meter").unwrap();
            let composer = visual.debug_bounds("chat-composer").unwrap();
            assert!(composer.contains(&meter.center()), "{label}");
        }
    }
}

/// The fill animates on a spring when the share changes, and snaps on
/// a chat switch; at rest it asks for no frames.
#[gpui::test]
fn context_hairline_springs_to_new_readings_and_snaps_across_chats(cx: &mut TestAppContext) {
    let (workspace, mut visual, _, _updates) = fixture(cx);
    show_view(&workspace, &mut visual, with_context("a", Some(30.)));
    assert!(!workspace.read_with(&visual, |this, _| this.gauge_moving()));
    visual.update(|_, cx| workspace.update(cx, |this, _| this.settings.reduce_motion = false));
    show_view(&workspace, &mut visual, with_context("a", Some(60.)));
    assert!(workspace.read_with(&visual, |this, _| this.gauge_moving()));
    let mut other = with_context("b", None);
    Arc::make_mut(&mut other.sessions)[1].merge(&serde_json::json!({
        "statusLine": {"contextUsedPct": 80.0, "contextWindowSize": 200000}
    }));
    show_view(&workspace, &mut visual, other);
    assert!(!workspace.read_with(&visual, |this, _| this.gauge_moving()));
    let line = visual.debug_bounds("title-context").unwrap();
    let fill = visual.debug_bounds("title-context-fill").unwrap();
    assert!((fill.size.width / line.size.width - 0.8).abs() < 0.02);
}

/// The title bar's model chip opens Change model on the session's exact
/// model and effort, lists the efforts the provider reports, and Apply
/// sends only what changed: nothing, the effort, or model then effort.
#[gpui::test]
fn title_model_chip_switches_model_and_effort_without_substitution(cx: &mut TestAppContext) {
    use wks_native::launch::{Catalog, ModelChoice};
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    let codex = |next: &mut View| {
        Arc::make_mut(&mut next.sessions)[0].merge(&serde_json::json!({
            "provider":"codex","cwd":"/project","transport":"stream",
            "settings":{"model":"gpt-5.5","effort":"medium"}}));
    };
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.demo = false;
            let mut next = state("a");
            codex(&mut next);
            this.update_view(Arc::new(next), window, cx);
            this.composer
                .update(cx, |input, cx| input.set_value("draft stays", window, cx));
        })
    });
    visual.run_until_parked();
    let chip = visual
        .debug_bounds("title-model")
        .expect("model chip in the title bar");
    visual.simulate_click(chip.center(), gpui::Modifiers::none());
    visual.run_until_parked();
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            assert_eq!(this.screen, Screen::Model);
            assert_eq!(this.effort, "medium", "opens on the session's effort");
            assert_eq!(this.model_choice, "__custom", "no catalog yet: exact ID");
            assert_eq!(this.model.read(cx).value().as_ref(), "gpt-5.5");
            let mut next = state("a");
            codex(&mut next);
            next.catalog = Catalog {
                key: this.catalog_key(cx),
                loading: false,
                error: None,
                models: vec![
                    ModelChoice {
                        id: "gpt-5.5".into(),
                        label: "GPT-5.5".into(),
                        efforts: vec!["low".into(), "medium".into(), "high".into()],
                        default_effort: Some("medium".into()),
                        ..Default::default()
                    },
                    ModelChoice {
                        id: "gpt-5.4".into(),
                        label: "GPT-5.4".into(),
                        efforts: vec!["low".into(), "high".into()],
                        ..Default::default()
                    },
                    ModelChoice {
                        id: "gpt-5.5-codex".into(),
                        label: "GPT-5.5 Codex".into(),
                        ..Default::default()
                    },
                ],
            };
            this.update_view(Arc::new(next), window, cx);
            assert_eq!(this.model_choice, "gpt-5.5", "the exact ID is selected");
            assert_eq!(
                this.effort_options().0,
                vec!["low", "medium", "high"],
                "efforts come from the provider's report for this model"
            );
            while commands.try_recv().is_ok() {}
            this.apply_model_change(cx);
            assert!(this.extras.notice.contains("already uses"));
        })
    });
    assert!(
        commands.try_recv().is_err(),
        "an unchanged form sends nothing"
    );
    visual.run_until_parked();
    assert!(visual.debug_bounds("launch-effort-picker").is_some());
    visual.update(|_, cx| {
        workspace.update(cx, |this, cx| {
            this.effort = "high".into();
            this.apply_model_change(cx);
        })
    });
    match commands.try_recv() {
        Ok(Command::Act {
            session,
            action: Action::SetEffort(effort),
        }) => assert_eq!((session.as_str(), effort.as_str()), ("a", "high")),
        other => panic!("effort-only change: {:?}", other.map(|_| ())),
    }
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            // Accepted: the baseline moves, so Apply does not resend it.
            this.note_model_receipt(&Receipt {
                number: 1,
                session: "a".into(),
                action: Action::SetEffort("high".into()),
                error: None,
            });
            this.model_choice = "gpt-5.4".into();
            this.reconcile_effort(window, cx);
            assert_eq!(this.effort, "high", "still offered by the new model");
            this.apply_model_change(cx);
        })
    });
    match commands.try_recv() {
        Ok(Command::Act {
            action:
                Action::SetModel {
                    model,
                    effort,
                    context_window,
                },
            ..
        }) => {
            assert_eq!(model, "gpt-5.4");
            assert_eq!(effort, None, "effort unchanged since the receipt");
            assert_eq!(context_window, None);
        }
        other => panic!("model change: {:?}", other.map(|_| ())),
    }
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.effort = "low".into();
            this.apply_model_change(cx);
            assert_eq!(this.composer.read(cx).value().as_ref(), "draft stays");
            this.show_screen(Screen::Conversation, window, cx);
            assert_eq!(this.composer.read(cx).value().as_ref(), "draft stays");
        })
    });
    match commands.try_recv() {
        Ok(Command::Act {
            action: Action::SetModel { model, effort, .. },
            ..
        }) => assert_eq!(
            (model.as_str(), effort.as_deref()),
            ("gpt-5.4", Some("low"))
        ),
        other => panic!("model and effort: {:?}", other.map(|_| ())),
    }
}
