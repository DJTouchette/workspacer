//! The composer and dock: sending, drafts, attachments and layout.
use super::*;

#[gpui::test]
fn keyboard_send_preserves_unicode_and_failed_drafts(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(state("a")), window, cx)
        })
    });
    visual.simulate_keystrokes("ctrl-l");
    visual.simulate_input("résumé 🦀");
    visual.simulate_keystrokes("ctrl-enter");
    let Command::Act {
        session,
        action: Action::Send(text),
    } = commands
        .try_recv()
        .expect("send shortcut reaches controller")
    else {
        panic!("wrong action");
    };
    assert_eq!(session, "a");
    assert_eq!(text, "résumé 🦀");
    let mut failed = state("a");
    failed.receipt = Some(Receipt {
        number: 1,
        session: "a".into(),
        action: Action::Send(text.clone()),
        error: Some("Disconnected; outcome unknown".into()),
    });
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(failed), window, cx);
            assert_eq!(this.composer.read(cx).value().as_ref(), text);
        })
    });
}

/// Settings → Keyboard → Send messages with Enter: Enter sends (queued
/// while the agent works), Shift+Enter adds a line, an IME composition
/// keeps its Enter, a busy composer swallows nothing into the draft, and
/// the default Ctrl/Cmd+Enter mode keeps Enter as a newline.
#[gpui::test]
fn enter_sends_setting_swaps_send_and_newline_in_the_composer_only(cx: &mut TestAppContext) {
    use gpui::EntityInputHandler;
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    let mut working = state("a");
    Arc::make_mut(&mut working.sessions)[0].state = "responding".into();
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.settings.enter_sends = true;
            this.update_view(Arc::new(working), window, cx)
        })
    });
    visual.run_until_parked();
    visual.simulate_keystrokes("ctrl-l");
    visual.simulate_input("line one");
    visual.simulate_keystrokes("shift-enter");
    visual.simulate_input("line two");
    // An open IME composition: Enter belongs to the input method.
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.composer.update(cx, |input, cx| {
                input.replace_and_mark_text_in_range(None, "に", None, window, cx)
            })
        })
    });
    visual.simulate_keystrokes("enter");
    assert!(
        commands.try_recv().is_err(),
        "composition Enter never sends"
    );
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.composer.update(cx, |input, cx| {
                input.replace_text_in_range(None, "", window, cx);
                input.unmark_text(window, cx);
                let value = input.value().to_string();
                let trimmed = value.trim_end_matches('\n').to_owned();
                input.set_value(trimmed, window, cx);
            })
        })
    });
    workspace.read_with(&visual, |this, cx| {
        assert_eq!(
            this.composer.read(cx).value().as_ref(),
            "line one\nline two"
        )
    });
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.composer
                .update(cx, |input, cx| input.focus(window, cx))
        })
    });
    visual.simulate_keystrokes("enter");
    match commands.try_recv() {
        Ok(Command::Act {
            session,
            action: Action::Send(text),
        }) => {
            assert_eq!(session, "a");
            assert_eq!(text, "line one\nline two", "queued while working");
        }
        other => panic!("Enter sends: {:?}", other.map(|_| ())),
    }
    let mut busy = state("a");
    busy.busy = true;
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(busy), window, cx);
            this.composer
                .update(cx, |input, cx| input.set_value("held", window, cx));
            this.composer
                .update(cx, |input, cx| input.focus(window, cx));
        })
    });
    visual.simulate_keystrokes("enter");
    assert!(commands.try_recv().is_err(), "busy: nothing sent");
    workspace.read_with(&visual, |this, cx| {
        assert_eq!(this.composer.read(cx).value().as_ref(), "held")
    });
    // Default mode: Enter is a newline and only Ctrl/Cmd+Enter sends.
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.settings.enter_sends = false;
            this.update_view(Arc::new(state("a")), window, cx);
            this.composer
                .update(cx, |input, cx| input.set_value("first", window, cx));
            this.composer
                .update(cx, |input, cx| input.focus(window, cx));
        })
    });
    visual.run_until_parked();
    visual.simulate_keystrokes("end enter");
    assert!(commands.try_recv().is_err(), "plain Enter does not send");
    workspace.read_with(&visual, |this, cx| {
        assert_eq!(this.composer.read(cx).value().as_ref(), "first\n")
    });
    visual.simulate_keystrokes("ctrl-enter");
    assert!(matches!(
        commands.try_recv(),
        Ok(Command::Act {
            action: Action::Send(_),
            ..
        })
    ));
}

