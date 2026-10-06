//! Chat rendering: Markdown, links, selection, tool cards, timestamps and wakes.
use super::*;

/// Clicks `selector` 20px in, at `y` from its top (centered when `None`),
/// until `done`; a selector a debounced parse has not drawn yet is waited for.
fn click_until(
    visual: &mut VisualTestContext,
    y: Option<gpui::Pixels>,
    selector: &'static str,
    mut done: impl FnMut(&mut VisualTestContext) -> bool,
) {
    for _ in 0..100 {
        let Some(bounds) = visual.debug_bounds(selector) else {
            visual.run_until_parked();
            std::thread::sleep(std::time::Duration::from_millis(20));
            continue;
        };
        let y = y.unwrap_or(bounds.size.height / 2.);
        visual.simulate_click(
            bounds.origin + gpui::point(px(20.), y),
            gpui::Modifiers::default(),
        );
        visual.run_until_parked();
        if done(visual) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    panic!("{selector} never responded to a click");
}

#[gpui::test]
fn chat_markdown_follows_the_appearance_syntax_theme(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    for appearance in Appearance::ALL {
        visual.update(|window, cx| {
            configure_theme(appearance, Some(window), cx);
            assert_eq!(
                *gpui_component::Theme::global(cx).highlight_theme,
                *syntax::highlight_theme(appearance)
            );
            workspace.update(cx, |this, cx| {
                this.appearance = appearance;
                let mut view = state("a");
                view.transcript.snapshot(ConversationSnapshot {
                    seq: 1,
                    first_seq: 1,
                    items: vec![Item {
                        kind: "assistant_text".into(),
                        text: "## Plan\n\n**Bold** and *italic* with `src/main.rs`.\n\n\
                                   - one\n1. two\n\n---\n\n```rust\nfn main() {}\n```\n\n```\nplain\n```"
                            .into(),
                        ..Default::default()
                    }],
                });
                this.update_view(Arc::new(view), window, cx);
            })
        });
        visual.run_until_parked();
        assert!(visual.debug_bounds("markdown-inline-live:a:0-0").is_some());
    }
    assert_ne!(
        syntax::highlight_theme(Appearance::Dark),
        syntax::highlight_theme(Appearance::Nord)
    );
}

/// Selection runs from where the drag started to the pointer in reading
/// order. The vendored TextView once selected the rectangle between the
/// two points, so a backward drag up and to the right took in the line
/// above from the start column, text the pointer never reached.
#[gpui::test]
fn chat_selection_follows_the_drag_in_reading_order(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            let mut view = state("a");
            view.transcript.snapshot(ConversationSnapshot {
                seq: 1,
                first_seq: 1,
                items: vec![Item {
                    kind: "assistant_text".into(),
                    text: "alpha beta gamma delta\n\nepsilon zeta eta theta".into(),
                    ..Default::default()
                }],
            });
            this.update_view(Arc::new(view), window, cx);
        })
    });
    visual.run_until_parked();
    let bounds = visual
        .debug_bounds("markdown-inline-live:a:0-0")
        .expect("assistant markdown");
    // The test text system is monospaced: every glyph is 0.6em wide.
    let font = workspace.read_with(&visual, |this, _| this.settings.text_size as f32);
    let (char_width, line_height) = (font * 0.6, font * 1.6);
    // Just right of the boundary before character `col`.
    let first =
        |col: f32| bounds.origin + gpui::point(px((col + 0.1) * char_width), px(line_height / 2.));
    let second = |col: f32| {
        gpui::point(
            bounds.left() + px((col + 0.1) * char_width),
            bounds.bottom() - px(line_height / 2.),
        )
    };
    let mut copy = |from: gpui::Point<gpui::Pixels>, to: gpui::Point<gpui::Pixels>| {
        let away = bounds.bottom_right() + gpui::point(px(0.), px(40.));
        visual.simulate_click(away, gpui::Modifiers::default());
        visual.run_until_parked();
        visual.simulate_mouse_down(from, gpui::MouseButton::Left, gpui::Modifiers::default());
        visual.simulate_mouse_move(
            to,
            Some(gpui::MouseButton::Left),
            gpui::Modifiers::default(),
        );
        visual.run_until_parked();
        visual.simulate_mouse_up(to, gpui::MouseButton::Left, gpui::Modifiers::default());
        visual.run_until_parked();
        visual.write_to_clipboard(gpui::ClipboardItem::new_string(String::new()));
        visual.simulate_keystrokes("secondary-c");
        visual.run_until_parked();
        visual
            .read_from_clipboard()
            .and_then(|item| item.text())
            .unwrap_or_default()
    };
    // Backward within a line, and its forward twin.
    assert_eq!(copy(first(16.), first(6.)), "beta gamma");
    assert_eq!(copy(first(6.), first(16.)), "beta gamma");
    // Backward up and to the right across paragraphs: from "zeta" back
    // to "gamma". Neither "beta" above nor "zeta" below is reached.
    for copied in [copy(second(8.), first(11.)), copy(first(11.), second(8.))] {
        assert!(
            copied.starts_with("gamma delta") && copied.ends_with("epsilon"),
            "{copied:?}"
        );
        assert!(
            !copied.contains("ta gamma") && !copied.contains("zet"),
            "{copied:?}"
        );
    }
    // Backward up and to the left: "beta" through "epsilon zeta".
    let copied = copy(second(12.), first(6.));
    assert!(
        copied.starts_with("beta gamma delta") && copied.ends_with("epsilon zeta"),
        "{copied:?}"
    );
}

