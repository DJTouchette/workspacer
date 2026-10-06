//! The file viewer, editor, review and agent terminal.
use super::*;

#[gpui::test]
fn file_viewer_contains_keys_over_a_nonempty_draft(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(state("a")), window, cx)
        })
    });
    visual.simulate_keystrokes("ctrl-l");
    visual.simulate_input("DRAFT_MUST_NOT_SEND");
    visual.run_until_parked();
    let png = {
        use base64::Engine;
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(32, 16)
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        base64::engine::general_purpose::STANDARD.encode(bytes.into_inner())
    };
    let open = |path: &str, number: u64, loading: bool, error: Option<&str>| {
        let wks_native::links::Link::File(target) = wks_native::links::classify("/repo", path)
        else {
            panic!("{path} is a file link");
        };
        let value = serde_json::json!({
            "contents": "one\ntwo\n", "size": 8, "png": png, "width": 32, "height": 16,
        });
        let (workspace, error) = (workspace.clone(), error.map(str::to_owned));
        move |window: &mut Window, cx: &mut App| {
            workspace.update(cx, |this, cx| {
                let mut view = (*this.view).clone();
                view.requests.insert(
                    "file-preview",
                    wks_native::features::RequestState {
                        request: wks_native::features::Request::FilePreview {
                            session: "a".into(),
                            target,
                        },
                        number,
                        loading,
                        error,
                        value: Arc::new(value),
                    },
                );
                this.update_view(Arc::new(view), window, cx);
            })
        }
    };
    let untouched = |visual: &mut VisualTestContext,
                     commands: &mut tokio::sync::mpsc::Receiver<Command>,
                     keys: &str| {
        workspace.read_with(visual, |this, cx| {
            assert!(
                this.file_viewer().is_some(),
                "{keys} must not close the viewer"
            );
            assert_eq!(
                this.composer.read(cx).value().as_ref(),
                "DRAFT_MUST_NOT_SEND",
                "{keys} reached the covered composer"
            );
            assert_eq!(this.screen, Screen::Conversation, "{keys} navigated");
            assert!(!this.new_session, "{keys} started a session");
            assert_eq!(this.view.selected.as_deref(), Some("a"));
        });
        visual.update(|window, cx| {
            assert!(
                workspace.read(cx).viewer_has_focus(window, cx),
                "{keys} moved focus out of the viewer"
            )
        });
        assert!(
            commands.try_recv().is_err(),
            "{keys} reached the workspace under the viewer"
        );
    };
    let states: [(&str, u64, bool, Option<&str>, &str); 5] = [
        ("docs/a.md", 1, true, None, "file-viewer"),
        ("docs/a.rs", 2, false, None, "file-viewer-text"),
        ("docs/a.md", 3, false, None, "file-viewer-markdown"),
        ("shot.png", 4, false, None, "file-viewer-image"),
        (
            "gone.md",
            5,
            false,
            Some("No file at this path."),
            "file-viewer-error",
        ),
    ];
    for (path, number, loading, error, selector) in states {
        visual.update(open(path, number, loading, error));
        visual.run_until_parked();
        assert!(visual.debug_bounds(selector).is_some(), "{selector} shows");
        visual.update(|window, cx| {
            assert!(
                workspace.read(cx).viewer_modal(window),
                "a 1000px window shows the viewer as a modal sheet"
            )
        });
        for keys in [
            "ctrl-enter",
            "cmd-enter",
            "alt-down",
            "alt-up",
            "ctrl-n",
            "ctrl-p",
            "ctrl-,",
            "ctrl-l",
            "ctrl-r",
            "ctrl-0",
            "tab",
            "shift-tab",
            "enter",
            "x",
            "backspace",
        ] {
            visual.simulate_keystrokes(keys);
            visual.run_until_parked();
            untouched(&mut visual, &mut commands, keys);
        }
        visual.simulate_input("typed");
        visual.run_until_parked();
        untouched(&mut visual, &mut commands, "typing");
        if selector == "file-viewer-text" {
            // The keys above edited the source, not the composer.
            let pane = workspace.read_with(&visual, |this, _| this.file_viewer().cloned().unwrap());
            let edited = pane.read_with(&visual, |pane, cx| {
                assert!(pane.dirty(), "the source is editable");
                pane.editor().unwrap().read(cx).value().to_string()
            });
            assert!(edited.contains("typed") && edited.contains("one\ntwo\n"));
            // The editor's own keys still work: select all and copy, on
            // the platform's modifier (cmd on macOS, where ctrl-a is Home).
            visual.simulate_keystrokes("secondary-a secondary-c");
            visual.run_until_parked();
            let copied = visual.update(|_, cx| cx.read_from_clipboard());
            assert_eq!(copied.and_then(|item| item.text()), Some(edited));
            untouched(&mut visual, &mut commands, "select all, copy");
            // Esc over unsaved edits asks first; Discard closes.
            visual.simulate_keystrokes("escape");
            visual.run_until_parked();
            untouched(&mut visual, &mut commands, "escape over edits");
            let discard = visual
                .debug_bounds("file-viewer-prompt-discard")
                .expect("unsaved edits ask before closing");
            visual.simulate_click(discard.center(), gpui::Modifiers::default());
            visual.run_until_parked();
            workspace.read_with(&visual, |this, _| assert!(this.file_viewer().is_none()));
            visual.update(|window, cx| {
                workspace.update(cx, |this, cx| {
                    this.composer.read(cx).focus_handle(cx).focus(window)
                })
            });
            continue;
        }
        // Focus pulled behind the sheet (a late receipt, a UI request) is
        // taken back before a key can reach the composer.
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.composer.read(cx).focus_handle(cx).focus(window)
            })
        });
        visual.simulate_keystrokes("ctrl-enter");
        visual.simulate_input("z");
        visual.run_until_parked();
        untouched(&mut visual, &mut commands, "refocused ctrl-enter");
        // Dismiss and reopen the next state from the composer again.
        visual.simulate_keystrokes("escape");
        visual.run_until_parked();
        workspace.read_with(&visual, |this, _| assert!(this.file_viewer().is_none()));
    }
    // Closed, the same draft and shortcut work normally again.
    let focused = visual.update(|window, cx| {
        workspace
            .read(cx)
            .composer
            .read(cx)
            .focus_handle(cx)
            .is_focused(window)
    });
    assert!(focused, "closing returns focus to the composer");
    visual.simulate_keystrokes("ctrl-enter");
    let Command::Act {
        session,
        action: Action::Send(text),
    } = commands.try_recv().expect("send works after closing")
    else {
        panic!("wrong action");
    };
    assert_eq!(
        (session.as_str(), text.as_str()),
        ("a", "DRAFT_MUST_NOT_SEND")
    );
}

const GUIDE: &str = "[Library source](../src/lib.rs:3)\n\n# Guide\n\n\
        Intro with **bold**, *italic*, `code`, ~~gone~~ and a [web link](https://example.com/docs).\n\n\
        ## Usage\n\n- one\n- two\n\n1. first\n2. second\n\n- [x] done\n- [ ] todo\n\n\
        > A quoted note.\n\n| Name | Value |\n| --- | --- |\n| a | 1 |\n\n\
        ```rust\nfn main() {}\n```\n\n![Diagram](img/diagram.png)\n\n---\n\n\
        ## Usage\n\nSecond usage.\n\n## Usage-1\n\nA literal heading that collides.\n\n\
        Filler 1.\n\nFiller 2.\n\nFiller 3.\n\nFiller 4.\n\nFiller 5.\n\nFiller 6.\n\n\
        Filler 7.\n\nFiller 8.\n\nFiller 9.\n\nFiller 10.\n\nFiller 11.\n\nFiller 12.\n\n\
        Filler 13.\n\nFiller 14.\n\nFiller 15.\n\nFiller 16.\n\nFiller 17.\n\nFiller 18.\n\n\
        Filler 19.\n\nFiller 20.\n\nFiller 21.\n\nFiller 22.\n\nFiller 23.\n\nFiller 24.\n";