/// A pasted or attached image shows as a thumbnail read back from the hub
/// before sending (PDFs keep their chip); Remove still discards it, and
/// the composer has no separate Paste-image button.
#[gpui::test]
fn draft_images_preview_as_thumbnails_and_stay_removable(cx: &mut TestAppContext) {
    use base64::Engine;
    use wks_native::features::{Request, RequestState};
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    let shot = "/hub/uploads/a/Screenshot.png";
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(state("a")), window, cx);
            this.extras.attachments.insert(
                "a".into(),
                vec![
                    ("Screenshot.png".into(), shot.into()),
                    ("brief.pdf".into(), "/hub/uploads/a/brief.pdf".into()),
                ],
            );
            cx.notify();
        })
    });
    visual.run_until_parked();
    let mut requested = None;
    while let Ok(command) = commands.try_recv() {
        if let Command::Request(Request::Previews { paths }) = command {
            requested = Some(paths);
        }
    }
    assert_eq!(
        requested.as_deref(),
        Some(&[shot.to_owned()][..]),
        "the image (not the PDF) is read back from the hub"
    );
    assert!(
        visual.debug_bounds("draft-attachment-0").is_some(),
        "loading chip"
    );
    assert!(
        visual.debug_bounds("draft-attachment-1").is_some(),
        "PDF chip"
    );
    let mut png = std::io::Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(4, 3)
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    let data = base64::engine::general_purpose::STANDARD.encode(png.into_inner());
    let mut next = state("a");
    next.requests.insert(
        "previews",
        RequestState {
            number: 1,
            request: Request::Previews {
                paths: vec![shot.into()],
            },
            loading: false,
            value: Arc::new(serde_json::json!({
                shot: {"width":4,"height":3,"dataUrl":format!("data:image/png;base64,{data}")}
            })),
            error: None,
        },
    );
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| this.update_view(Arc::new(next), window, cx))
    });
    visual.run_until_parked();
    assert!(
        visual.debug_bounds("draft-thumbnail-0").is_some(),
        "image draft renders as a thumbnail"
    );
    assert!(visual.debug_bounds("draft-attachment-1").is_some());
    workspace.read_with(&visual, |this, cx| {
        assert_eq!(
            this.attachment_text("a", "look"),
            format!("[Image: {shot}]\n[PDF: /hub/uploads/a/brief.pdf]\nlook")
        );
        let _ = cx;
    });
    // Remove sits on the thumbnail's corner.
    let thumb = visual.debug_bounds("draft-thumbnail-0").unwrap();
    visual.simulate_click(
        gpui::point(thumb.right() - px(14.), thumb.top() + px(14.)),
        gpui::Modifiers::none(),
    );
    visual.run_until_parked();
    workspace.read_with(&visual, |this, _| {
        assert_eq!(
            this.extras.attachments["a"],
            vec![(
                "brief.pdf".to_owned(),
                "/hub/uploads/a/brief.pdf".to_owned()
            )]
        );
    });
}

#[gpui::test]
fn late_send_completion_does_not_clear_another_sessions_draft(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(state("a")), window, cx);
            this.composer
                .update(cx, |input, cx| input.set_value("Alpha draft", window, cx));
            this.update_view(Arc::new(state("b")), window, cx);
            this.composer
                .update(cx, |input, cx| input.set_value("Beta draft", window, cx));
            let mut acknowledged = state("b");
            acknowledged.receipt = Some(Receipt {
                number: 1,
                session: "a".into(),
                action: Action::Send("Alpha draft".into()),
                error: None,
            });
            this.update_view(Arc::new(acknowledged), window, cx);
            assert_eq!(this.composer.read(cx).value().as_ref(), "Beta draft");
            assert!(!this.drafts.contains_key("a"));
        })
    });
}