#[gpui::test]
fn markdown_file_link_requests_preview_and_shows_the_result(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            let mut view = state("a");
            Arc::make_mut(&mut view.sessions)[0].cwd = "/fixture/repo".into();
            view.transcript.snapshot(ConversationSnapshot {
                seq: 1,
                first_seq: 1,
                items: vec![Item {
                    kind: "assistant_text".into(),
                    text: "[Open README](docs/README.md:12)".into(),
                    ..Default::default()
                }],
            });
            this.update_view(Arc::new(view), window, cx);
        })
    });
    visual.run_until_parked();
    let bounds = visual
        .debug_bounds("markdown-inline-live:a:0-0")
        .expect("native inline file link");
    let start = bounds.origin + gpui::point(px(2.), bounds.size.height / 2.);
    let end = bounds.origin + gpui::point(px(90.), bounds.size.height / 2.);
    visual.simulate_mouse_down(start, gpui::MouseButton::Left, gpui::Modifiers::default());
    visual.simulate_mouse_move(
        end,
        Some(gpui::MouseButton::Left),
        gpui::Modifiers::default(),
    );
    visual.run_until_parked();
    visual.simulate_mouse_up(end, gpui::MouseButton::Left, gpui::Modifiers::default());
    visual.run_until_parked();
    assert!(
        commands.try_recv().is_err(),
        "selecting a link must not open it"
    );
    visual.simulate_click(
        bounds.origin + gpui::point(px(25.), bounds.size.height / 2.),
        gpui::Modifiers::default(),
    );
    visual.run_until_parked();
    let request = commands.try_recv().expect("file click issues a request");
    let Command::Request(wks_native::features::Request::FilePreview { session, target }) = request
    else {
        panic!("file click must request a native preview");
    };
    assert_eq!(session, "a");
    assert_eq!(target.path, "/fixture/repo/docs/README.md");
    assert_eq!(target.line, Some(12));
    let preview = |loading: bool, error: Option<&str>, number: u64| {
        let (session, target, workspace) = (session.clone(), target.clone(), workspace.clone());
        let error = error.map(str::to_owned);
        move |window: &mut Window, cx: &mut App| {
            workspace.update(cx, |this, cx| {
                let mut view = (*this.view).clone();
                view.requests.insert("file-preview", wks_native::features::RequestState {
                    request: wks_native::features::Request::FilePreview { session, target },
                    number, loading, error,
                    value: Arc::new(serde_json::json!({"contents":(1..=40).map(|i| format!("line {i}\n")).collect::<String>(),"size":290})),
                });
                this.update_view(Arc::new(view), window, cx);
            })
        }
    };
    visual.update(preview(true, None, 1));
    visual.run_until_parked();
    assert!(
        visual.debug_bounds("file-viewer").is_some(),
        "loading opens the sheet"
    );
    assert!(visual.debug_bounds("file-viewer-text").is_none());
    visual.update(preview(false, None, 1));
    visual.run_until_parked();
    assert!(visual.debug_bounds("file-viewer-title").is_some());
    assert!(visual.debug_bounds("file-viewer-text").is_some());
    let (text, cursor) = workspace.read_with(&visual, |this, cx| {
        let pane = this.file_viewer().unwrap().read(cx);
        assert_eq!(
            pane.mode(),
            file_viewer::Mode::Source,
            "a line anchor opens a Markdown file's source at that line"
        );
        let editor = pane.editor().unwrap().read(cx);
        (editor.value().to_string(), editor.cursor_position())
    });
    assert!(text.starts_with("line 1\n"));
    assert_eq!(cursor.line, 11, "line anchors place the cursor on line 12");
    // Editable: typing changes the editor only. Nothing reaches the
    // hub until Save.
    visual.simulate_input("typed");
    visual.run_until_parked();
    let pane = pane_of(&workspace, &visual);
    pane.read_with(&visual, |pane, cx| {
        let value = pane.editor().unwrap().read(cx).value().to_string();
        assert_ne!(value, text);
        assert!(value.contains("typed"));
        assert!(pane.dirty());
    });
    assert!(commands.try_recv().is_err(), "typing sent nothing");
    // Restoring the loaded text leaves nothing unsaved.
    visual.update(|window, cx| {
        let editor = pane.read(cx).editor().unwrap().clone();
        editor.update(cx, |e, cx| e.set_value(text.clone(), window, cx));
    });
    visual.run_until_parked();
    assert!(!pane.read_with(&visual, |p, _| p.dirty()));
    // Wheel over the sheet never scrolls the conversation underneath.
    let top = |this: &Workspace| {
        let top = this.list.logical_scroll_top();
        (top.item_ix, top.offset_in_item)
    };
    let before = workspace.read_with(&visual, |this, _| top(this));
    let sheet = visual.debug_bounds("file-viewer-text").unwrap();
    visual.simulate_event(gpui::ScrollWheelEvent {
        position: sheet.center(),
        delta: gpui::ScrollDelta::Lines(gpui::point(0., -3.)),
        ..Default::default()
    });
    visual.run_until_parked();
    assert_eq!(workspace.read_with(&visual, |this, _| top(this)), before);
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();
    workspace.read_with(&visual, |this, _| {
        assert!(this.file_viewer().is_none(), "Esc closes the viewer")
    });
    // Later view updates never rebuild a dismissed viewer or take focus.
    visual.update(preview(false, None, 1));
    visual.run_until_parked();
    workspace.read_with(&visual, |this, _| {
        assert!(this.file_viewer().is_none());
    });
    // A failed read stays visible as a message inside the sheet.
    visual.update(preview(
        false,
        Some("No file at this path on the session's machine."),
        2,
    ));
    visual.run_until_parked();
    assert!(visual.debug_bounds("file-viewer-error").is_some());
    settle(&mut visual);
    let close = visual.debug_bounds("file-viewer-close").unwrap();
    visual.simulate_click(close.center(), gpui::Modifiers::default());
    visual.run_until_parked();
    workspace.read_with(&visual, |this, _| {
        assert!(
            this.file_viewer().is_none(),
            "close button closes the viewer"
        )
    });
}