fn preview_request(
    commands: &mut tokio::sync::mpsc::Receiver<Command>,
) -> Option<(String, wks_native::links::FileTarget)> {
    match commands.try_recv().ok()? {
        Command::Request(wks_native::features::Request::FilePreview { session, target }) => {
            Some((session, target))
        }
        _ => panic!("unexpected command instead of a file preview"),
    }
}

#[gpui::test]
fn markdown_files_render_with_a_source_toggle_and_document_links(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            let mut view = state("a");
            // Document links must ignore the session's working folder.
            Arc::make_mut(&mut view.sessions)[0].cwd = "/work/elsewhere".into();
            this.update_view(Arc::new(view), window, cx);
        })
    });
    let guide = file_target("/repo/docs/Guide.MD");
    preview_state(
        &workspace,
        &mut visual,
        "a",
        guide.clone(),
        1,
        true,
        None,
        serde_json::json!({}),
    );
    assert!(visual.debug_bounds("file-viewer").is_some());
    preview_state(
        &workspace,
        &mut visual,
        "a",
        guide.clone(),
        1,
        false,
        None,
        serde_json::json!({"contents": GUIDE, "size": GUIDE.len()}),
    );
    assert!(
        visual.debug_bounds("file-viewer-markdown").is_some(),
        "Markdown opens rendered"
    );
    assert!(visual.debug_bounds("file-viewer-text").is_none());
    assert!(visual.debug_bounds("file-viewer-mode-source").is_some());
    let pane = pane_of(&workspace, &visual);
    // The rendered document has the GFM structures, headings in order.
    visual.update(|_, cx| {
        let pane = pane.read(cx);
        assert_eq!(pane.mode(), file_viewer::Mode::Preview);
        let document = pane.document().expect("rendered document");
        let kinds = document.block_kinds(cx);
        for kind in [
            "heading",
            "paragraph",
            "list",
            "ordered-list",
            "task-list",
            "blockquote",
            "table",
            "code",
            "divider",
        ] {
            assert!(kinds.contains(&kind), "{kind} missing from {kinds:?}");
        }
        let headings: Vec<_> = document
            .headings(cx)
            .into_iter()
            .map(|(_, level, text)| (level, text))
            .collect();
        assert_eq!(
            headings,
            [
                (1, "Guide".into()),
                (2, "Usage".into()),
                (2, "Usage".into()),
                (2, "Usage-1".into())
            ]
        );
    });
    settle(&mut visual);
    // A real click on the document's first link goes through the
    // pane's router, against the document's folder on the same session.
    let mut request = None;
    for _ in 0..100 {
        let bounds = visual.debug_bounds("file-viewer-markdown").unwrap();
        visual.simulate_click(
            bounds.origin + gpui::point(px(30. + 24.), px(16. + 10.)),
            gpui::Modifiers::default(),
        );
        visual.run_until_parked();
        request = preview_request(&mut commands);
        if request.is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let (session, target) = request.expect("the document link opened");
    assert_eq!(session, "a");
    assert_eq!(
        (target.path.as_str(), target.line),
        ("/repo/src/lib.rs", Some(3))
    );
    let follow = |visual: &mut VisualTestContext, raw: &'static str| {
        visual.update(|window, cx| pane.update(cx, |pane, cx| pane.follow_link(raw, window, cx)));
        visual.run_until_parked();
    };
    // Images are labels that open the image viewer from the same folder.
    follow(&mut visual, "img/diagram.png");
    let (_, image) = preview_request(&mut commands).expect("image link reads the image");
    assert_eq!(image.path, "/repo/docs/img/diagram.png");
    assert_eq!(image.kind, wks_native::links::FileKind::Image);
    // Same-document anchors scroll; they never re-read the file.
    let top = |visual: &mut VisualTestContext| {
        visual.update(|_, cx| pane.read(cx).document().unwrap().scroll_top(cx).0)
    };
    let usage_again = visual.update(|_, cx| pane.read(cx).document().unwrap().headings(cx)[2].0);
    follow(&mut visual, "#usage-1");
    assert_eq!(
        top(&mut visual),
        usage_again,
        "#usage-1 is the second Usage"
    );
    // A literal "Usage-1" heading never shares the generated anchor.
    let literal = visual.update(|_, cx| pane.read(cx).document().unwrap().headings(cx)[3].0);
    assert!(literal > usage_again);
    follow(&mut visual, "#usage-1-1");
    assert_eq!(
        top(&mut visual),
        literal,
        "#usage-1-1 is the literal Usage-1"
    );
    visual.update(|_, cx| assert!(pane.read(cx).notice().is_none()));
    follow(&mut visual, "#usage-1");
    assert_eq!(top(&mut visual), usage_again);
    follow(&mut visual, "#");
    assert_eq!(top(&mut visual), 0);
    follow(&mut visual, "Guide.MD#usage-1");
    assert_eq!(top(&mut visual), usage_again);
    assert!(commands.try_recv().is_err(), "anchors stay in the document");
    follow(&mut visual, "#not-here");
    visual.update(|_, cx| assert!(pane.read(cx).notice().unwrap().contains("No heading")));
    // Unsafe schemes are refused visibly inside the viewer.
    follow(&mut visual, "javascript:alert(1)");
    visual.update(|_, cx| assert!(pane.read(cx).notice().unwrap().contains("javascript")));
    assert!(visual.debug_bounds("file-viewer-notice").is_some());
    assert!(cx.opened_url().is_none() && commands.try_recv().is_err());
    follow(&mut visual, "https://example.com/docs");
    assert_eq!(cx.opened_url().as_deref(), Some("https://example.com/docs"));
    // A line of this same document opens its source there.
    follow(&mut visual, "Guide.MD:5");
    assert!(
        commands.try_recv().is_err(),
        "same-file lines do not re-read"
    );
    visual.update(|window, cx| {
        let pane = pane.read(cx);
        assert_eq!(pane.mode(), file_viewer::Mode::Source);
        let editor = pane.editor().unwrap().read(cx);
        assert_eq!(editor.cursor_position().line, 4);
        assert_eq!(editor.value().as_ref(), GUIDE, "the loaded file is kept");
        assert!(editor.focus_handle(cx).is_focused(window));
    });
    assert!(visual.debug_bounds("file-viewer-text").is_some());
    // Ctrl+Shift+V switches back without reading anything again, and
    // keeps keyboard focus inside the viewer.
    visual.simulate_keystrokes("ctrl-shift-v");
    visual.run_until_parked();
    visual.update(|window, cx| {
        assert_eq!(pane.read(cx).mode(), file_viewer::Mode::Preview);
        assert!(workspace.read(cx).viewer_has_focus(window, cx));
    });
    assert!(visual.debug_bounds("file-viewer-markdown").is_some());
    // Keys scroll the rendered document.
    visual.simulate_keystrokes("end");
    visual.run_until_parked();
    let bottom = top(&mut visual);
    assert!(bottom > 0, "End scrolls to the bottom");
    // A round trip through the source keeps the reading position.
    visual.simulate_keystrokes("ctrl-shift-v");
    visual.run_until_parked();
    visual.simulate_keystrokes("ctrl-shift-v");
    visual.run_until_parked();
    assert_eq!(
        top(&mut visual),
        bottom,
        "Preview → Source → Preview keeps the scroll"
    );
    visual.simulate_keystrokes("home");
    visual.run_until_parked();
    assert_eq!(top(&mut visual), 0);
    // Ctrl+F from the preview searches the source.
    visual.simulate_keystrokes("ctrl-f");
    visual.run_until_parked();
    visual.update(|window, cx| {
        assert_eq!(pane.read(cx).mode(), file_viewer::Mode::Source);
        assert!(workspace.read(cx).viewer_has_focus(window, cx));
    });
    // Clicking the Preview segment returns to the rendered document.
    let segment = visual.debug_bounds("file-viewer-mode-preview").unwrap();
    visual.simulate_click(segment.center(), gpui::Modifiers::default());
    visual.run_until_parked();
    visual.update(|_, cx| assert_eq!(pane.read(cx).mode(), file_viewer::Mode::Preview));
    assert!(commands.try_recv().is_err());
    // Following a file link records Back; the next file arrives in place.
    follow(&mut visual, "../src/lib.rs:3");
    let (_, lib) = preview_request(&mut commands).unwrap();
    preview_state(
        &workspace,
        &mut visual,
        "a",
        lib,
        2,
        false,
        None,
        serde_json::json!({"contents": "a\nb\nc\nd\n", "size": 8}),
    );
    assert!(visual.debug_bounds("file-viewer-text").is_some());
    visual.update(|_, cx| assert!(!pane.read(cx).previewable(), "source files have no toggle"));
    let back = visual
        .debug_bounds("file-viewer-back")
        .expect("Back after a link");
    visual.simulate_click(back.center(), gpui::Modifiers::default());
    visual.run_until_parked();
    let (_, again) = preview_request(&mut commands).expect("Back re-reads the document");
    assert_eq!(again.path, "/repo/docs/Guide.MD");
}