#[gpui::test]
fn conversation_retry_and_first_message_keep_the_draft(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            let mut next = state("a");
            next.loading = true;
            this.update_view(Arc::new(next), window, cx);
            this.composer.update(cx, |input, cx| {
                input.set_value("Retain my draft", window, cx)
            });
        })
    });
    visual.run_until_parked();
    assert!(visual.debug_bounds("state-conversation-loading").is_some());
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            let mut next = state("a");
            next.notice = "Conversation unavailable: request timed out".into();
            this.update_view(Arc::new(next), window, cx);
        })
    });
    visual.run_until_parked();
    assert!(visual.debug_bounds("state-conversation-error").is_some());
    let retry = visual.debug_bounds("retry-empty").unwrap();
    visual.simulate_click(retry.center(), gpui::Modifiers::default());
    assert!(matches!(commands.try_recv().unwrap(), Command::Refresh));
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(state("a")), window, cx)
        })
    });
    visual.run_until_parked();
    let write = visual.debug_bounds("focus-first-message").unwrap();
    visual.simulate_click(write.center(), gpui::Modifiers::default());
    visual.run_until_parked();
    assert!(visual.update(|window, cx| {
        workspace
            .read(cx)
            .composer
            .read(cx)
            .focus_handle(cx)
            .is_focused(window)
    }));
    workspace.read_with(&visual, |this, cx| {
        assert_eq!(this.composer.read(cx).value().as_str(), "Retain my draft")
    });
    assert!(commands.try_recv().is_err());
}

#[gpui::test]
fn chat_and_composer_share_centered_edges_with_compact_approval(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    let mut view = state("a");
    view.sessions = Arc::new(vec![Session {
        id: "a".into(),
        label: "Alpha".into(),
        approval: Some(serde_json::json!({
            "toolName": "Bash", "toolInput": {"command": "cargo test"}
        })),
        ..Default::default()
    }]);
    view.transcript.snapshot(ConversationSnapshot {
        seq: 1,
        first_seq: 1,
        items: vec![Item {
            kind: "assistant_text".into(),
            text: "Ready for review".into(),
            ..Default::default()
        }],
    });
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| this.update_view(Arc::new(view), window, cx))
    });
    for (width, height) in [(1600., 900.), (1000., 700.), (720., 480.)] {
        visual.simulate_resize(size(px(width), px(height)));
        visual.run_until_parked();
        let chat = visual.debug_bounds("chat-content-column").unwrap();
        let composer = visual.debug_bounds("chat-composer").unwrap();
        assert!(
            (f32::from(chat.left() - composer.left())).abs() < 1.,
            "{width}x{height}: chat={chat:?}, composer={composer:?}"
        );
        assert!((f32::from(chat.right() - composer.right())).abs() < 1.);
        workspace.read_with(&visual, |this, _| {
            assert!(
                (f32::from(chat.center().x - this.list.viewport_bounds().center().x)).abs() < 1.
            );
            assert!(this.composer_dock_bounds.top() - this.header_bounds.bottom() > px(140.));
        });
    }
}

#[gpui::test]
fn floating_composer_keeps_the_last_message_clear_without_shortening_the_viewport(
    cx: &mut TestAppContext,
) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    let mut view = state("a");
    view.transcript.snapshot(ConversationSnapshot {
        seq: 30,
        first_seq: 1,
        items: (0..30)
            .map(|i| Item {
                kind: "assistant_text".into(),
                text: format!("Message {i}\n\nA paragraph in the conversation."),
                ..Default::default()
            })
            .collect(),
    });
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| this.update_view(Arc::new(view), window, cx))
    });
    visual.run_until_parked();
    let last_row = visual.debug_bounds("last-transcript-row").unwrap();
    let initial_height = workspace.read_with(&visual, |this, _| {
        let dock = this.composer_dock_bounds;
        assert!(dock.size.height > px(60.));
        assert!(this.header_bounds.size.height > px(30.));
        assert!(this.list.viewport_bounds().top() < this.header_bounds.bottom());
        assert!(this.list.viewport_bounds().bottom() > dock.top() + px(60.));
        assert!(last_row.bottom() <= dock.top());
        dock.size.height
    });
    visual.simulate_keystrokes("ctrl-l");
    visual.simulate_input("First line\nSecond line\nThird line\nFourth line");
    visual.run_until_parked();
    let last_row = visual.debug_bounds("last-transcript-row").unwrap();
    workspace.read_with(&visual, |this, _| {
        assert!(this.composer_dock_bounds.size.height > initial_height);
        assert!(last_row.bottom() <= this.composer_dock_bounds.top());
    });
    visual.simulate_resize(size(px(720.), px(480.)));
    visual.run_until_parked();
    let last_row = visual.debug_bounds("last-transcript-row").unwrap();
    workspace.read_with(&visual, |this, _| {
        assert!(this.composer_dock_bounds.top() > px(180.));
        assert!(last_row.bottom() <= this.composer_dock_bounds.top());
    });
}