#[gpui::test]
fn chat_links_route_web_refused_and_image_targets(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            let mut view = state("a");
            Arc::make_mut(&mut view.sessions)[0].cwd = "/fixture/repo".into();
            view.transcript.snapshot(ConversationSnapshot {
                seq: 3,
                first_seq: 1,
                items: [
                    "[Docs site](https://example.com/docs)",
                    "[Mail the team](mailto:team@example.com)",
                    "![Screenshot](out/shot.png)",
                ]
                .into_iter()
                .map(|text| Item {
                    kind: "assistant_text".into(),
                    text: text.into(),
                    ..Default::default()
                })
                .collect(),
            });
            this.update_view(Arc::new(view), window, cx);
        })
    });
    visual.run_until_parked();
    // TextView parses Markdown behind a real-time debounce the test
    // executor does not drive, so retry each click within a bounded wait.
    let platform = cx.clone();
    click_until(&mut visual, None, "markdown-inline-live:a:0-0", |_| {
        platform.opened_url().is_some()
    });
    assert_eq!(cx.opened_url().as_deref(), Some("https://example.com/docs"));
    assert!(commands.try_recv().is_err(), "web links never read files");
    let notice = workspace.clone();
    click_until(&mut visual, None, "markdown-inline-live:a:1-0", |visual| {
        notice.read_with(visual, |this, _| !this.extras.notice.is_empty())
    });
    workspace.read_with(&visual, |this, _| {
        assert!(
            this.extras.notice.contains("mailto"),
            "refusals are visible"
        )
    });
    assert_eq!(cx.opened_url().as_deref(), Some("https://example.com/docs"));
    assert!(commands.try_recv().is_err());
    // The image is a label, not a client-side load, and opens the viewer.
    let mut request = None;
    click_until(&mut visual, None, "markdown-inline-live:a:2-0", |_| {
        request = commands.try_recv().ok();
        request.is_some()
    });
    let Some(Command::Request(wks_native::features::Request::FilePreview { session, target })) =
        request
    else {
        panic!("image click must request a native preview");
    };
    assert_eq!(target.path, "/fixture/repo/out/shot.png");
    assert_eq!(target.kind, wks_native::links::FileKind::Image);
    let png = {
        use base64::Engine;
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(64, 32)
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        base64::engine::general_purpose::STANDARD.encode(bytes.into_inner())
    };
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            let mut view = (*this.view).clone();
            view.requests.insert(
                "file-preview",
                wks_native::features::RequestState {
                    request: wks_native::features::Request::FilePreview { session, target },
                    number: 1,
                    loading: false,
                    error: None,
                    value: Arc::new(
                        serde_json::json!({"png":png,"width":64,"height":32,"size":120}),
                    ),
                },
            );
            this.update_view(Arc::new(view), window, cx);
        })
    });
    visual.run_until_parked();
    assert!(visual.debug_bounds("file-viewer-image").is_some());
    // Backdrop click closes it. Under an app-drawn caption (Windows) the
    // backdrop starts below the caption strip, so aim inside its bounds.
    let backdrop = visual.debug_bounds("file-viewer-backdrop").unwrap();
    visual.simulate_click(
        backdrop.origin + gpui::point(px(4.), px(4.)),
        gpui::Modifiers::default(),
    );
    visual.run_until_parked();
    workspace.read_with(&visual, |this, _| assert!(this.file_viewer().is_none()));
}