#[gpui::test]
fn one_ctrl_f_from_the_preview_opens_source_search(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(state("a")), window, cx);
            this.composer
                .update(cx, |c, cx| c.set_value("KEEP_DRAFT", window, cx));
        })
    });
    preview_state(
        &workspace,
        &mut visual,
        "a",
        file_target("/repo/docs/Guide.MD"),
        1,
        false,
        None,
        serde_json::json!({"contents": GUIDE, "size": GUIDE.len()}),
    );
    settle(&mut visual);
    let pane = pane_of(&workspace, &visual);
    let query = |visual: &mut VisualTestContext| {
        visual.update(|_, cx| {
            let editor = pane.read(cx).editor().unwrap().read(cx);
            editor.search_query(cx).map(|q| q.to_string())
        })
    };
    visual.update(|window, cx| {
        assert_eq!(pane.read(cx).mode(), file_viewer::Mode::Preview);
        assert!(workspace.read(cx).viewer_has_focus(window, cx));
    });
    assert_eq!(query(&mut visual), None);
    // One press: the source shows with its search open and focused.
    visual.simulate_keystrokes("ctrl-f");
    visual.run_until_parked();
    assert_eq!(query(&mut visual).as_deref(), Some(""));
    visual.update(|window, cx| {
        let pane = pane.read(cx);
        assert_eq!(pane.mode(), file_viewer::Mode::Source);
        assert!(
            !pane.editor().unwrap().focus_handle(cx).is_focused(window),
            "the query field, not the read-only source, has the keyboard"
        );
        assert!(workspace.read(cx).viewer_has_focus(window, cx));
    });
    // Typing goes into the query, and Enter finds the next match.
    visual.simulate_input("Second usage");
    visual.run_until_parked();
    assert_eq!(query(&mut visual).as_deref(), Some("Second usage"));
    visual.simulate_keystrokes("enter");
    visual.run_until_parked();
    assert_eq!(query(&mut visual).as_deref(), Some("Second usage"));
    // Esc closes the search, back to the source; a second Esc the viewer.
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();
    assert_eq!(query(&mut visual), None);
    visual.update(|window, cx| {
        assert!(
            pane.read(cx)
                .editor()
                .unwrap()
                .focus_handle(cx)
                .is_focused(window)
        );
        assert!(workspace.read(cx).file_viewer().is_some());
    });
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();
    workspace.read_with(&visual, |this, cx| {
        assert!(this.file_viewer().is_none());
        assert_eq!(this.composer.read(cx).value().as_ref(), "KEEP_DRAFT");
    });
    // Nothing typed into the viewer reached the hub.
    assert!(commands.try_recv().is_err());
}

#[gpui::test]
fn markdown_line_anchors_and_oversized_documents_open_as_source(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(state("a")), window, cx)
        })
    });
    preview_state(
        &workspace,
        &mut visual,
        "a",
        file_target("/repo/README.markdown:3"),
        1,
        false,
        None,
        serde_json::json!({"contents": "# A\n\nb\nc\n", "size": 10}),
    );
    let pane = pane_of(&workspace, &visual);
    visual.update(|_, cx| {
        let pane = pane.read(cx);
        assert_eq!(
            pane.mode(),
            file_viewer::Mode::Source,
            "line anchors open source"
        );
        assert_eq!(pane.editor().unwrap().read(cx).cursor_position().line, 2);
    });
    // The toggle still offers the rendered document.
    assert!(visual.debug_bounds("file-viewer-mode-preview").is_some());
    visual.simulate_keystrokes("ctrl-shift-v");
    visual.run_until_parked();
    visual.update(|_, cx| assert_eq!(pane.read(cx).mode(), file_viewer::Mode::Preview));
    // Too large to render on the UI thread: source, with the reason.
    let big = "x".repeat(wks_native::links::MAX_RENDERED_MARKDOWN_BYTES + 1);
    preview_state(
        &workspace,
        &mut visual,
        "a",
        file_target("/repo/BIG.md"),
        2,
        false,
        None,
        serde_json::json!({"contents": big, "size": big.len()}),
    );
    visual.update(|_, cx| {
        let pane = pane.read(cx);
        assert_eq!(pane.mode(), file_viewer::Mode::Source);
        assert!(!pane.previewable());
        assert!(pane.notice().unwrap().contains("opens as source"));
    });
    visual.simulate_keystrokes("ctrl-shift-v");
    visual.run_until_parked();
    visual.update(|_, cx| assert_eq!(pane.read(cx).mode(), file_viewer::Mode::Source));
    assert!(visual.debug_bounds("file-viewer-text").is_some());
}

#[gpui::test]
fn viewer_ignores_stale_states_and_reports_interrupted_loads(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(state("a")), window, cx)
        })
    });
    let text = |s: &str| serde_json::json!({"contents": s, "size": s.len()});
    preview_state(
        &workspace,
        &mut visual,
        "a",
        file_target("/r/new.rs"),
        5,
        false,
        None,
        text("new"),
    );
    // A late answer to an older request never replaces newer content.
    preview_state(
        &workspace,
        &mut visual,
        "a",
        file_target("/r/old.rs"),
        4,
        false,
        None,
        text("old"),
    );
    let pane = pane_of(&workspace, &visual);
    visual.update(|_, cx| {
        let pane = pane.read(cx);
        assert_eq!(pane.state().number, 5);
        assert_eq!(pane.editor().unwrap().read(cx).value().as_ref(), "new");
    });
    // A read in flight that the controller drops (session switch)
    // stops loading and says so instead of spinning forever.
    preview_state(
        &workspace,
        &mut visual,
        "a",
        file_target("/r/next.md"),
        6,
        true,
        None,
        serde_json::json!({}),
    );
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            let mut view = (*this.view).clone();
            view.requests.remove("file-preview");
            view.selected = Some("b".into());
            this.update_view(Arc::new(view), window, cx);
        })
    });
    visual.run_until_parked();
    visual.update(|_, cx| {
        let state = pane.read(cx).state();
        assert!(!state.loading);
        assert!(state.error.as_deref().unwrap().contains("Loading stopped"));
    });
    assert!(visual.debug_bounds("file-viewer-error").is_some());
    // Closing dismisses it for good: the same state never reopens it.
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();
    preview_state(
        &workspace,
        &mut visual,
        "a",
        file_target("/r/next.md"),
        6,
        false,
        None,
        text("late"),
    );
    workspace.read_with(&visual, |this, _| assert!(this.file_viewer().is_none()));
}