/// Bug #26: a wheel over the conversation dock's gutters (its padding,
/// the gaps between cards) must never scroll the composer's top edge
/// under a clip, at any size, interface size or card/draft growth.
/// Scroll state survives renders, so each variant inherits the previous
/// one's wheels, as an intermittent real session would.
#[gpui::test]
fn conversation_dock_wheel_never_clips_the_composer_top(cx: &mut TestAppContext) {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            set_zoom(1.);
        }
    }
    let _reset = Reset;
    let _caption = CaptionPreview::new();
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    let view = |approval: bool, questions: bool| {
        let mut view = if questions {
            question_state()
        } else {
            state("a")
        };
        if approval {
            Arc::make_mut(&mut view.sessions)[0].approval = Some(serde_json::json!({
                "toolName": "Bash", "toolInput": {"command": "cargo test"}
            }));
        }
        view.transcript.snapshot(ConversationSnapshot {
            seq: 20,
            first_seq: 1,
            items: (0..20)
                .map(|i| Item {
                    kind: "assistant_text".into(),
                    text: format!("Message {i}\n\nA paragraph in the conversation."),
                    ..Default::default()
                })
                .collect(),
        });
        Arc::new(view)
    };
    let wheels = |dock: gpui::Bounds<gpui::Pixels>, composer: gpui::Bounds<gpui::Pixels>| {
        [
            gpui::point(dock.left() + px(2.), composer.center().y),
            gpui::point(dock.right() - px(2.), composer.top() + px(4.)),
            gpui::point(dock.center().x, dock.top() + px(2.)),
            gpui::point(dock.center().x, dock.bottom() - px(2.)),
        ]
    };
    let mut checked = 0;
    for zoom_steps in [0, 3] {
        for _ in 0..zoom_steps {
            visual.simulate_keystrokes("ctrl-=");
        }
        for (width, height) in [(1000., 700.), (720., 480.), (1600., 900.), (860., 560.)] {
            for (approval, questions, files, draft) in [
                (false, false, false, ""),
                (true, false, false, ""),
                (false, true, false, ""),
                (false, false, true, "one\ntwo\nthree\nfour"),
            ] {
                visual.simulate_resize(size(px(width), px(height)));
                visual.update(|window, cx| {
                    workspace.update(cx, |this, cx| {
                        this.update_view(view(approval, questions), window, cx);
                        let attached = if files {
                            vec![
                                ("screen.png".into(), "/remote/screen.png".into()),
                                ("spec.pdf".into(), "/remote/spec.pdf".into()),
                            ]
                        } else {
                            vec![]
                        };
                        this.extras.attachments.insert("a".into(), attached);
                        this.composer
                            .update(cx, |input, cx| input.set_value(draft, window, cx));
                    })
                });
                visual.run_until_parked();
                let case = format!(
                    "{width}x{height} zoom+{zoom_steps} approval={approval} questions={questions} files={files}"
                );
                let dock = visual.debug_bounds("conversation-dock").unwrap();
                let cards = visual.debug_bounds("conversation-dock-cards").unwrap();
                let composer = visual.debug_bounds("chat-composer").unwrap();
                let send = visual.debug_bounds("composer-send").unwrap();
                // The composer sits below the cards' clip, never under it,
                // with Send inside the dock and the window.
                // (Half a pixel: layout rounds fractional card heights.)
                assert!(
                    composer.top() >= cards.bottom() - px(0.5),
                    "{case}: {composer:?} under {cards:?}"
                );
                assert!(
                    composer.top() >= dock.top() + px(4.),
                    "{case}: {composer:?} in {dock:?}"
                );
                assert!(
                    send.bottom() <= dock.bottom() && dock.bottom() <= px(height),
                    "{case}: {send:?} in {dock:?}"
                );
                checked += 1;
                for position in wheels(dock, composer) {
                    for delta in [
                        gpui::ScrollDelta::Lines(gpui::point(0., -3.)),
                        gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-60.))),
                    ] {
                        visual.simulate_event(gpui::ScrollWheelEvent {
                            position,
                            delta,
                            ..Default::default()
                        });
                        visual.run_until_parked();
                        assert_eq!(
                            visual.debug_bounds("chat-composer").unwrap(),
                            composer,
                            "{case}: wheel {delta:?} at {position:?} moved the composer"
                        );
                        assert_eq!(
                            visual.debug_bounds("conversation-dock").unwrap(),
                            dock,
                            "{case}"
                        );
                    }
                }
                // Cards that do not fit scroll inside their own region:
                // a wheel over them reaches the approval actions.
                if approval {
                    // (Debug bounds outlive their element; read it only here.)
                    let card = visual.debug_bounds("approval-card").unwrap();
                    visual.simulate_event(gpui::ScrollWheelEvent {
                        position: card.center(),
                        delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-2000.))),
                        ..Default::default()
                    });
                    visual.run_until_parked();
                    let card = visual.debug_bounds("approval-card").unwrap();
                    assert!(
                        card.bottom() <= cards.bottom() + px(0.5),
                        "{case}: {card:?} in {cards:?}"
                    );
                    assert_eq!(
                        visual.debug_bounds("chat-composer").unwrap(),
                        composer,
                        "{case}"
                    );
                }
            }
        }
        visual.simulate_keystrokes("ctrl-0");
    }
    assert_eq!(checked, 32);
}