// Desktop parity (HtmlResponseCard footer): trusted card actions are one
// compact row of outlined buttons that wraps inside the card when narrow,
// not a full-width stack, and still do what they say.
#[gpui::test]
fn html_card_actions_are_a_compact_wrapping_row(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    visual.simulate_resize(size(px(1400.), px(1000.)));
    let raw = serde_json::json!({
        "v": 1, "title": "Review complete", "fallback": "Ready for review",
        "bodyHtml": "<p>Ready</p>",
        "actions": [
            {"kind": "fill_composer", "label": "Continue", "text": "Carry on"},
            {"kind": "view_diff", "label": "main.rs", "path": "src/main.rs"},
            {"kind": "open_worker", "label": "Worker 1", "sessionId": "b"},
        ],
    });
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            let mut view = state("a");
            view.transcript.snapshot(ConversationSnapshot {
                seq: 1,
                first_seq: 1,
                items: vec![Item {
                    kind: "assistant_text".into(),
                    text: format!("```wks-html-card\n{raw}\n```"),
                    ..Default::default()
                }],
            });
            this.update_view(Arc::new(view), window, cx);
        })
    });
    visual.run_until_parked();
    let actions = |visual: &mut VisualTestContext| {
        (0..3)
            .map(|i| {
                visual
                    .debug_bounds(Box::leak(
                        format!("card-action-live:a:0-0-{i}").into_boxed_str(),
                    ))
                    .unwrap_or_else(|| panic!("card action {i} not rendered"))
            })
            .collect::<Vec<_>>()
    };
    let wide = actions(&mut visual);
    for (i, bounds) in wide.iter().enumerate() {
        assert_eq!(
            bounds.top(),
            wide[0].top(),
            "action {i} left the row: {wide:?}"
        );
        assert!(
            bounds.size.height < px(40.),
            "action {i} is not compact: {bounds:?}"
        );
        assert!(
            bounds.size.width < px(300.),
            "action {i} spans the card: {bounds:?}"
        );
    }
    assert!(wide[0].right() < wide[1].left() && wide[1].right() < wide[2].left());

    visual.simulate_resize(size(px(720.), px(1000.)));
    visual.run_until_parked();
    let card = visual.debug_bounds("html-card-live:a:0-0").unwrap();
    for (i, bounds) in actions(&mut visual).iter().enumerate() {
        assert!(
            bounds.right() <= px(720.),
            "action {i} spilled past the window: {bounds:?}"
        );
        assert!(
            bounds.left() >= card.left(),
            "action {i} left its card: {bounds:?}"
        );
    }

    let prefill = actions(&mut visual)[0].center();
    visual.simulate_click(prefill, gpui::Modifiers::default());
    visual.run_until_parked();
    workspace.read_with(&visual, |this, cx| {
        assert_eq!(this.composer.read(cx).value().as_ref(), "Carry on");
    });
}

#[gpui::test]
fn html_card_links_route_from_raw_fence_through_sanitizer(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    // Tall enough that every card is laid out at once.
    visual.simulate_resize(size(px(1000.), px(1600.)));
    let card = |body: &str| {
        let raw = serde_json::json!({"v":1,"title":"Card","fallback":"Fallback","bodyHtml":body});
        format!("```wks-html-card\n{raw}\n```")
    };
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            let mut view = state("a");
            Arc::make_mut(&mut view.sessions)[0].cwd = "/fixture/repo".into();
            view.transcript.snapshot(ConversationSnapshot {
                seq: 5,
                first_seq: 1,
                items: [
                    card("<p><a href='https://example.com/docs?a=1&amp;b=&quot;2&quot;' onclick='x()'>Web docs</a></p>"),
                    card("<p><a href='src/long.rs#L150'>HTML source</a></p>"),
                    card("<p><img src='out/shot.png' alt='HTML image' onerror='x()'></p>"),
                    card("<p><a href='javascript:alert(1)'>Script link</a> <a href='mailto:a@b.c'>Mail</a></p>"),
                    card("<p><img src='https://example.com/x.png' alt='Remote image'>Remote</p>"),
                ]
                .into_iter()
                .map(|text| Item {
                    kind: "assistant_text".into(),
                    text,
                    ..Default::default()
                })
                .collect(),
            });
            this.update_view(Arc::new(view), window, cx);
        })
    });
    visual.run_until_parked();
    // TextView parses HTML behind a real-time debounce; retry clicks.
    let platform = cx.clone();
    click_until(&mut visual, Some(px(8.)), "html-card-live:a:0-0", |_| {
        platform.opened_url().is_some()
    });
    assert_eq!(
        cx.opened_url().as_deref(),
        Some("https://example.com/docs?a=1&b=%222%22"),
        "the web destination survives sanitizing and both HTML parses (then URL-normalized)"
    );
    assert!(commands.try_recv().is_err(), "web links never read files");
    let mut request = None;
    click_until(&mut visual, Some(px(8.)), "html-card-live:a:1-0", |_| {
        request = commands.try_recv().ok();
        request.is_some()
    });
    let Some(Command::Request(wks_native::features::Request::FilePreview { session, target })) =
        request
    else {
        panic!("an HTML file link must request a native preview");
    };
    assert_eq!(session, "a");
    assert_eq!(target.path, "/fixture/repo/src/long.rs");
    assert_eq!(target.line, Some(150));
    let mut request = None;
    click_until(&mut visual, Some(px(8.)), "html-card-live:a:2-0", |_| {
        request = commands.try_recv().ok();
        request.is_some()
    });
    let Some(Command::Request(wks_native::features::Request::FilePreview { target, .. })) = request
    else {
        panic!("an HTML image must request a native preview");
    };
    assert_eq!(target.path, "/fixture/repo/out/shot.png");
    assert_eq!(target.kind, wks_native::links::FileKind::Image);
    // Refused schemes and remote images were dropped by the sanitizer:
    // nothing opens, nothing is requested, and no router refusal ran.
    for card in ["html-card-live:a:3-0", "html-card-live:a:4-0"] {
        for _ in 0..25 {
            let Some(bounds) = visual.debug_bounds(card) else {
                visual.run_until_parked();
                std::thread::sleep(std::time::Duration::from_millis(20));
                continue;
            };
            visual.simulate_click(
                bounds.origin + gpui::point(px(20.), px(8.)),
                gpui::Modifiers::default(),
            );
            visual.run_until_parked();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
    assert_eq!(
        cx.opened_url().as_deref(),
        Some("https://example.com/docs?a=1&b=%222%22")
    );
    assert!(commands.try_recv().is_err());
    workspace.read_with(&visual, |this, _| assert!(this.extras.notice.is_empty()));
}

#[gpui::test]
fn long_transcript_only_builds_visible_rows(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    let mut view = state("a");
    let mut transcript = Transcript::default();
    transcript.snapshot(ConversationSnapshot {
        seq: 2000,
        first_seq: 1,
        items: (0..2000)
            .map(|i| Item {
                kind: "assistant_text".into(),
                text: format!("Message {i}\n\nA paragraph in a long transcript."),
                ..Default::default()
            })
            .collect(),
    });
    view.transcript = transcript;
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| this.update_view(Arc::new(view), window, cx))
    });
    visual.run_until_parked();
    visual.simulate_resize(size(px(900.), px(650.)));
    visual.run_until_parked();
    workspace.read_with(&visual, |this, _| {
        let rendered = this.rendered_rows.borrow().len();
        assert!(
            rendered > 0 && rendered < 100,
            "constructed {rendered} of 2000 rows"
        );
    });
}