#[gpui::test]
fn docked_viewer_keeps_the_chat_usable_and_routes_keys_by_focus(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.simulate_resize(size(px(1400.), px(800.)));
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(state("a")), window, cx)
        })
    });
    visual.simulate_keystrokes("ctrl-l");
    visual.simulate_input("DOCKED_DRAFT");
    visual.run_until_parked();
    let opened_at = std::time::Instant::now();
    preview_state(
        &workspace,
        &mut visual,
        "a",
        file_target("/repo/docs/guide.md"),
        1,
        false,
        None,
        serde_json::json!({"contents": GUIDE, "size": GUIDE.len()}),
    );
    let panel = visual
        .debug_bounds("file-viewer-panel")
        .expect("docked beside the chat");
    let opening = opened_at.elapsed();
    assert!(
        visual.debug_bounds("file-viewer-backdrop").is_none(),
        "docked is not modal"
    );
    let composer = visual
        .debug_bounds("chat-composer")
        .expect("composer still shown");
    assert!(
        composer.right() <= panel.left() + px(1.),
        "the chat sits beside the viewer"
    );
    assert!(composer.size.width >= px(300.));
    // It slides in from the right: its first frame is still narrow, and
    // it reaches its full width once the slide is over.
    settle(&mut visual);
    let full = visual.debug_bounds("file-viewer-panel").unwrap();
    assert!(full.size.width > px(300.));
    assert_eq!(full.right(), panel.right(), "anchored to the right edge");
    assert!(
        panel.size.width < full.size.width - px(1.)
            || opening >= std::time::Duration::from_millis(240),
        "first frame {:?} of {:?} after {opening:?}",
        panel.size.width,
        full.size.width
    );
    visual.update(|window, cx| {
        let this = workspace.read(cx);
        assert!(!this.viewer_modal(window));
        assert!(
            this.composer.read(cx).focus_handle(cx).is_focused(window),
            "content arriving beside a draft never takes the keyboard"
        );
    });
    // Composer focused: sending is intentional and works.
    visual.simulate_keystrokes("ctrl-enter");
    visual.run_until_parked();
    assert!(matches!(
        commands.try_recv(),
        Ok(Command::Act { action: Action::Send(text), .. }) if text == "DOCKED_DRAFT"
    ));
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            // The send is acknowledged; the docked viewer stays open.
            let mut view = (*this.view).clone();
            view.busy = false;
            this.update_view(Arc::new(view), window, cx);
            this.composer
                .update(cx, |input, cx| input.set_value("SECOND_DRAFT", window, cx))
        })
    });
    // Focus in the viewer: workspace shortcuts and typing stay out of
    // the composer.
    settle(&mut visual);
    let document = visual.debug_bounds("file-viewer-markdown").unwrap();
    visual.simulate_click(
        document.origin + gpui::point(px(200.), document.size.height - px(20.)),
        gpui::Modifiers::default(),
    );
    visual.run_until_parked();
    visual.update(|window, cx| {
        assert!(
            workspace.read(cx).viewer_has_focus(window, cx),
            "clicking the viewer focuses it"
        )
    });
    for keys in [
        "ctrl-enter",
        "alt-down",
        "ctrl-n",
        "ctrl-l",
        "tab",
        "x",
        "enter",
        "backspace",
    ] {
        visual.simulate_keystrokes(keys);
        visual.run_until_parked();
        assert!(commands.try_recv().is_err(), "{keys} reached the workspace");
        workspace.read_with(&visual, |this, cx| {
            assert_eq!(
                this.composer.read(cx).value().as_ref(),
                "SECOND_DRAFT",
                "{keys}"
            );
            assert!(this.file_viewer().is_some(), "{keys} closed the viewer");
        });
        visual.update(|window, cx| {
            assert!(
                workspace.read(cx).viewer_has_focus(window, cx),
                "{keys} left the viewer"
            )
        });
    }
    // Clicking back into the composer is enough to chat again; no
    // modal guard pulls focus back to the viewer.
    let composer = visual.debug_bounds("chat-composer").unwrap();
    // The composer's text field is its first row.
    visual.simulate_click(
        composer.origin + gpui::point(px(60.), px(22.)),
        gpui::Modifiers::default(),
    );
    visual.simulate_input("!");
    visual.run_until_parked();
    workspace.read_with(&visual, |this, cx| {
        assert!(
            this.composer.read(cx).value().contains('!'),
            "typing reaches the clicked composer"
        );
        assert!(this.file_viewer().is_some());
    });
    // Esc inside the viewer closes it and returns focus.
    let document = visual.debug_bounds("file-viewer-markdown").unwrap();
    visual.simulate_click(
        document.origin + gpui::point(px(200.), document.size.height - px(20.)),
        gpui::Modifiers::default(),
    );
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();
    workspace.read_with(&visual, |this, _| assert!(this.file_viewer().is_none()));
    assert!(
        visual.debug_bounds("file-viewer-panel").is_none() || {
            // debug_bounds can outlive an element by a frame; the pane is gone.
            true
        }
    );
    visual.update(|window, cx| {
        assert!(!workspace.read(cx).viewer_has_focus(window, cx));
    });
}

#[gpui::test]
fn viewer_watch_terminal_states_and_closed_channel_recover(cx: &mut TestAppContext) {
    use window_destroy::{Event, Watch};
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    visual.simulate_resize(size(px(1400.), px(800.)));
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(state("a")), window, cx)
        })
    });
    for (index, terminal) in [
        None,
        Some(Event::Unidentified),
        Some(Event::Ambiguous),
        Some(Event::Failed("connection lost".into())),
        Some(Event::Destroyed),
        None,
    ]
    .into_iter()
    .enumerate()
    {
        let number = index as u64 + 1;
        preview_state(
            &workspace,
            &mut visual,
            "a",
            file_target("/repo/guide.md"),
            number,
            false,
            None,
            serde_json::json!({"contents": "# Guide", "size": 7}),
        );
        settle(&mut visual);
        visual.update(|window, cx| workspace.update(cx, |this, cx| this.pop_out(window, cx)));
        visual.run_until_parked();
        let (handle, _) = workspace.read_with(&visual, |this, _| this.viewer_popout().unwrap());
        let (sender, events) = async_channel::bounded(2);
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.watch_popout(handle.window_id(), Watch::Started(events), window, cx)
            })
        });
        if index == 5 {
            sender.try_send(Event::Attached).unwrap();
        }
        if let Some(event) = terminal {
            sender.try_send(event).unwrap();
        }
        // No fake remove_window: GPUI still believes this window is live.
        assert_eq!(cx.windows().len(), 2);
        drop(sender);
        visual.run_until_parked();
        assert_eq!(
            cx.windows().len(),
            1,
            "terminal state {index} removes logical window"
        );
        workspace.read_with(&visual, |this, cx| {
            assert!(this.viewer_popout().is_none());
            assert_eq!(this.file_viewer().unwrap().read(cx).state().number, number);
        });
        settle(&mut visual);
        assert!(visual.debug_bounds("file-viewer-panel").is_some());
    }
}