#[gpui::test]
fn short_physical_windows_bound_long_notices_and_many_attachments(cx: &mut TestAppContext) {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            set_zoom(1.);
        }
    }
    let _reset = Reset;
    let _caption = CaptionPreview::new();
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    for scale in [1., 1.25, 1.5, 2.] {
        set_zoom(scale);
        // Raw GPUI pixels: zoom must not enlarge the physical test window.
        visual.simulate_resize(size(gpui::px(720.), gpui::px(480.)));
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.apply_typography(cx);
                let mut next = question_state();
                Arc::make_mut(&mut next.sessions)[0].merge(&serde_json::json!({
                    "statusLine": {"contextUsedPct": 42.0, "contextWindowSize": 200000}
                }));
                next.notice = format!(
                    "Model change refused: {}",
                    "The host returned a detailed explanation. ".repeat(40)
                );
                this.update_view(Arc::new(next), window, cx);
                this.extras.attachments.insert(
                    "a".into(),
                    (0..12)
                        .map(|i| {
                            (
                                format!("document-{i}.pdf"),
                                format!("/remote/document-{i}.pdf"),
                            )
                        })
                        .collect(),
                );
                this.composer.update(cx, |input, cx| {
                    input.set_value("one\ntwo\nthree\nfour", window, cx)
                });
            })
        });
        visual.run_until_parked();
        let header = workspace.read_with(&visual, |this, _| this.header_bounds);
        let dock = visual.debug_bounds("conversation-dock").unwrap();
        let send = visual.debug_bounds("composer-send").unwrap();
        assert!(
            header.bottom() + gpui::px(if scale < 2. { 24. } else { 0. }) <= dock.top(),
            "scale {scale}: header {header:?}, dock {dock:?}"
        );
        assert!(
            send.bottom() <= gpui::px(480.)
                && send.top() > header.bottom()
                && send.right() <= gpui::px(720.),
            "scale {scale}: send {send:?}, header {header:?}"
        );
        let before = visual.debug_bounds("chat-composer").unwrap();
        assert!(
            before.top() >= header.bottom() && before.bottom() <= gpui::px(480.),
            "scale {scale}: composer {before:?}"
        );
        let tray = visual.debug_bounds("title-island-notices").unwrap();
        visual.simulate_event(gpui::ScrollWheelEvent {
            position: tray.center(),
            delta: gpui::ScrollDelta::Pixels(gpui::point(gpui::px(0.), gpui::px(-100_000.))),
            ..Default::default()
        });
        visual.run_until_parked();
        assert_eq!(visual.debug_bounds("chat-composer").unwrap(), before);
        let message = visual.debug_bounds("island-notice-status").unwrap();
        assert!(
            message.bottom() <= tray.bottom() + gpui::px(1.),
            "full notice can be scrolled to its end: {message:?} {tray:?}"
        );
    }
}

#[gpui::test]
fn response_actions_preserve_drafts_and_validate_worker_ownership(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(state("a")), window, cx);
            this.composer
                .update(cx, |input, cx| input.set_value("My draft", window, cx));
            this.card_action(
                "a",
                &wks_native::transcript::CardAction::FillComposer {
                    label: "Continue".into(),
                    text: "Proposed follow-up".into(),
                },
                window,
                cx,
            );
            assert_eq!(
                this.composer.read(cx).value().as_ref(),
                "My draft\nProposed follow-up"
            );
            this.card_action(
                "a",
                &wks_native::transcript::CardAction::OpenWorker {
                    label: "Unrelated".into(),
                    session_id: "b".into(),
                },
                window,
                cx,
            );
            assert!(this.local_notice.contains("not a live worker"));
        });
    });
    assert!(
        commands.try_recv().is_err(),
        "prefills do not send and unrelated workers do not open"
    );
}