#[gpui::test]
fn consecutive_tools_group_expand_and_keep_draft(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    let mut view = state("a");
    view.transcript.snapshot(ConversationSnapshot {
        seq: 3,
        first_seq: 1,
        items: (0..3)
            .map(|i| Item {
                kind: "tool_use".into(),
                id: format!("call-{i}"),
                name: "Read".into(),
                input: serde_json::json!({"file_path":format!("src/file-{i}.rs")}),
                ..Default::default()
            })
            .collect(),
    });
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(view), window, cx);
            this.composer
                .update(cx, |input, cx| input.set_value("keep my draft", window, cx));
        })
    });
    visual.run_until_parked();
    assert!(visual.debug_bounds("tool-activity-group").is_some());
    // Work cards open with one-line steps; the header collapses them.
    assert!(visual.debug_bounds("tool-toggle-0").is_some());
    let open = visual
        .debug_bounds("last-transcript-row")
        .unwrap()
        .size
        .height;
    let toggle = visual.debug_bounds("work-card-toggle").unwrap();
    visual.simulate_click(toggle.center(), gpui::Modifiers::default());
    visual.run_until_parked();
    assert!(
        visual
            .debug_bounds("last-transcript-row")
            .unwrap()
            .size
            .height
            < open
    );
    let toggle = visual.debug_bounds("work-card-toggle").unwrap();
    visual.simulate_click(toggle.center(), gpui::Modifiers::default());
    visual.run_until_parked();
    let tool = visual.debug_bounds("tool-toggle-0").unwrap();
    visual.simulate_click(tool.center(), gpui::Modifiers::default());
    visual.run_until_parked();
    workspace.read_with(&visual, |this, cx| {
        assert_eq!(this.composer.read(cx).value().as_ref(), "keep my draft");
        assert_eq!(this.tool_expansion.get("call:call-0"), Some(&true));
    });
    assert!(commands.try_recv().is_err());
}

#[gpui::test]
fn merged_turn_card_carries_interior_notes_only(cx: &mut TestAppContext) {
    let (workspace, mut visual, _, _updates) = fixture(cx);
    let mut view = state("a");
    let call = |i: usize| Item {
        kind: "tool_use".into(),
        id: format!("call-{i}"),
        name: "Read".into(),
        input: serde_json::json!({"file_path":format!("src/file-{i}.rs")}),
        ..Default::default()
    };
    let note = |text: &str| Item {
        kind: "assistant_text".into(),
        text: text.into(),
        ..Default::default()
    };
    view.transcript.snapshot(ConversationSnapshot {
        seq: 4,
        first_seq: 1,
        items: vec![
            call(0),
            note("Now the second file."),
            call(2),
            note("All done."),
        ],
    });
    let view = Arc::new(view);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| this.update_view(view.clone(), window, cx))
    });
    visual.run_until_parked();
    // Off (the default): prose splits the calls into separate cards.
    assert!(visual.debug_bounds("work-note-1").is_none());
    assert!(visual.debug_bounds("work-card-toggle").is_none());
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.settings.merge_turn_tools = true;
            let count = this.view.transcript.rows.len();
            this.list.splice(0..count, count);
            this.update_view(view.clone(), window, cx);
            cx.notify();
        })
    });
    visual.run_until_parked();
    assert!(visual.debug_bounds("work-card-toggle").is_some());
    assert!(visual.debug_bounds("work-note-1").is_some());
    // The closing answer stays an ordinary message below the card.
    assert!(visual.debug_bounds("work-note-3").is_none());
}

#[gpui::test]
fn orchestration_calls_use_the_work_card_shell(cx: &mut TestAppContext) {
    let (workspace, mut visual, _, _updates) = fixture(cx);
    let mut view = state("a");
    view.transcript.snapshot(ConversationSnapshot {
        seq: 1,
        first_seq: 1,
        items: vec![Item {
            kind: "tool_use".into(),
            id: "skill-0".into(),
            name: "Skill".into(),
            input: serde_json::json!({"skill":"review"}),
            ..Default::default()
        }],
    });
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| this.update_view(Arc::new(view), window, cx))
    });
    visual.run_until_parked();
    let card = visual.debug_bounds("orchestration-card").unwrap();
    let toggle = visual.debug_bounds("tool-toggle-0").unwrap();
    assert!(visual.debug_bounds("tool-activity-group").is_none());
    // The header row spans the card, inset only by its border.
    assert!(toggle.size.width + px(4.) >= card.size.width);
    visual.simulate_click(toggle.center(), gpui::Modifiers::default());
    visual.run_until_parked();
    workspace.read_with(&visual, |this, _| {
        assert_eq!(this.tool_expansion.get("call:skill-0"), Some(&true));
    });
}