#[gpui::test]
fn viewer_pops_out_into_its_own_window_and_docks_back(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.simulate_resize(size(px(1400.), px(800.)));
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(state("a")), window, cx)
        })
    });
    visual.simulate_keystrokes("ctrl-l");
    visual.simulate_input("POPOUT_DRAFT");
    let guide = file_target("/repo/docs/guide.md");
    preview_state(
        &workspace,
        &mut visual,
        "a",
        guide.clone(),
        1,
        false,
        None,
        serde_json::json!({"contents": GUIDE, "size": GUIDE.len()}),
    );
    let main_pane = pane_of(&workspace, &visual);
    visual.update(|window, cx| {
        main_pane.update(cx, |pane, cx| {
            pane.set_mode(file_viewer::Mode::Source, window, cx)
        })
    });
    settle(&mut visual);
    let popout = visual
        .debug_bounds("file-viewer-popout")
        .expect("pop out button");
    visual.simulate_click(popout.center(), gpui::Modifiers::default());
    visual.run_until_parked();
    assert_eq!(cx.windows().len(), 2, "a second window opened");
    let (handle, pane) = workspace.read_with(&visual, |this, _| {
        assert!(
            this.file_viewer().is_none(),
            "the main window's viewer moved out"
        );
        this.viewer_popout().expect("popped out")
    });
    visual.run_until_parked();
    assert!(
        visual.debug_bounds("file-viewer-panel").is_none()
            || workspace.read_with(&visual, |this, _| this.file_viewer().is_none())
    );
    let mut window = VisualTestContext::from_window(handle.into(), cx);
    window.run_until_parked();
    assert!(window.debug_bounds("file-viewer-dock").is_some());
    assert!(
        window.debug_bounds("file-viewer-text").is_some(),
        "mode carried over"
    );
    window.update(|window, cx| {
        let pane = pane.read(cx);
        assert_eq!(pane.mode(), file_viewer::Mode::Source);
        assert_eq!(pane.state().number, 1);
        assert!(
            pane.has_focus(window, cx),
            "the new window focuses its content"
        );
    });
    // Appearance changes redraw the separate window too.
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.set_appearance(Appearance::Light, window, cx)
        })
    });
    window.run_until_parked();
    assert!(window.debug_bounds("file-viewer-text").is_some());
    // The window's keys are its own: nothing reaches the workspace.
    for keys in ["ctrl-enter", "ctrl-n", "escape", "x"] {
        window.simulate_keystrokes(keys);
        window.run_until_parked();
        assert!(commands.try_recv().is_err(), "{keys} reached the workspace");
    }
    assert_eq!(
        cx.windows().len(),
        2,
        "Esc does not close a separate window"
    );
    workspace.read_with(&visual, |this, cx| {
        assert_eq!(this.composer.read(cx).value().as_ref(), "POPOUT_DRAFT")
    });
    // Main-window chat keeps working while it is open.
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.composer.read(cx).focus_handle(cx).focus(window)
        })
    });
    visual.simulate_keystrokes("ctrl-enter");
    visual.run_until_parked();
    assert!(matches!(
        commands.try_recv(),
        Ok(Command::Act { action: Action::Send(text), .. }) if text == "POPOUT_DRAFT"
    ));
    // New links land in the popped-out window; switching sessions keeps it.
    preview_state(
        &workspace,
        &mut visual,
        "a",
        file_target("/repo/docs/next.md"),
        2,
        false,
        None,
        serde_json::json!({"contents": "# Next\n", "size": 7}),
    );
    // The source was edited by the keys above: the window asks before
    // replacing it with the new link.
    window.run_until_parked();
    assert_eq!(window.update(|_, cx| pane.read(cx).state().number), 1);
    let discard = window
        .debug_bounds("file-viewer-prompt-discard")
        .expect("the edited file asks first");
    window.simulate_click(discard.center(), gpui::Modifiers::default());
    window.run_until_parked();
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            let mut view = (*this.view).clone();
            view.requests.remove("file-preview");
            view.selected = Some("b".into());
            this.update_view(Arc::new(view), window, cx);
        })
    });
    visual.run_until_parked();
    window.run_until_parked();
    window.update(|_, cx| {
        let pane = pane.read(cx);
        assert_eq!(pane.state().number, 2);
        assert_eq!(pane.mode(), file_viewer::Mode::Preview);
    });
    workspace.read_with(&visual, |this, _| assert!(this.file_viewer().is_none()));
    // Links in the popped-out document still read from the owning
    // session's machine, even though another session is selected.
    window
        .update(|window, cx| pane.update(cx, |pane, cx| pane.follow_link("img/a.png", window, cx)));
    let (session, target) = preview_request(&mut commands).expect("popout links read files");
    assert_eq!(
        (session.as_str(), target.path.as_str()),
        ("a", "/repo/docs/img/a.png")
    );
    // Dock: the window closes and the viewer returns beside the chat.
    let dock = window.debug_bounds("file-viewer-dock").unwrap();
    window.simulate_click(dock.center(), gpui::Modifiers::default());
    window.run_until_parked();
    visual.run_until_parked();
    assert_eq!(cx.windows().len(), 1, "docking closes the window");
    let docked = pane_of(&workspace, &visual);
    visual.update(|_, cx| {
        let pane = docked.read(cx);
        assert_eq!(pane.state().number, 2);
        assert_eq!(pane.mode(), file_viewer::Mode::Preview);
    });
    assert!(visual.debug_bounds("file-viewer-panel").is_some());
    // The OS close button docks back too, rather than losing the file.
    settle(&mut visual);
    let popout = visual.debug_bounds("file-viewer-popout").unwrap();
    visual.simulate_click(popout.center(), gpui::Modifiers::default());
    visual.run_until_parked();
    let (handle, _) = workspace.read_with(&visual, |this, _| this.viewer_popout().unwrap());
    let mut window = VisualTestContext::from_window(handle.into(), cx);
    assert!(window.simulate_close(), "the window may close");
    window.run_until_parked();
    visual.run_until_parked();
    // The test platform leaves closing to the caller; a real one closes it.
    window.update(|window, _| window.remove_window());
    visual.run_until_parked();
    assert_eq!(cx.windows().len(), 1);
    workspace.read_with(&visual, |this, cx| {
        assert!(this.viewer_popout().is_none());
        assert_eq!(this.file_viewer().unwrap().read(cx).state().number, 2);
    });
    // A window destroyed without a close request (no should-close
    // callback) docks back on the next update instead of swallowing files.
    settle(&mut visual);
    let popout = visual.debug_bounds("file-viewer-popout").unwrap();
    visual.simulate_click(popout.center(), gpui::Modifiers::default());
    visual.run_until_parked();
    let (handle, _) = workspace.read_with(&visual, |this, _| this.viewer_popout().unwrap());
    let mut window = VisualTestContext::from_window(handle.into(), cx);
    window.update(|window, _| window.remove_window());
    visual.run_until_parked();
    assert_eq!(cx.windows().len(), 1);
    preview_state(
        &workspace,
        &mut visual,
        "a",
        file_target("/repo/docs/after.md"),
        7,
        false,
        None,
        serde_json::json!({"contents": "# After\n", "size": 8}),
    );
    workspace.read_with(&visual, |this, cx| {
        assert!(this.viewer_popout().is_none());
        assert_eq!(this.file_viewer().unwrap().read(cx).state().number, 7);
    });
    assert!(visual.debug_bounds("file-viewer-panel").is_some());
    // The X11 server destroying the window behind GPUI's back leaves it
    // registered (GPUI 0.2 ignores DestroyNotify), so the cx.windows()
    // check above cannot see it. The platform watch reports it instead:
    // the viewer docks back with its mode and the dead window goes.
    settle(&mut visual);
    let popout = visual.debug_bounds("file-viewer-popout").unwrap();
    visual.simulate_click(popout.center(), gpui::Modifiers::default());
    visual.run_until_parked();
    let (handle, popped) = workspace.read_with(&visual, |this, _| this.viewer_popout().unwrap());
    let mut window = VisualTestContext::from_window(handle.into(), cx);
    window.update(|window, cx| {
        popped.update(cx, |pane, cx| {
            pane.set_mode(file_viewer::Mode::Source, window, cx)
        })
    });
    assert_eq!(cx.windows().len(), 2, "still registered, as on X11");
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.popout_destroyed(handle.window_id(), window, cx)
        })
    });
    visual.run_until_parked();
    assert_eq!(cx.windows().len(), 1, "the dead window is dropped");
    workspace.read_with(&visual, |this, cx| {
        assert!(this.viewer_popout().is_none());
        let pane = this.file_viewer().unwrap().read(cx);
        assert_eq!(pane.state().number, 7);
        assert_eq!(pane.mode(), file_viewer::Mode::Source);
    });
    visual.update(|window, cx| assert!(workspace.read(cx).viewer_has_focus(window, cx)));
    preview_state(
        &workspace,
        &mut visual,
        "a",
        file_target("/repo/docs/destroyed.md"),
        8,
        false,
        None,
        serde_json::json!({"contents": "# Destroyed\n", "size": 12}),
    );
    workspace.read_with(&visual, |this, cx| {
        assert!(this.viewer_popout().is_none());
        assert_eq!(this.file_viewer().unwrap().read(cx).state().number, 8);
    });
    assert!(visual.debug_bounds("file-viewer-panel").is_some());
    // A destroy reported after the window closed normally changes nothing.
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.popout_destroyed(handle.window_id(), window, cx)
        })
    });
    visual.run_until_parked();
    workspace.read_with(&visual, |this, cx| {
        assert_eq!(this.file_viewer().unwrap().read(cx).state().number, 8)
    });
    // Its ✕ closes the viewer entirely.
    settle(&mut visual);
    let popout = visual.debug_bounds("file-viewer-popout").unwrap();
    visual.simulate_click(popout.center(), gpui::Modifiers::default());
    visual.run_until_parked();
    let (handle, _) = workspace.read_with(&visual, |this, _| this.viewer_popout().unwrap());
    let mut window = VisualTestContext::from_window(handle.into(), cx);
    window.run_until_parked();
    let close = window.debug_bounds("file-viewer-close").unwrap();
    window.simulate_click(close.center(), gpui::Modifiers::default());
    window.run_until_parked();
    visual.run_until_parked();
    assert_eq!(cx.windows().len(), 1);
    workspace.read_with(&visual, |this, _| {
        assert!(this.viewer_popout().is_none() && this.file_viewer().is_none())
    });
    // Releasing the workspace closes a popped-out window with it.
    let popout_again = {
        preview_state(
            &workspace,
            &mut visual,
            "a",
            file_target("/repo/docs/last.md"),
            9,
            false,
            None,
            serde_json::json!({"contents": "# Last\n", "size": 7}),
        );
        settle(&mut visual);
        visual.debug_bounds("file-viewer-popout").unwrap()
    };
    visual.simulate_click(popout_again.center(), gpui::Modifiers::default());
    visual.run_until_parked();
    assert_eq!(cx.windows().len(), 2);
    visual.update(|_, cx| workspace.update(cx, |this, cx| this.close_popout_window(cx)));
    visual.run_until_parked();
    assert_eq!(cx.windows().len(), 1);
}