#[gpui::test]
fn an_explicit_target_never_sends_to_the_default_session(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.open_session(Some("b".into()));
            this.composer.update(cx, |input, cx| {
                input.set_value("targeted message", window, cx)
            });
            this.update_view(Arc::new(state("a")), window, cx);
            this.send(&SendMessage, window, cx);
            assert!(this.view.selected.is_none());
        })
    });
    assert!(matches!(commands.try_recv().unwrap(),Command::Select(id) if id == "b"));
    assert!(commands.try_recv().is_err());
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(state("b")), window, cx);
            this.composer.update(cx, |input, cx| {
                input.set_value("still targeted", window, cx)
            });
            let mut disappeared = state("a");
            disappeared.sessions = Arc::new(vec![disappeared.sessions[0].clone()]);
            this.update_view(Arc::new(disappeared), window, cx);
            this.send(&SendMessage, window, cx);
            assert!(!this.view.connected);
            assert_eq!(this.view.selected.as_deref(), Some("b"));
        })
    });
    assert!(commands.try_recv().is_err());
}

#[gpui::test]
fn secondary_views_preserve_drafts_and_only_request_reads(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(state("a")), window, cx);
            this.composer
                .update(cx, |input, cx| input.set_value("unsent work", window, cx));
            this.open_feature(Screen::Changes, window, cx);
            assert_eq!(this.composer.read(cx).value().as_ref(), "unsent work");
            this.open_feature(Screen::Session, window, cx);
            this.show_screen(Screen::Conversation, window, cx);
            assert_eq!(this.composer.read(cx).value().as_ref(), "unsent work");
        })
    });
    assert!(matches!(
        commands.try_recv().unwrap(),
        Command::Request(wks_native::features::Request::Changes { .. })
    ));
    assert!(commands.try_recv().is_err());
}

#[gpui::test]
fn attachment_receipts_preserve_new_drafts_and_other_sessions(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(state("a")), window, cx);
            this.extras.attachments.insert(
                "a".into(),
                vec![("screen.png".into(), "/remote/screen.png".into())],
            );
            this.composer
                .update(cx, |input, cx| input.set_value("inspect this", window, cx));
            this.send(&SendMessage, window, cx);
            this.update_view(Arc::new(state("b")), window, cx);
            this.composer
                .update(cx, |input, cx| input.set_value("other draft", window, cx));
            let mut next = state("b");
            next.receipt = Some(Receipt {
                number: 1,
                session: "a".into(),
                action: Action::Send("[Image: /remote/screen.png]\ninspect this".into()),
                error: None,
            });
            this.update_view(Arc::new(next), window, cx);
            assert_eq!(this.composer.read(cx).value().as_ref(), "other draft");
            assert!(!this.drafts.contains_key("a"));
            assert!(this.extras.attachments["a"].is_empty());
        })
    });
    assert!(
        matches!(commands.try_recv().unwrap(), Command::Act { session, action:Action::Send(text) } if session == "a" && text.contains("[Image: /remote/screen.png]"))
    );
}

#[gpui::test]
fn failed_attachment_send_keeps_original_draft_and_file(cx: &mut TestAppContext) {
    let (workspace, mut visual, _, _) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(state("a")), window, cx);
            this.extras.attachments.insert(
                "a".into(),
                vec![("screen.png".into(), "/remote/screen.png".into())],
            );
            this.composer
                .update(cx, |input, cx| input.set_value("inspect this", window, cx));
            this.send(&SendMessage, window, cx);
            let mut next = state("a");
            next.receipt = Some(Receipt {
                number: 1,
                session: "a".into(),
                action: Action::Send("[Image: /remote/screen.png]\ninspect this".into()),
                error: Some("Disconnected; outcome unknown".into()),
            });
            this.update_view(Arc::new(next), window, cx);
            assert_eq!(this.composer.read(cx).value().as_ref(), "inspect this");
            assert_eq!(this.extras.attachments["a"].len(), 1);
        })
    });
}

#[gpui::test]
fn normal_text_paste_still_reaches_the_composer(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(state("a")), window, cx);
            cx.write_to_clipboard(ClipboardItem::new_string("ordinary paste".into()));
            this.composer
                .update(cx, |input, cx| input.focus(window, cx));
        })
    });
    visual.simulate_keystrokes(if cfg!(target_os = "macos") {
        "cmd-v"
    } else {
        "ctrl-v"
    });
    workspace.read_with(&visual, |this, cx| {
        assert_eq!(this.composer.read(cx).value().as_ref(), "ordinary paste")
    });
    assert!(commands.try_recv().is_err());
}