#[gpui::test]
fn message_timestamps_are_below_user_assistant_and_tool_content(cx: &mut TestAppContext) {
    let (workspace, mut visual, _, _updates) = fixture(cx);
    for role in ["user_message", "assistant_text", "tool_use"] {
        let mut view = state("a");
        view.transcript.snapshot(ConversationSnapshot {
            seq: 1,
            first_seq: 1,
            items: vec![Item {
                kind: role.into(),
                text: "A message to timestamp".into(),
                id: "timestamped".into(),
                name: "Read".into(),
                input: serde_json::json!({"file_path":"src/main.rs"}),
                ..Default::default()
            }],
        });
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.update_view(Arc::new(view), window, cx);
            })
        });
        visual.run_until_parked();
        let footer = visual.debug_bounds("message-timestamp-footer").unwrap();
        let row = visual.debug_bounds("last-transcript-row").unwrap();
        assert!(footer.top() > row.top() + row.size.height / 2.);
    }
}

#[gpui::test]
fn tool_cards_expand_without_sending_and_code_languages_are_highlighted(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    let mut view = state("a");
    view.transcript.snapshot(ConversationSnapshot {
        seq: 2,
        first_seq: 1,
        items: vec![
            Item {
                kind: "tool_use".into(),
                id: "call".into(),
                name: "Bash".into(),
                input: serde_json::json!({"command":"printf hello"}),
                ..Default::default()
            },
            Item {
                kind: "tool_result".into(),
                tool_use_id: "call".into(),
                content: "hello".into(),
                ..Default::default()
            },
        ],
    });
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| this.update_view(Arc::new(view), window, cx))
    });
    visual.run_until_parked();
    let collapsed = visual.debug_bounds("last-transcript-row").unwrap();
    let toggle = visual.debug_bounds("tool-toggle-0").unwrap();
    visual.simulate_click(toggle.center(), gpui::Modifiers::default());
    visual.run_until_parked();
    assert!(
        visual
            .debug_bounds("last-transcript-row")
            .unwrap()
            .size
            .height
            > collapsed.size.height
    );
    assert!(commands.try_recv().is_err());
    let toggle = visual.debug_bounds("tool-toggle-0").unwrap();
    visual.simulate_click(toggle.center(), gpui::Modifiers::default());
    visual.run_until_parked();
    assert_eq!(
        visual
            .debug_bounds("last-transcript-row")
            .unwrap()
            .size
            .height,
        collapsed.size.height
    );
    for (language, code) in [
        ("rust", "fn main() { let count = 42; }"),
        ("typescript", "const count: number = 42;"),
        ("diff", "@@ -1 +1 @@\n-old\n+new\n"),
    ] {
        let mut highlighter = gpui_component::highlighter::SyntaxHighlighter::new(language);
        highlighter.update(None, &gpui_component::input::Rope::from_str(code));
        let styles = highlighter.styles(
            &(0..code.len()),
            &gpui_component::highlighter::HighlightTheme::default_dark(),
        );
        assert!(
            !styles.is_empty(),
            "{language} must have real syntax highlighting"
        );
    }
}

#[gpui::test]
fn open_tool_cards_survive_reseeds_and_late_results(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    let call = Item {
        kind: "tool_use".into(),
        id: "stable-call".into(),
        name: "Bash".into(),
        input: serde_json::json!({"command":"cargo test"}),
        ..Default::default()
    };
    let mut view = state("a");
    view.transcript.snapshot(ConversationSnapshot {
        seq: 1,
        first_seq: 1,
        items: vec![call.clone()],
    });
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| this.update_view(Arc::new(view), window, cx))
    });
    visual.run_until_parked();
    let closed_height = visual
        .debug_bounds("last-transcript-row")
        .unwrap()
        .size
        .height;
    let toggle = visual.debug_bounds("tool-toggle-0").unwrap();
    visual.simulate_click(toggle.center(), gpui::Modifiers::default());
    visual.run_until_parked();
    for _ in 0..3 {
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                let mut view = (*this.view).clone();
                view.transcript.snapshot(ConversationSnapshot {
                    seq: 3,
                    first_seq: 1,
                    items: vec![
                        Item {
                            kind: "assistant_text".into(),
                            text: "Running the checks.".into(),
                            ..Default::default()
                        },
                        call.clone(),
                        Item {
                            kind: "tool_result".into(),
                            tool_use_id: "stable-call".into(),
                            content: "All tests passed".into(),
                            ..Default::default()
                        },
                    ],
                });
                this.update_view(Arc::new(view), window, cx);
            })
        });
        visual.run_until_parked();
        assert!(
            visual
                .debug_bounds("last-transcript-row")
                .unwrap()
                .size
                .height
                > closed_height
        );
        workspace.read_with(&visual, |this, _| {
            assert_eq!(this.tool_expansion.get("call:stable-call"), Some(&true))
        });
    }
    // A deliberate collapse must also survive the next server snapshot.
    let toggle = visual.debug_bounds("tool-toggle-0").unwrap();
    visual.simulate_click(toggle.center(), gpui::Modifiers::default());
    visual.run_until_parked();
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            let mut view = (*this.view).clone();
            view.transcript.snapshot(ConversationSnapshot {
                seq: 1,
                first_seq: 1,
                items: vec![call],
            });
            this.update_view(Arc::new(view), window, cx);
        })
    });
    visual.run_until_parked();
    workspace.read_with(&visual, |this, _| {
        assert_eq!(this.tool_expansion.get("call:stable-call"), Some(&false))
    });
    assert_eq!(
        visual
            .debug_bounds("last-transcript-row")
            .unwrap()
            .size
            .height,
        closed_height
    );
}