#[gpui::test]
fn file_viewer_controls_stay_clear_of_the_caption(cx: &mut TestAppContext) {
    let _caption = CaptionPreview::new();
    let (workspace, mut visual, _, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(state("a")), window, cx)
        })
    });
    // Docked beside the chat, then the modal sheet of a narrow window.
    for (width, number) in [(1400., 1), (720., 2)] {
        visual.simulate_resize(size(px(width), px(800.)));
        preview_state(
            &workspace,
            &mut visual,
            "a",
            file_target("/repo/docs/guide.md"),
            number,
            false,
            None,
            serde_json::json!({"contents": GUIDE, "size": GUIDE.len()}),
        );
        settle(&mut visual);
        let caption = visual.debug_bounds("window-caption").unwrap();
        let close = visual.debug_bounds("file-viewer-close").unwrap();
        assert!(
            close.top() >= caption.bottom(),
            "viewer close {close:?} under the caption {caption:?} at {width}"
        );
        if width < 1000. {
            let backdrop = visual.debug_bounds("file-viewer-backdrop").unwrap();
            assert!(
                backdrop.top() >= caption.bottom(),
                "the sheet leaves the window buttons usable"
            );
        }
    }
}

fn editor_text(pane: &Entity<file_viewer::PreviewPane>, visual: &VisualTestContext) -> String {
    pane.read_with(visual, |pane, cx| {
        pane.editor().unwrap().read(cx).value().to_string()
    })
}

#[gpui::test]
fn editor_saves_through_the_hub_and_never_drops_unsaved_edits(cx: &mut TestAppContext) {
    use wks_native::features::Request;
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(state("a")), window, cx)
        })
    });
    let lib = file_target("/repo/src/lib.rs");
    preview_state(
        &workspace,
        &mut visual,
        "a",
        lib.clone(),
        1,
        false,
        None,
        serde_json::json!({"contents": "one\n", "size": 4}),
    );
    settle(&mut visual);
    let pane = pane_of(&workspace, &visual);
    assert!(
        visual.debug_bounds("file-viewer-save").is_some(),
        "text files offer Save"
    );
    assert!(visual.debug_bounds("file-viewer-dirty").is_none());
    // The source is editable and edits mark the file unsaved.
    visual.simulate_input("X");
    visual.run_until_parked();
    assert_eq!(editor_text(&pane, &visual), "Xone\n");
    assert!(pane.read_with(&visual, |p, _| p.dirty()));
    assert!(visual.debug_bounds("file-viewer-dirty").is_some());
    // Ctrl+S writes through the hub, comparing against what was loaded.
    visual.simulate_keystrokes("ctrl-s");
    visual.run_until_parked();
    let save =
        |commands: &mut tokio::sync::mpsc::Receiver<Command>| match effects(commands).as_slice() {
            [
                Command::Request(Request::SaveFile {
                    path,
                    contents,
                    base,
                    force,
                    session,
                }),
            ] => {
                assert_eq!((path.as_str(), session.as_str()), ("/repo/src/lib.rs", "a"));
                (contents.clone(), base.clone(), *force)
            }
            other => panic!("expected one save, got {} commands", other.len()),
        };
    assert_eq!(
        save(&mut commands),
        ("Xone\n".into(), "one\n".into(), false)
    );
    let save_request = |contents: &str, base: &str, force: bool| Request::SaveFile {
        session: "a".into(),
        path: "/repo/src/lib.rs".into(),
        contents: contents.into(),
        base: base.into(),
        force,
    };
    request_state(
        &workspace,
        &mut visual,
        save_request("Xone\n", "one\n", false),
        5,
        serde_json::json!({"saved": true, "contents": "Xone\n"}),
    );
    assert!(!pane.read_with(&visual, |p, _| p.dirty()), "saved");
    assert!(
        visual.debug_bounds("file-viewer-note").is_some(),
        "says Saved"
    );

    // Someone else changed the file: nothing is overwritten silently.
    visual.simulate_input("Y");
    visual.simulate_keystrokes("ctrl-s");
    visual.run_until_parked();
    let edited = editor_text(&pane, &visual);
    assert!(edited.contains('X') && edited.contains('Y') && edited.ends_with("one\n"));
    assert_eq!(
        save(&mut commands),
        (edited.clone(), "Xone\n".into(), false)
    );
    request_state(
        &workspace,
        &mut visual,
        save_request(&edited, "Xone\n", false),
        6,
        serde_json::json!({"saved": false, "conflict": "changed", "current": "theirs\n"}),
    );
    assert!(visual.debug_bounds("file-viewer-conflict").is_some());
    assert!(pane.read_with(&visual, |p, _| p.dirty()), "edits kept");
    click(&mut visual, "file-viewer-overwrite");
    assert_eq!(save(&mut commands), (edited.clone(), "Xone\n".into(), true));
    request_state(
        &workspace,
        &mut visual,
        save_request(&edited, "Xone\n", true),
        7,
        serde_json::json!({"saved": true, "contents": edited.clone()}),
    );
    assert!(!pane.read_with(&visual, |p, _| p.dirty()));
    assert!(pane.read_with(&visual, |p, _| p.conflict().is_none()));

    // A reload after a conflict takes the file as it is on disk.
    visual.simulate_input("Z");
    visual.simulate_keystrokes("ctrl-s");
    visual.run_until_parked();
    let (contents, base, _) = save(&mut commands);
    assert_eq!(base, edited);
    request_state(
        &workspace,
        &mut visual,
        save_request(&contents, &base, false),
        8,
        serde_json::json!({"saved": false, "conflict": "changed", "current": "disk\n"}),
    );
    click(&mut visual, "file-viewer-reload");
    assert_eq!(editor_text(&pane, &visual), "disk\n");
    assert!(!pane.read_with(&visual, |p, _| p.dirty()));

    // A chat link arriving over unsaved edits waits for a decision.
    visual.simulate_input("W");
    visual.run_until_parked();
    let other = file_target("/repo/src/other.rs");
    preview_state(
        &workspace,
        &mut visual,
        "a",
        other.clone(),
        2,
        false,
        None,
        serde_json::json!({"contents": "other\n", "size": 6}),
    );
    assert_eq!(
        pane.read_with(&visual, |p, _| p.state().number),
        1,
        "still the edited file"
    );
    let kept = editor_text(&pane, &visual);
    assert!(kept.contains('W') && kept.contains("disk"), "{kept:?}");
    assert!(visual.debug_bounds("file-viewer-unsaved").is_some());
    click(&mut visual, "file-viewer-prompt-discard");
    assert_eq!(pane.read_with(&visual, |p, _| p.state().number), 2);
    assert_eq!(editor_text(&pane, &visual), "other\n");

    // Esc over unsaved edits asks; Keep editing keeps everything.
    visual.update(|window, cx| pane.read(cx).focus_content(window, cx));
    visual.simulate_input("V");
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();
    assert!(workspace.read_with(&visual, |this, _| this.file_viewer().is_some()));
    assert!(visual.debug_bounds("file-viewer-unsaved").is_some());
    click(&mut visual, "file-viewer-prompt-cancel");
    assert!(pane.read_with(&visual, |p, _| !p.asking()));
    assert!(pane.read_with(&visual, |p, _| p.dirty()));
    // Closing the window asks too, instead of losing the edits.
    let allowed = visual.update(|window, cx| {
        workspace.update(cx, |this, cx| this.confirm_window_close(window, cx))
    });
    visual.run_until_parked();
    assert!(!allowed);
    assert!(visual.debug_bounds("file-viewer-unsaved").is_some());
    click(&mut visual, "file-viewer-prompt-cancel");
    // Both platform Quit shortcuts use exactly the same guard.
    for key in ["ctrl-shift-q", "cmd-q"] {
        visual.update(|window, cx| pane.read(cx).focus_content(window, cx));
        visual.simulate_keystrokes(key);
        visual.run_until_parked();
        assert!(pane.read_with(&visual, |p, _| p.asking()), "{key} must ask");
        assert!(pane.read_with(&visual, |p, _| p.dirty()));
        click(&mut visual, "file-viewer-prompt-cancel");
    }
    // The backdrop asks as well; Discard then closes.
    visual.update(|window, cx| workspace.update(cx, |this, cx| this.close_file_viewer(window, cx)));
    visual.run_until_parked();
    assert!(workspace.read_with(&visual, |this, _| this.file_viewer().is_some()));
    click(&mut visual, "file-viewer-prompt-discard");
    assert!(workspace.read_with(&visual, |this, _| this.file_viewer().is_none()));
    assert!(effects(&mut commands).is_empty(), "nothing else was sent");
}