#[gpui::test]
fn structured_results_render_in_assistant_and_completion_messages(cx: &mut TestAppContext) {
    let (workspace, mut visual, _, _updates) = fixture(cx);
    for text in [
        "Done\n```wks-result\n{\"merged\":true,\"caveats\":[\"Not checked\"],\"customField\":{\"count\":2}}\n```",
        "[fleet] Worker finished:\n- Builder (session:b, cwd /repo) — last reply: Done\n\nStructured result — Builder (session:b):\n{\"ok\":true}",
        "```wks-result\ninvalid JSON\n```",
    ] {
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                let mut view = state("a");
                view.transcript.snapshot(ConversationSnapshot {
                    seq: 1,
                    first_seq: 1,
                    items: vec![Item {
                        kind: if text.starts_with("[fleet]") {
                            "user_message"
                        } else {
                            "assistant_text"
                        }
                        .into(),
                        text: text.into(),
                        ..Default::default()
                    }],
                });
                this.update_view(Arc::new(view), window, cx);
            })
        });
        visual.run_until_parked();
        assert!(visual.debug_bounds("structured-result-card").is_some());
    }
}

fn fleet_wake_view(wake: &str, approval: bool) -> View {
    let mut view = state("manager");
    let mut sessions = fleet_sessions();
    if approval {
        sessions[2].approval = Some(serde_json::json!({"toolName":"Bash"}));
    }
    view.sessions = Arc::new(sessions);
    view.transcript.snapshot(ConversationSnapshot {
        seq: 2,
        first_seq: 1,
        items: vec![
            Item {
                kind: "user_message".into(),
                text: "Keep going with the plan.".into(),
                ..Default::default()
            },
            Item {
                kind: "user_message".into(),
                text: wake.into(),
                timestamp: Some(chrono::Utc::now().to_rfc3339()),
                ..Default::default()
            },
        ],
    });
    view
}

const BLOCKED_WAKE: &str = "[supervisor] An agent is now blocked on a decision:\n- Native editor · terminal · Git (session:worker-1, approval)\nRun a /supervise pass now.";

#[gpui::test]
fn fleet_wakes_render_as_named_worker_cards_not_user_bubbles(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(fleet_wake_view(BLOCKED_WAKE, true)), window, cx);
            this.composer
                .update(cx, |input, cx| input.set_value("My draft", window, cx));
            let fleet = wks_native::transcript::fleet(BLOCKED_WAKE).unwrap();
            // Named for the worker, not "You", and never its raw UUID.
            assert_eq!(this.fleet_title(&fleet), "Native editor · terminal · Git");
        })
    });
    visual.run_until_parked();
    let card = visual.debug_bounds("fleet-card").unwrap();
    let column = visual.debug_bounds("chat-content-column").unwrap();
    // A full-width card, not a right-aligned 85% user bubble.
    assert!(card.size.width >= column.size.width - px(8.));
    assert!(visual.debug_bounds("fleet-card-status").is_some());
    let header = visual.debug_bounds("fleet-card-header").unwrap();
    assert!(header.size.height <= px(56.), "header {:?}", header.size);
    // Compact icon actions, not wide text buttons.
    let open = visual.debug_bounds("fleet-entry-open-0").unwrap();
    let reply = visual.debug_bounds("fleet-entry-reply-0").unwrap();
    assert!(open.size.width <= px(32.) && reply.size.width <= px(32.));
    // The whole single-worker card stays compact.
    assert!(card.size.height <= px(140.), "card {:?}", card.size);
    // Approval vs resolved follows the live session.
    let status = |this: &Workspace| {
        let fleet = wks_native::transcript::fleet(BLOCKED_WAKE).unwrap();
        let live = this.view.sessions.iter().find(|s| s.id == "worker-1");
        fleet_card::entry_status(
            fleet.kind,
            &fleet.entries[0],
            live,
            this.appearance.palette(),
        )
        .0
    };
    workspace.read_with(&visual, |this, _| {
        assert_eq!(status(this), "Needs approval")
    });
    // Open selects the direct worker; Reply keeps the draft.
    visual.simulate_click(open.center(), gpui::Modifiers::default());
    visual.run_until_parked();
    assert!(matches!(next_effect(&mut commands), Some(Command::Select(id)) if id == "worker-1"));
    visual.simulate_click(reply.center(), gpui::Modifiers::default());
    visual.run_until_parked();
    workspace.read_with(&visual, |this, cx| {
        assert_eq!(
            this.composer.read(cx).value().as_ref(),
            "My draft\nRe: session:worker-1 (Native editor · terminal · Git) — "
        );
    });
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(fleet_wake_view(BLOCKED_WAKE, false)), window, cx);
            assert_eq!(status(this), "Resolved");
        })
    });
    // The original wake is a secondary disclosure.
    visual.run_until_parked();
    let toggle = visual.debug_bounds("fleet-toggle-Original wake").unwrap();
    visual.simulate_click(toggle.center(), gpui::Modifiers::default());
    visual.run_until_parked();
    workspace.read_with(&visual, |this, _| {
        assert!(this.chat.open.values().any(|open| *open));
    });
    // Toggled closed by mouse, the chip keeps no "still on" accent ring;
    // reached by keyboard it shows one.
    visual.simulate_click(toggle.center(), gpui::Modifiers::default());
    visual.run_until_parked();
    workspace.read_with(&visual, |this, _| {
        assert!(!this.chat.open.values().any(|open| *open));
    });
    let toggle = visual.debug_bounds("fleet-toggle-Original wake").unwrap();
    assert!(!accent_border(&workspace, &mut visual, toggle));
    visual.simulate_keystrokes("tab shift-tab");
    visual.run_until_parked();
    assert!(accent_border(&workspace, &mut visual, toggle));
}

#[gpui::test]
fn multi_worker_wakes_share_a_title_and_guard_foreign_sessions(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _) = fixture(cx);
    let wake = "[fleet] Worker finished:\n- Native controls (session:worker-2, cwd /home/u/.workspacer/worktrees/workspacer/native-controls-projects-codex) — last reply: Done\n- Someone else's (session:foreign, cwd /x) — FAILED: API failed\nReview each.";
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            let mut view = fleet_wake_view(wake, false);
            let sessions = Arc::make_mut(&mut view.sessions);
            sessions.push(Session {
                id: "foreign".into(),
                parent_session_id: "another-manager".into(),
                ..Default::default()
            });
            this.update_view(Arc::new(view), window, cx);
            let fleet = wks_native::transcript::fleet(wake).unwrap();
            assert_eq!(this.fleet_title(&fleet), "2 sessions");
        })
    });
    visual.run_until_parked();
    assert!(visual.debug_bounds("fleet-entry-status-0").is_some());
    assert!(visual.debug_bounds("fleet-entry-status-1").is_some());
    assert!(visual.debug_bounds("fleet-card-status").is_none());
    // Another manager's worker cannot be opened from this conversation.
    let foreign = visual.debug_bounds("fleet-entry-open-1").unwrap();
    visual.simulate_click(foreign.center(), gpui::Modifiers::default());
    visual.run_until_parked();
    assert!(next_effect(&mut commands).is_none());
    let own = visual.debug_bounds("fleet-entry-open-0").unwrap();
    visual.simulate_click(own.center(), gpui::Modifiers::default());
    visual.run_until_parked();
    assert!(matches!(next_effect(&mut commands), Some(Command::Select(id)) if id == "worker-2"));
}

#[gpui::test]
fn ordinary_user_messages_keep_their_bubble(cx: &mut TestAppContext) {
    // A fresh window: no card has ever rendered (see debug_bounds note).
    let (workspace, mut visual, _, _) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            let view = fleet_wake_view("[fleet] Worker finished: not a bullet list", false);
            this.update_view(Arc::new(view), window, cx)
        })
    });
    visual.run_until_parked();
    assert!(visual.debug_bounds("fleet-card").is_none());
    assert!(visual.debug_bounds("last-transcript-row").is_some());
}

#[gpui::test]
fn timestamps_keep_clear_space_from_neighbouring_cards(cx: &mut TestAppContext) {
    let (workspace, mut visual, _, _) = fixture(cx);
    let stamp = Some(chrono::Utc::now().to_rfc3339());
    let mut view = state("a");
    view.transcript.snapshot(ConversationSnapshot {
        seq: 3,
        first_seq: 1,
        items: vec![
            Item {
                kind: "tool_use".into(),
                id: "snap".into(),
                name: "mcp__workspacer__get_snapshot".into(),
                input: serde_json::json!({"sessionId":"x"}),
                timestamp: stamp.clone(),
                ..Default::default()
            },
            Item {
                kind: "tool_result".into(),
                tool_use_id: "snap".into(),
                text: "ok".into(),
                timestamp: stamp.clone(),
                ..Default::default()
            },
            Item {
                kind: "assistant_text".into(),
                text: "Approved it.".into(),
                timestamp: stamp.clone(),
                ..Default::default()
            },
        ],
    });
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| this.update_view(Arc::new(view), window, cx))
    });
    visual.run_until_parked();
    let card = visual.debug_bounds("tool-activity-group").unwrap();
    let footer = visual.debug_bounds("work-card-timestamp").unwrap();
    let next = visual.debug_bounds("last-transcript-row").unwrap();
    let above = footer.top() - card.bottom();
    let below = next.top() - footer.bottom();
    assert!(
        above >= px(transcript::META_GAP_ABOVE - 0.5),
        "card→time {above:?}"
    );
    assert!(
        below >= px(transcript::META_GAP_BELOW - 0.5),
        "time→next {below:?}"
    );
    // Deliberate, not huge.
    assert!(
        above <= px(12.) && below <= px(20.),
        "{above:?} / {below:?}"
    );
}