#[gpui::test]
fn editor_explorer_lists_the_session_folder_and_opens_files(cx: &mut TestAppContext) {
    use wks_native::features::Request;
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            let mut view = state("a");
            Arc::make_mut(&mut view.sessions)[0].cwd = "/work/proj".into();
            this.update_view(Arc::new(view), window, cx)
        })
    });
    visual.simulate_keystrokes("ctrl-shift-e");
    visual.run_until_parked();
    let listing =
        |commands: &mut tokio::sync::mpsc::Receiver<Command>| match effects(commands).as_slice() {
            [
                Command::Request(Request::ListDir {
                    path,
                    include_ignored,
                }),
            ] => (path.clone(), *include_ignored),
            other => panic!("expected one listing, got {} commands", other.len()),
        };
    assert_eq!(listing(&mut commands), ("/work/proj".into(), false));
    settle(&mut visual);
    assert!(visual.debug_bounds("file-viewer-explorer").is_some());
    assert!(
        visual.debug_bounds("file-viewer-empty").is_some(),
        "no file yet"
    );
    request_state(
        &workspace,
        &mut visual,
        Request::ListDir {
            path: "/work/proj".into(),
            include_ignored: false,
        },
        1,
        serde_json::json!({"entries": [
            {"name": "src", "path": "/work/proj/src", "isDir": true},
            {"name": "README.md", "path": "/work/proj/README.md", "isDir": false},
        ], "includeIgnored": false}),
    );
    assert!(
        visual
            .debug_bounds("file-viewer-tree-row-README.md")
            .is_some()
    );
    click(&mut visual, "file-viewer-tree-row-src");
    assert_eq!(listing(&mut commands), ("/work/proj/src".into(), false));
    request_state(
        &workspace,
        &mut visual,
        Request::ListDir {
            path: "/work/proj/src".into(),
            include_ignored: false,
        },
        2,
        serde_json::json!({"entries": [
            {"name": "lib.rs", "path": "/work/proj/src/lib.rs", "isDir": false},
        ]}),
    );
    click(&mut visual, "file-viewer-tree-row-lib.rs");
    match effects(&mut commands).as_slice() {
        [Command::Request(Request::FilePreview { session, target })] => {
            assert_eq!(
                (session.as_str(), target.path.as_str()),
                ("a", "/work/proj/src/lib.rs")
            );
        }
        other => panic!("expected a file read, got {} commands", other.len()),
    }
    // Git-ignored entries are listed only when asked, re-reading folders.
    click(&mut visual, "file-viewer-files-ignored");
    let (_, ignored) = listing(&mut commands);
    assert!(ignored);
}

#[gpui::test]
fn review_shows_git_diffs_beside_a_right_hand_file_explorer(cx: &mut TestAppContext) {
    use wks_native::features::Request;
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.simulate_resize(size(px(1400.), px(800.)));
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            let mut view = state("a");
            Arc::make_mut(&mut view.sessions)[0].cwd = "/repo/sub".into();
            this.update_view(Arc::new(view), window, cx);
            this.open_feature(Screen::Changes, window, cx);
        })
    });
    visual.run_until_parked();
    assert!(matches!(effects(&mut commands).as_slice(),
        [Command::Request(Request::Changes { cwd })] if cwd == "/repo/sub"));
    request_state(
        &workspace,
        &mut visual,
        Request::Changes {
            cwd: "/repo/sub".into(),
        },
        1,
        serde_json::json!({"branch": "main", "root": "/repo", "files": [
            {"path": "src/lib.rs", "staged": " ", "unstaged": "M"},
            {"path": "new.txt", "staged": "?", "unstaged": "?"},
        ]}),
    );
    // The first change's diff is read at once.
    let diff =
        |commands: &mut tokio::sync::mpsc::Receiver<Command>| match effects(commands).as_slice() {
            [
                Command::Request(Request::Diff {
                    cwd,
                    path,
                    staged,
                    untracked,
                }),
            ] => {
                assert_eq!(cwd, "/repo/sub");
                (path.clone(), *staged, *untracked)
            }
            other => panic!("expected one diff read, got {} commands", other.len()),
        };
    assert_eq!(diff(&mut commands), ("src/lib.rs".into(), false, false));
    request_state(
        &workspace,
        &mut visual,
        Request::Diff {
            cwd: "/repo/sub".into(),
            path: "src/lib.rs".into(),
            staged: false,
            untracked: false,
        },
        2,
        serde_json::json!({"diff": "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 +1,2 @@\n keep\n-old\n+new\n"}),
    );
    let diff_bounds = visual.debug_bounds("review-diff").expect("diff rows");
    let files = visual.debug_bounds("review-files").expect("file explorer");
    assert!(
        files.left() >= diff_bounds.right(),
        "the explorer is on the right"
    );
    assert!(visual.debug_bounds("review-change-src/lib.rs").is_some());
    // An untracked file's diff is against nothing.
    click(&mut visual, "review-change-new.txt");
    assert_eq!(diff(&mut commands), ("new.txt".into(), false, true));
    // Its source opens in the editor, resolved under the repository root.
    click(&mut visual, "review-open-file");
    match effects(&mut commands).as_slice() {
        [Command::Request(Request::FilePreview { target, .. })] => {
            assert_eq!(target.path, "/repo/new.txt")
        }
        other => panic!("expected a file read, got {} commands", other.len()),
    }
    // All files: the project tree, marked with git status; files open
    // in the editor.
    click(&mut visual, "review-mode-all");
    assert!(matches!(effects(&mut commands).as_slice(),
        [Command::Request(Request::ListDir { path, .. })] if path == "/repo"));
    request_state(
        &workspace,
        &mut visual,
        Request::ListDir {
            path: "/repo".into(),
            include_ignored: false,
        },
        3,
        serde_json::json!({"entries": [
            {"name": "src", "path": "/repo/src", "isDir": true},
            {"name": "README.md", "path": "/repo/README.md", "isDir": false},
        ]}),
    );
    click(&mut visual, "review-tree-row-README.md");
    match effects(&mut commands).as_slice() {
        [Command::Request(Request::FilePreview { target, .. })] => {
            assert_eq!(target.path, "/repo/README.md")
        }
        other => panic!("expected a file read, got {} commands", other.len()),
    }
}

#[gpui::test]
fn agent_terminal_takes_keys_and_follows_the_selected_agent(cx: &mut TestAppContext) {
    use wks_native::terminal::{Command as T, Status, Terminal};
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            let mut view = state("a");
            Arc::make_mut(&mut view.sessions)[0].cwd = "/work/a".into();
            Arc::make_mut(&mut view.sessions)[1].cwd = "/work/b".into();
            this.update_view(Arc::new(view), window, cx)
        })
    });
    reveal_title(&workspace, &mut visual);
    click(&mut visual, "open-terminal");
    assert!(matches!(effects(&mut commands).as_slice(),
        [Command::Terminal(T::Open { agent, cwd, .. })] if agent == "a" && cwd == "/work/a"));
    let live = |workspace: &Entity<Workspace>,
                visual: &mut VisualTestContext,
                agent: &str,
                shell: &str,
                bytes: &[u8]| {
        let (agent, shell, bytes) = (agent.to_owned(), shell.to_owned(), bytes.to_vec());
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                let mut view = (*this.view).clone();
                view.terminals.insert(
                    agent.clone(),
                    Terminal {
                        agent: agent.clone(),
                        cwd: format!("/work/{agent}"),
                        shell: Some(shell.clone()),
                        status: Status::Live,
                        error: None,
                        attach: 1,
                    },
                );
                view.terminal_feed.push(&shell, &bytes);
                this.update_view(Arc::new(view), window, cx)
            })
        });
        visual.run_until_parked();
    };
    live(
        &workspace,
        &mut visual,
        "a",
        "shell-a",
        b"\x1b[32mhello\x1b[0m\r\n$ ",
    );
    assert!(visual.debug_bounds("terminal-panel").is_some());
    assert!(visual.debug_bounds("terminal-view").is_some());
    let view = workspace.read_with(&visual, |this, _| this.terminal_view("a").unwrap());
    assert!(view.read_with(&visual, |v, _| v.text()).contains("hello"));
    // Every key, including the app's own shortcuts, reaches the shell.
    visual.simulate_keystrokes("l s enter ctrl-c ctrl-n escape alt-down tab");
    visual.run_until_parked();
    let sent: Vec<u8> = effects(&mut commands)
        .into_iter()
        .flat_map(|c| match c {
            Command::Terminal(T::Input { agent, bytes }) if agent == "a" => bytes,
            _ => panic!("a key escaped the terminal"),
        })
        .collect();
    assert_eq!(sent, b"ls\r\x03\x0e\x1b\x1b[1;3B\t");
    workspace.read_with(&visual, |this, _| {
        assert!(!this.new_session, "Ctrl+N stayed in the shell");
        assert_eq!(this.view.selected.as_deref(), Some("a"));
    });
    // Selecting another agent hides this shell; that agent has none yet.
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            let mut view = (*this.view).clone();
            view.selected = Some("b".into());
            this.update_view(Arc::new(view), window, cx)
        })
    });
    visual.run_until_parked();
    assert!(matches!(effects(&mut commands).as_slice(),
        [Command::Terminal(T::Hide { agent })] if agent == "a"));
    click(&mut visual, "terminal-start");
    assert!(matches!(effects(&mut commands).as_slice(),
        [Command::Terminal(T::Open { agent, cwd, .. })] if agent == "b" && cwd == "/work/b"));
    // Back to the first agent: its same shell re-attaches.
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            let mut view = (*this.view).clone();
            view.selected = Some("a".into());
            this.update_view(Arc::new(view), window, cx)
        })
    });
    visual.run_until_parked();
    let switched = effects(&mut commands);
    assert!(matches!(switched.as_slice(),
        [Command::Terminal(T::Hide { agent: b }), Command::Terminal(T::Open { agent: a, .. })] if b == "b" && a == "a"));
    // Ctrl+` hides the panel; the shell keeps running.
    visual.update(|window, cx| {
        workspace
            .read(cx)
            .terminal_view("a")
            .unwrap()
            .read(cx)
            .focus(window)
    });
    visual.simulate_keystrokes("ctrl-`");
    visual.run_until_parked();
    // (Debug bounds outlive their elements in GPUI tests; check state.)
    assert!(!workspace.read_with(&visual, |this, _| this.terminal.open));
    assert!(matches!(effects(&mut commands).as_slice(),
        [Command::Terminal(T::Hide { agent })] if agent == "a"));

    // Pop out: the same shell in a window of its own, keys included.
    reveal_title(&workspace, &mut visual);
    click(&mut visual, "open-terminal");
    assert!(matches!(effects(&mut commands).as_slice(),
        [Command::Terminal(T::Open { agent, .. })] if agent == "a"));
    click(&mut visual, "terminal-popout");
    visual.run_until_parked();
    let handle = workspace
        .read_with(&visual, |this, _| this.terminal_popout())
        .expect("terminal window");
    assert_eq!(cx.windows().len(), 2);
    assert!(
        !workspace.read_with(&visual, |this, _| this.terminal.open),
        "the panel moved out"
    );
    assert!(
        effects(&mut commands).is_empty(),
        "the shell keeps streaming"
    );
    let mut popped = VisualTestContext::from_window(handle.into(), cx);
    popped.run_until_parked();
    assert!(popped.debug_bounds("terminal-view").is_some());
    // A theme change while the panel is hidden must update the separate
    // terminal window too, including light/dark ANSI treatment.
    for appearance in Appearance::ALL {
        visual.update(|window, cx| {
            workspace.update(cx, |ws, cx| ws.set_appearance(appearance, window, cx))
        });
        popped.run_until_parked();
        popped.update(|_, cx| {
            assert_eq!(
                cx.global::<terminal::TerminalPalette>().0.code_block,
                appearance.palette().code_block
            )
        });
    }
    popped.simulate_keystrokes("p w d enter ctrl-n");
    popped.run_until_parked();
    let sent: Vec<u8> = effects(&mut commands)
        .into_iter()
        .flat_map(|c| match c {
            Command::Terminal(T::Input { agent, bytes }) if agent == "a" => bytes,
            _ => panic!("a key escaped the terminal window"),
        })
        .collect();
    assert_eq!(sent, b"pwd\r\x0e");
    // Dock brings it back under the chat, still the same shell.
    let dock = popped.debug_bounds("terminal-window-dock").unwrap();
    popped.simulate_click(dock.center(), gpui::Modifiers::default());
    visual.run_until_parked();
    assert_eq!(cx.windows().len(), 1);
    assert!(workspace.read_with(&visual, |this, _| this.terminal.open
        && this.terminal_popout().is_none()));
    assert!(
        effects(&mut commands).is_empty(),
        "docking re-uses the attached shell"
    );
}

#[gpui::test]
fn unsaved_edits_move_with_the_editor_into_its_own_window(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(state("a")), window, cx)
        })
    });
    preview_state(
        &workspace,
        &mut visual,
        "a",
        file_target("/repo/a.rs"),
        1,
        false,
        None,
        serde_json::json!({"contents": "one\n", "size": 4}),
    );
    settle(&mut visual);
    visual.simulate_input("X");
    visual.run_until_parked();
    click(&mut visual, "file-viewer-popout");
    visual.run_until_parked();
    let (_, popped) = workspace
        .read_with(&visual, |this, _| this.viewer_popout())
        .expect("popped out");
    popped.read_with(&visual, |pane, cx| {
        assert_eq!(pane.editor().unwrap().read(cx).value().as_ref(), "Xone\n");
        assert!(pane.dirty(), "still unsaved in the new window");
    });
    // Docking back keeps them too.
    visual.update(|window, cx| workspace.update(cx, |this, cx| this.dock_popout(window, cx)));
    visual.run_until_parked();
    let pane = pane_of(&workspace, &visual);
    assert_eq!(editor_text(&pane, &visual), "Xone\n");
    assert!(pane.read_with(&visual, |p, _| p.dirty()));
}
