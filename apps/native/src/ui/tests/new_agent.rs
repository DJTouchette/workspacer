//! Starting an agent: the new-agent form, models, effort, access and resume.
use super::*;

#[gpui::test]
fn new_session_click_leaves_a_pinned_view_and_creates_the_selected_session(
    cx: &mut TestAppContext,
) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.demo = false;
            this.open_session(Some("a".into()));
            let mut initial = state("a");
            Arc::make_mut(&mut initial.sessions)[0].cwd = "/work/project".into();
            this.update_view(Arc::new(initial), window, cx);
        })
    });
    visual.run_until_parked();
    let button = visual.debug_bounds("new-session-button").unwrap();
    visual.simulate_click(button.center(), gpui::Modifiers::default());
    visual.run_until_parked();
    workspace.read_with(&visual, |this, _| {
        assert!(this.new_session);
        assert!(this.requested_session.is_none());
        assert_eq!(this.projects.cwd.as_str(), "/work/project");
    });
    while let Ok(command) = commands.try_recv() {
        assert!(
            matches!(command, Command::Refresh) || project_read(&command),
            "opening the form must not launch anything"
        );
    }
    visual.simulate_keystrokes("ctrl-enter");
    let Command::Create(request) = commands.try_recv().expect("create from the pinned window")
    else {
        panic!("expected create")
    };
    assert_eq!(request.cwd, "/work/project");
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            let mut created = state("b");
            created.spawn_receipt = Some(wks_native::controller::SpawnReceipt {
                number: 1,
                session: Some("b".into()),
                error: None,
                unsent_message: None,
            });
            this.update_view(Arc::new(created), window, cx);
            assert_eq!(this.view.selected.as_deref(), Some("b"));
            assert!(!this.new_session);
            assert!(this.requested_session.is_none());
        })
    });
    // The acknowledged launch marks its project recently used on the hub,
    // exactly once, and does nothing else.
    let Ok(Command::Request(wks_native::features::Request::TouchProject { path, at })) =
        commands.try_recv()
    else {
        panic!("an acknowledged launch records its project")
    };
    assert_eq!(path, "/work/project");
    assert!(at > 0);
    assert!(commands.try_recv().is_err());
    // A failed launch keeps nothing: a later receipt for a launch this
    // window did not start (another window's) records no project.
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.spawn_pending = true;
            this.launched_cwd = "/work/failed".into();
            let mut failed = state("b");
            failed.spawn_receipt = Some(wks_native::controller::SpawnReceipt {
                number: 2,
                session: None,
                error: Some("provider executable not found".into()),
                unsent_message: None,
            });
            this.update_view(Arc::new(failed), window, cx);
            let mut other = state("c");
            other.spawn_receipt = Some(wks_native::controller::SpawnReceipt {
                number: 3,
                session: Some("c".into()),
                error: None,
                unsent_message: None,
            });
            this.update_view(Arc::new(other), window, cx);
        })
    });
    assert!(
        std::iter::from_fn(|| commands.try_recv().ok()).all(|c| !matches!(
            c,
            Command::Request(wks_native::features::Request::TouchProject { .. })
        )),
        "another window's launch never records this window's failed folder"
    );
}

#[gpui::test]
fn guided_launch_keeps_options_and_start_action_accessible(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.simulate_resize(size(px(1000.), px(1100.)));
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.demo = false;
            let mut view = state("a");
            Arc::make_mut(&mut view.sessions)[0].cwd = "/work/alpha".into();
            Arc::make_mut(&mut view.sessions)[1].cwd = "/work/beta".into();
            this.update_view(Arc::new(view), window, cx);
            this.show_new_session(window, cx);
            this.prompt
                .update(cx, |input, cx| input.set_value("Fix the tests", window, cx));
        });
    });
    visual.run_until_parked();
    assert!(visual.debug_bounds("launch-details").is_none());
    let customize = visual.debug_bounds("launch-customize").unwrap();
    visual.simulate_click(customize.center(), gpui::Modifiers::default());
    visual.run_until_parked();
    assert!(visual.debug_bounds("launch-details").is_some());
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.label
                .update(cx, |input, cx| input.set_value("Test repair", window, cx));
            this.permission = Permission::Plan;
        });
    });
    visual.simulate_click(customize.center(), gpui::Modifiers::default());
    let provider = visual.debug_bounds("launch-provider-codex").unwrap();
    visual.simulate_click(provider.center(), gpui::Modifiers::default());
    // The open conversation's folder is preselected; Change swaps it.
    workspace.read_with(&visual, |this, _| {
        assert_eq!(this.projects.cwd, "/work/alpha")
    });
    visual.run_until_parked();
    let change = visual.debug_bounds("launch-project-change").unwrap();
    visual.simulate_click(change.center(), gpui::Modifiers::default());
    visual.run_until_parked();
    let project = visual.debug_bounds("launch-project-row-1").unwrap();
    visual.simulate_click(project.center(), gpui::Modifiers::default());
    workspace.read_with(&visual, |this, cx| {
        assert!(!this.launch_details_open);
        assert_eq!(this.permission, Permission::Ask);
        assert_eq!(this.provider, "codex");
        assert_eq!(this.projects.cwd.as_str(), "/work/beta");
        assert_eq!(this.label.read(cx).value().as_str(), "Test repair");
        assert_eq!(this.prompt.read(cx).value().as_str(), "Fix the tests");
    });
    assert!(next_effect(&mut commands).is_none());
    visual.simulate_resize(size(px(720.), px(480.)));
    visual.run_until_parked();
    let start = visual.debug_bounds("launch-start").unwrap();
    assert!(start.bottom() <= px(480.) && start.right() <= px(720.));
    assert!(start.top() >= px(0.));
    visual.simulate_click(start.center(), gpui::Modifiers::default());
    let Command::Create(request) = commands.try_recv().unwrap() else {
        panic!("expected launch")
    };
    assert_eq!(request.provider, "codex");
    assert_eq!(request.cwd, "/work/beta");
    assert_eq!(request.label, "Test repair");
    assert_eq!(request.message, "Fix the tests");
    assert!(commands.try_recv().is_err());
}

/// The hub's outcome-unknown launch failure at the smallest supported
/// window: the recovery step is a whole, unclipped line above a visible
/// Start action, and every word of the hub's text can be scrolled to.
#[gpui::test]
fn uncertain_launch_error_keeps_recovery_and_full_text_reachable(cx: &mut TestAppContext) {
    const ERROR: &str = "launch admission may have executed for session 480e8332-3423-44e7-9c18-2b7e274ea3f9: daemon returned 503 Service Unavailable: execution engine unavailable: provider executable not found or not executable; execution outcome is unknown; inspect its outcome before retrying; cleanup: daemon returned 404 Not Found: no wrapper attached";
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.simulate_resize(size(px(720.), px(480.)));
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.demo = false;
            this.update_view(Arc::new(state("a")), window, cx);
            this.show_new_session(window, cx);
            this.projects.cwd = "/work/api".into();
            let mut failed = state("a");
            failed.spawn_receipt = Some(wks_native::controller::SpawnReceipt {
                number: 1,
                session: None,
                error: Some(ERROR.into()),
                unsent_message: None,
            });
            this.update_view(Arc::new(failed), window, cx);
        });
    });
    visual.run_until_parked();
    let window = gpui::Bounds::new(gpui::point(px(0.), px(0.)), size(px(720.), px(480.)));
    let inside = |b: gpui::Bounds<gpui::Pixels>| {
        b.top() >= window.top()
            && b.bottom() <= window.bottom()
            && b.left() >= window.left()
            && b.right() <= window.right()
    };
    let card = visual.debug_bounds("launch-error").unwrap();
    let headline = visual.debug_bounds("launch-error-headline").unwrap();
    let guidance = visual.debug_bounds("launch-error-guidance").unwrap();
    let start = visual.debug_bounds("launch-start").unwrap();
    for (name, b) in [
        ("card", card),
        ("headline", headline),
        ("guidance", guidance),
        ("start", start),
    ] {
        assert!(inside(b), "{name} {b:?} leaves the window");
    }
    // The guidance may wrap; its own box holds every line, and the card
    // encloses it rather than clipping it.
    assert!(guidance.bottom() <= card.bottom() && card.bottom() <= start.top());
    // The hub's wording is longer than its region; scrolling reaches the end.
    let region = visual.debug_bounds("launch-error-details").unwrap();
    let text = visual.debug_bounds("launch-error-text").unwrap();
    assert!(inside(region), "{region:?}");
    // Every wrapped line of the whole string is laid out: the box is as
    // tall as GPUI's own unclamped wrap of it at that width.
    let (lines, line_height) = visual.update(|window, _| {
        let mut style = window.text_style();
        style.font_size = px(12.).into();
        let shaped = window
            .text_system()
            .shape_text(
                ERROR.into(),
                px(12.),
                &[style.to_run(ERROR.len())],
                Some(text.size.width),
                None,
            )
            .unwrap();
        let lines: usize = shaped.iter().map(|l| l.wrap_boundaries().len() + 1).sum();
        (lines, style.line_height_in_pixels(window.rem_size()))
    });
    assert!(
        lines > 3,
        "a long error wraps past any small clamp: {lines}"
    );
    assert!(
        text.size.height >= line_height * (lines as f32 - 0.5),
        "{text:?} shows fewer than {lines} lines of {line_height:?}"
    );
    assert!(
        text.size.height > region.size.height,
        "{text:?} in {region:?}"
    );
    assert!(
        text.bottom() > region.bottom(),
        "the end starts out of view"
    );
    visual.simulate_event(gpui::ScrollWheelEvent {
        position: region.center(),
        delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-400.))),
        ..Default::default()
    });
    visual.run_until_parked();
    let scrolled = visual.debug_bounds("launch-error-text").unwrap();
    assert!(
        scrolled.bottom() <= region.bottom() + px(1.) && scrolled.bottom() > region.top(),
        "end of {scrolled:?} reachable in {region:?}"
    );
    assert_eq!(visual.debug_bounds("launch-start").unwrap(), start);
    workspace.read_with(&visual, |this, _| {
        assert_eq!(this.spawn_error, ERROR, "nothing is truncated in state");
        assert!(wks_native::launch::uncertain_outcome(&this.spawn_error));
    });
    // Still allowed after checking: one click is one launch.
    visual.simulate_click(start.center(), gpui::Modifiers::default());
    let Some(Command::Create(request)) = next_effect(&mut commands) else {
        panic!("expected one create command")
    };
    assert_eq!(request.cwd, "/work/api");
    assert!(commands.try_recv().is_err());
}

/// Bug #22: the New Agent footer is a panel inset by the form's gutters and
/// aligned with its cards, not a band at the window edge; it stays put while
/// the form scrolls, in every theme and at the smallest window.
#[gpui::test]
fn launch_footer_is_an_inset_panel_aligned_with_the_form(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.demo = false;
            this.update_view(Arc::new(state("a")), window, cx);
            this.show_new_session(window, cx);
            this.projects.cwd = "/work/api".into();
        });
    });
    for (width, height) in [(720., 480.), (1000., 700.), (1400., 900.)] {
        visual.simulate_resize(size(px(width), px(height)));
        let gutter = px(if height < 620. { 12. } else { 20. });
        for appearance in Appearance::ALL {
            visual.update(|window, cx| {
                workspace.update(cx, |this, cx| this.set_appearance(appearance, window, cx))
            });
            visual.run_until_parked();
            let label = appearance.label();
            let footer = visual.debug_bounds("launch-footer").unwrap();
            let content = visual.debug_bounds("launch-content").unwrap();
            let start = visual.debug_bounds("launch-start").unwrap();
            let status = visual.debug_bounds("launch-status").unwrap();
            let near = |a: gpui::Pixels, b: gpui::Pixels| (a - b).abs() <= px(1.);
            assert!(
                near(footer.bottom(), px(height) - gutter),
                "{label} {width}x{height}: {footer:?} is not inset from the bottom"
            );
            assert!(
                near(footer.left(), content.left()) && near(footer.right(), content.right()),
                "{label} {width}x{height}: {footer:?} not aligned with {content:?}"
            );
            for (name, inner) in [("start", start), ("status", status)] {
                assert!(
                    inner.left() > footer.left()
                        && inner.right() < footer.right()
                        && inner.top() > footer.top()
                        && inner.bottom() < footer.bottom(),
                    "{label} {width}x{height}: {name} {inner:?} outside {footer:?}"
                );
            }
            // Button and status share one centered row.
            assert!(near(start.center().y, status.center().y), "{label}");
        }
    }
    // Sticky: scrolling the form moves the cards, never the footer.
    visual.simulate_resize(size(px(720.), px(480.)));
    visual.run_until_parked();
    let footer = visual.debug_bounds("launch-footer").unwrap();
    let content = visual.debug_bounds("launch-content").unwrap();
    visual.simulate_event(gpui::ScrollWheelEvent {
        position: content.center(),
        delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-200.))),
        ..Default::default()
    });
    visual.run_until_parked();
    assert!(visual.debug_bounds("launch-content").unwrap().top() < content.top());
    assert_eq!(visual.debug_bounds("launch-footer").unwrap(), footer);
    let start = visual.debug_bounds("launch-start").unwrap();
    visual.simulate_click(start.center(), gpui::Modifiers::default());
    let Some(Command::Create(request)) = next_effect(&mut commands) else {
        panic!("expected one create command")
    };
    assert_eq!(request.cwd, "/work/api");
    assert!(commands.try_recv().is_err());
}

#[gpui::test]
fn new_session_form_creates_once_and_keeps_failed_input(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.demo = false;
            this.update_view(Arc::new(state("a")), window, cx);
        })
    });
    visual.simulate_keystrokes("ctrl-n");
    visual.simulate_input("/work/project");
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            assert!(this.new_session);
            this.provider = "codex";
            this.prompt
                .update(cx, |input, cx| input.set_value("Hello", window, cx));
        })
    });
    visual.simulate_keystrokes("ctrl-enter");
    visual.simulate_keystrokes("ctrl-enter");
    let Some(Command::Create(request)) = next_effect(&mut commands) else {
        panic!("expected one create command")
    };
    assert_eq!(request.cwd, "/work/project");
    assert_eq!(request.provider, "codex");
    assert_eq!(request.message, "Hello");
    assert!(commands.try_recv().is_err());
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            let mut failed = state("a");
            failed.spawn_receipt = Some(wks_native::controller::SpawnReceipt {
                number: 1,
                session: None,
                error: Some("Launch failed".into()),
                unsent_message: None,
            });
            this.update_view(Arc::new(failed), window, cx);
            assert!(this.new_session);
            assert!(!this.spawn_pending);
            assert_eq!(this.prompt.read(cx).value().as_ref(), "Hello");
            assert_eq!(this.projects.cwd.as_str(), "/work/project");
            assert_eq!(this.spawn_error, "Launch failed");
            let mut success = state("b");
            success.spawn_receipt = Some(wks_native::controller::SpawnReceipt {
                number: 2,
                session: Some("b".into()),
                error: None,
                unsent_message: Some("Hello".into()),
            });
            this.update_view(Arc::new(success), window, cx);
            assert!(!this.new_session);
            assert_eq!(this.composer.read(cx).value().as_ref(), "Hello");
        })
    });
}

#[gpui::test]
fn default_access_applies_on_new_launch_and_provider_switch(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.demo = false;
            this.settings.default_claude_access = Permission::FullAccess;
            this.settings.default_codex_access = Permission::Ask;
            this.update_view(Arc::new(state("a")), window, cx);
            this.show_new_session(window, cx);
            assert_eq!(this.permission, Permission::FullAccess);
            this.choose_provider("codex", window, cx);
            assert_eq!(this.permission, Permission::Ask);
            this.permission = Permission::FullAccess;
            this.new_session = false;
            this.settings.default_provider = Provider::Codex;
            this.show_new_session(window, cx);
            assert_eq!(
                this.permission,
                Permission::Ask,
                "a new launch reapplies the saved default even for the same provider"
            );
            this.choose_provider("claude", window, cx);
            assert_eq!(this.permission, Permission::FullAccess);
            this.select_project("/work/project", window, cx);
            this.create(window, cx);
        });
    });
    let Some(Command::Create(request)) = next_effect(&mut commands) else {
        panic!("expected create")
    };
    assert_eq!(request.permission, Permission::FullAccess);
    assert_eq!(
        request.params().unwrap()["permissionMode"],
        "bypassPermissions"
    );
}

#[gpui::test]
fn model_menu_shows_catalog_when_custom_is_selected(cx: &mut TestAppContext) {
    let (workspace, mut visual, _, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.demo = false;
            let mut loaded = state("a");
            loaded.catalog = wks_native::launch::Catalog {
                key: CatalogKey {
                    provider: "claude".into(),
                    cwd: String::new(),
                },
                models: ["opus", "sonnet", "haiku"]
                    .into_iter()
                    .map(|id| ModelChoice {
                        id: id.into(),
                        label: id.into(),
                        windows: vec![],
                        is_default: false,
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            };
            this.update_view(Arc::new(loaded), window, cx);
            this.show_new_session(window, cx);
            this.model_choice = "__custom".into();
            this.model_picker.update(cx, |picker, cx| {
                picker.set_selected_value(&String::from("__custom"), window, cx);
                picker.focus(window, cx);
            });
        });
    });
    visual.simulate_keystrokes("enter");
    visual.run_until_parked();
    for selector in [
        "model-option-opus",
        "model-option-sonnet",
        "model-option-haiku",
    ] {
        assert!(
            visual.debug_bounds(selector).is_some(),
            "selected Custom must not scroll {selector} out of a menu that can fit all entries"
        );
    }
    visual.simulate_input("custom");
    visual.run_until_parked();
    visual.simulate_keystrokes("enter");
    workspace.read_with(&visual, |this, _| assert_eq!(this.model_choice, "__custom"));
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.show_screen(Screen::Conversation, window, cx);
            this.show_new_session(window, cx);
            this.model_picker
                .update(cx, |picker, cx| picker.focus(window, cx));
        });
    });
    visual.simulate_keystrokes("enter down enter");
    workspace.read_with(&visual, |this, _| {
        assert_eq!(
            this.model_choice, "opus",
            "a new picker must clear the previous Custom-only search"
        )
    });
}

#[gpui::test]
fn fresh_agent_does_not_inherit_custom_model_from_session_controls(cx: &mut TestAppContext) {
    let (workspace, mut visual, _, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.demo = false;
            this.update_view(Arc::new(state("a")), window, cx);
            this.open_feature(Screen::Model, window, cx);
            assert_eq!(this.model_choice, "__custom");
            this.show_new_session(window, cx);
            assert!(this.model_choice.is_empty(), "new agent should start with Provider default, rather than the inspected session's Custom entry");
            assert_eq!(this.model_picker.read(cx).selected_value().map(String::as_str), Some(""));
        });
    });
}

#[gpui::test]
fn opening_new_agent_uses_an_already_loaded_catalog(cx: &mut TestAppContext) {
    let (workspace, mut visual, _, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.demo = false;
            let mut loaded = state("a");
            loaded.catalog = wks_native::launch::Catalog {
                key: CatalogKey {
                    provider: "claude".into(),
                    cwd: String::new(),
                },
                models: vec![ModelChoice {
                    id: "opus".into(),
                    label: "Opus".into(),
                    windows: vec![200000],
                    is_default: false,
                    ..Default::default()
                }],
                ..Default::default()
            };
            this.update_view(Arc::new(loaded), window, cx);
            this.show_new_session(window, cx);
            assert_eq!(
                this.catalog_models.len(),
                1,
                "a cached catalog must populate without waiting for another update"
            );
        });
    });
}

#[gpui::test]
fn open_model_dropdown_renders_arriving_catalog(cx: &mut TestAppContext) {
    let (workspace, mut visual, _, _updates) = fixture(cx);
    visual.simulate_resize(size(px(1000.), px(700.)));
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.demo = false;
            this.update_view(Arc::new(state("a")), window, cx);
            this.show_new_session(window, cx);
            this.model_picker
                .update(cx, |picker, cx| picker.focus(window, cx));
        });
    });
    visual.simulate_keystrokes("enter");
    visual.run_until_parked();
    assert!(visual.debug_bounds("model-option-").is_some());
    assert!(visual.debug_bounds("model-option-opus").is_none());
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            let mut loaded = state("a");
            loaded.catalog = wks_native::launch::Catalog {
                key: CatalogKey {
                    provider: "claude".into(),
                    cwd: String::new(),
                },
                models: vec![ModelChoice {
                    id: "opus".into(),
                    label: "Opus".into(),
                    windows: vec![200000],
                    is_default: false,
                    ..Default::default()
                }],
                ..Default::default()
            };
            this.update_view(Arc::new(loaded), window, cx);
        });
    });
    visual.run_until_parked();
    let option = visual
        .debug_bounds("model-option-opus")
        .expect("catalog arrival should redraw the open dropdown");
    assert!(option.size.width > px(0.) && option.size.height > px(0.));
    assert!(
        option.top() >= px(0.) && option.bottom() <= px(700.),
        "model option outside viewport: {option:?}"
    );
    visual.simulate_click(option.center(), gpui::Modifiers::default());
    workspace.read_with(&visual, |this, _| assert_eq!(this.model_choice, "opus"));
}

#[gpui::test]
fn model_picker_keyboard_selection_and_provider_reset(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.demo = false;
            this.update_view(Arc::new(state("a")), window, cx);
            this.show_new_session(window, cx);
            this.select_project("/work/project", window, cx);
            let mut loaded = state("a");
            loaded.catalog = wks_native::launch::Catalog {
                key: CatalogKey {
                    provider: "claude".into(),
                    cwd: String::new(),
                },
                models: vec![ModelChoice {
                    id: "opus".into(),
                    label: "Opus".into(),
                    windows: vec![200000, 1000000],
                    is_default: false,
                    ..Default::default()
                }],
                ..Default::default()
            };
            this.update_view(Arc::new(loaded), window, cx);
            this.launch_details_open = true;
            this.model_picker
                .update(cx, |picker, cx| picker.focus(window, cx));
        });
    });
    visual.simulate_keystrokes("enter down enter");
    workspace.read_with(&visual, |this, _| {
        assert_eq!(this.model_choice, "opus");
        assert_eq!(this.context_window, Some(200000));
    });
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.context_window = Some(1000000);
            this.permission = Permission::Plan;
            this.create(window, cx);
            this.spawn_pending = false;
            this.choose_provider("codex", window, cx);
            assert!(this.model_choice.is_empty());
            assert_eq!(this.context_window, None);
            assert_eq!(this.permission, Permission::Ask);
            assert!(this.catalog_models.is_empty());
            this.model_choice = "__custom".into();
            this.create(window, cx);
            assert!(this.spawn_error.contains("Enter a custom model"));
        });
    });
    let requests: Vec<_> = std::iter::from_fn(|| commands.try_recv().ok())
        .filter_map(|command| match command {
            Command::Create(request) => Some(request),
            _ => None,
        })
        .collect();
    assert_eq!(requests.len(), 1);
    let params = requests[0].params().unwrap();
    assert_eq!(params["model"], "opus");
    assert_eq!(params["contextWindow"], 1000000);
    assert_eq!(params["permissionMode"], "plan");
    assert_eq!(params["skipPermissions"], false);
}

#[gpui::test]
fn effort_sits_beside_the_model_and_follows_what_the_model_accepts(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    let catalog = |view: &mut View| {
        view.catalog = wks_native::launch::Catalog {
            key: CatalogKey {
                provider: "codex".into(),
                cwd: "/work/project".into(),
            },
            models: wks_native::launch::parse_models(
                "codex",
                serde_json::json!([
                    {"id":"sol","label":"Sol","default":true,
                     "effortLevels":["low","medium","high","xhigh"],"defaultEffort":"medium"},
                    {"id":"mini","label":"Mini","effortLevels":["minimal","low"],"defaultEffort":"low"}
                ]),
            )
            .unwrap(),
            ..Default::default()
        };
    };
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.demo = false;
            this.settings.default_provider = Provider::Codex;
            this.update_view(Arc::new(state("a")), window, cx);
            this.show_new_session(window, cx);
            this.select_project("/work/project", window, cx);
            let mut loaded = state("a");
            catalog(&mut loaded);
            this.update_view(Arc::new(loaded), window, cx);
        })
    });
    visual.run_until_parked();
    // Visible without opening Options, on the model's row.
    let model = visual.debug_bounds("launch-model-picker").unwrap();
    let effort = visual.debug_bounds("launch-effort-picker").unwrap();
    assert!(visual.debug_bounds("launch-details").is_none());
    assert_eq!(model.top(), effort.top());
    assert!(effort.left() > model.right());
    let focus = |picker: fn(&Workspace) -> Entity<SelectState<SearchableVec<PickerItem>>>| {
        let workspace = workspace.clone();
        move |window: &mut Window, cx: &mut App| {
            let picker = picker(workspace.read(cx));
            picker.update(cx, |p, cx| p.focus(window, cx));
        }
    };
    // The menu lists exactly what Sol accepts.
    visual.update(focus(|this| this.effort_picker.clone()));
    visual.simulate_keystrokes("enter");
    visual.run_until_parked();
    assert!(visual.debug_bounds("effort-option-xhigh").is_some());
    assert!(
        visual.debug_bounds("effort-option-max").is_none(),
        "Codex Sol has no max"
    );
    visual.simulate_keystrokes("down down down down enter");
    workspace.read_with(&visual, |this, _| assert_eq!(this.effort, "xhigh"));
    // A model that does not accept the level returns effort to Default.
    visual.update(focus(|this| this.model_picker.clone()));
    visual.simulate_keystrokes("enter down down enter");
    workspace.read_with(&visual, |this, _| {
        assert_eq!(this.model_choice, "mini");
        assert!(
            this.effort.is_empty(),
            "stale xhigh must not be sent to Mini"
        );
        assert_eq!(this.effort_options().0, ["minimal", "low"]);
    });
    visual.update(focus(|this| this.effort_picker.clone()));
    visual.simulate_keystrokes("enter down enter");
    workspace.read_with(&visual, |this, _| assert_eq!(this.effort, "minimal"));
    visual.simulate_keystrokes("ctrl-enter");
    let Some(Command::Create(request)) = next_effect(&mut commands) else {
        panic!("expected launch")
    };
    let params = request.params().unwrap();
    assert_eq!(
        (params["model"].as_str(), params["effort"].as_str()),
        (Some("mini"), Some("minimal"))
    );
    // Claude has its own ladder; nothing chosen for Codex carries over.
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.spawn_pending = false;
            this.choose_provider("claude", window, cx);
            assert!(this.effort.is_empty());
            assert_eq!(
                this.effort_options().0,
                wks_native::launch::CLAUDE_EFFORTS.map(String::from)
            );
            this.create(window, cx);
        })
    });
    let Some(Command::Create(request)) = next_effect(&mut commands) else {
        panic!("expected launch")
    };
    assert!(
        request.params().unwrap().get("effort").is_none(),
        "Default sends no effort"
    );
    // Still on screen at the minimum window size.
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.spawn_pending = false;
            this.show_new_session(window, cx);
        })
    });
    visual.simulate_resize(size(px(720.), px(480.)));
    visual.run_until_parked();
    let effort = visual.debug_bounds("launch-effort-picker").unwrap();
    assert!(effort.right() <= px(720.));
}

/// A hub registry reply, as the controller would publish it.
fn with_registry(view: &mut View, number: u64, mut registry: serde_json::Value) {
    registry["revision"] = serde_json::json!(number);
    view.requests.insert(
        "projects",
        wks_native::features::RequestState {
            number,
            request: wks_native::features::Request::Projects,
            loading: false,
            value: Arc::new(registry),
            error: None,
        },
    );
}

#[gpui::test]
fn project_chooser_is_keyboard_first_and_never_launches_without_a_folder(cx: &mut TestAppContext) {
    use wks_native::features::Request;
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.demo = false;
            let mut view = state("a");
            with_registry(
                &mut view,
                1,
                serde_json::json!({"projects":{
                    "/work/api":{"label":"API Server","lastOpened":20},
                    "/work/web":{"favourite":true},
                    "/work/old":{"lastOpened":10}
                },"favourites":[],"recent":[],"configured":[]}),
            );
            this.update_view(Arc::new(view), window, cx);
        })
    });
    // No folder yet: the form opens on the chooser with the search focused.
    visual.simulate_keystrokes("ctrl-n");
    visual.run_until_parked();
    workspace.read_with(&visual, |this, cx| {
        assert!(this.projects.picker_open && this.projects.cwd.is_empty());
        let rows: Vec<_> = this
            .pick_rows(cx)
            .iter()
            .map(|r| match r {
                projects::PickRow::Project(p) => p.path.clone(),
                other => panic!("unexpected {other:?}"),
            })
            .collect();
        assert_eq!(
            rows,
            ["/work/web", "/work/api", "/work/old"],
            "pinned, then recent"
        );
    });
    assert!(
        std::iter::from_fn(|| commands.try_recv().ok())
            .any(|c| matches!(c, Command::Request(Request::Projects))),
        "opening the form reads the hub's registry"
    );
    // Launching without a folder explains itself and does not create.
    visual.simulate_keystrokes("ctrl-enter");
    workspace.read_with(&visual, |this, _| {
        assert_eq!(this.spawn_error, "Choose a project folder first.");
        assert!(this.projects.picker_open);
    });
    assert!(next_effect(&mut commands).is_none());
    // Search by label, move, choose with Enter.
    visual.simulate_input("server");
    visual.simulate_keystrokes("enter");
    workspace.read_with(&visual, |this, _| {
        assert_eq!(this.projects.cwd, "/work/api");
        assert!(!this.projects.picker_open);
        assert!(this.spawn_error.is_empty());
    });
    let inspected = std::iter::from_fn(|| commands.try_recv().ok()).find_map(|c| match c {
        Command::Request(Request::InspectProject { path }) => Some(path),
        _ => None,
    });
    assert_eq!(
        inspected.as_deref(),
        Some("/work/api"),
        "the hub checks the chosen folder"
    );
    // Change, arrow down, Esc keeps the original choice.
    visual
        .update(|window, cx| workspace.update(cx, |this, cx| this.open_project_picker(window, cx)));
    visual.simulate_keystrokes("down");
    visual.simulate_keystrokes("escape");
    workspace.read_with(&visual, |this, _| {
        assert!(!this.projects.picker_open);
        assert_eq!(this.projects.cwd, "/work/api");
    });
    // A pasted path that is not a project: offered first, kept visible
    // as the current folder when the list reopens, and launched into.
    visual
        .update(|window, cx| workspace.update(cx, |this, cx| this.open_project_picker(window, cx)));
    visual.simulate_input("/srv/fresh/");
    workspace.read_with(&visual, |this, cx| {
        assert!(matches!(&this.pick_rows(cx)[0], projects::PickRow::Typed(p) if p == "/srv/fresh"));
    });
    visual.simulate_keystrokes("ctrl-enter");
    let Some(Command::Create(request)) = next_effect(&mut commands) else {
        panic!("ctrl-enter launches into the pasted folder")
    };
    assert_eq!(request.cwd, "/srv/fresh");
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.spawn_pending = false;
            this.open_project_picker(window, cx);
            assert!(
                matches!(&this.pick_rows(cx)[0], projects::PickRow::Current(p) if p == "/srv/fresh")
            );
        })
    });
}

#[gpui::test]
fn fresh_forms_drop_resumed_context_and_stale_folder_checks(cx: &mut TestAppContext) {
    use wks_native::features::{Request, RequestState};
    use wks_native::projects::Inspection;
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.demo = false;
            let mut view = state("a");
            Arc::make_mut(&mut view.sessions)[0].cwd = "/work/alpha".into();
            Arc::make_mut(&mut view.sessions)[1].cwd = "/work/beta".into();
            this.update_view(Arc::new(view), window, cx);
            let old = Session {
                id: "past".into(),
                label: "Old work".into(),
                provider: "claude".into(),
                cwd: "/work/beta".into(),
                state: "stopped".into(),
                ..Default::default()
            };
            this.resume_session(&old, window, cx);
            assert_eq!(this.projects.cwd, "/work/beta");
            assert_eq!(this.label.read(cx).value().as_ref(), "Old work");
            this.prompt
                .update(cx, |i, cx| i.set_value("continue please", window, cx));
            this.effort = "high".into();
            this.show_screen(Screen::Conversation, window, cx);
            // A fresh New Agent is not the resumed conversation.
            this.show_new_session(window, cx);
            assert!(this.extras.resume.is_none());
            assert!(this.label.read(cx).value().is_empty());
            assert!(this.prompt.read(cx).value().is_empty());
            assert!(this.effort.is_empty());
            assert_eq!(
                this.projects.cwd, "/work/alpha",
                "starts where the user is looking"
            );
            // A plain draft survives leaving and reopening the form.
            this.prompt
                .update(cx, |i, cx| i.set_value("my draft", window, cx));
            this.show_screen(Screen::Conversation, window, cx);
            this.show_new_session(window, cx);
            assert_eq!(this.prompt.read(cx).value().as_ref(), "my draft");
            // A check for a folder that is no longer chosen is not shown.
            let mut view = (*this.view).clone();
            view.requests.insert(
                "project-inspect",
                RequestState {
                    number: 50,
                    request: Request::InspectProject {
                        path: "/work/beta".into(),
                    },
                    loading: false,
                    value: Arc::new(serde_json::json!({"exists":false,"error":"gone"})),
                    error: None,
                },
            );
            this.update_view(Arc::new(view), window, cx);
            assert!(this.inspection().is_none());
            let mut view = (*this.view).clone();
            view.requests.insert(
                "project-inspect",
                RequestState {
                    number: 51,
                    request: Request::InspectProject {
                        path: "/work/alpha".into(),
                    },
                    loading: false,
                    value: Arc::new(serde_json::json!({"exists":false,"error":"gone"})),
                    error: None,
                },
            );
            this.update_view(Arc::new(view), window, cx);
            assert_eq!(
                this.inspection(),
                Some(Ok(Inspection::Missing("gone".into())))
            );
            // Re-checking the same folder keeps the last answer up rather
            // than blanking it to "Checking folder…" on every open.
            let mut view = (*this.view).clone();
            view.requests.insert(
                "project-inspect",
                RequestState {
                    number: 52,
                    request: Request::InspectProject {
                        path: "/work/alpha".into(),
                    },
                    loading: true,
                    value: Arc::new(serde_json::json!({"exists":false,"error":"gone"})),
                    error: None,
                },
            );
            this.update_view(Arc::new(view), window, cx);
            assert!(!this.inspecting());
            assert_eq!(
                this.inspection(),
                Some(Ok(Inspection::Missing("gone".into())))
            );
        })
    });
    visual.run_until_parked();
    assert!(visual.debug_bounds("launch-project-status").is_some());
}

#[gpui::test]
fn setup_round_trip_preserves_resume_and_model_picker_tracks_actual_choice(
    cx: &mut TestAppContext,
) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.demo = false;
            this.update_view(Arc::new(state("a")), window, cx);
            let old = Session {
                id: "past".into(),
                provider: "codex".into(),
                cwd: "/project".into(),
                state: "stopped".into(),
                ..Default::default()
            };
            this.resume_session(&old, window, cx);
            this.open_feature(Screen::Setup, window, cx);
            this.back_from_feature(window, cx);
            assert!(this.new_session);
            assert_eq!(this.extras.resume.as_deref(), Some("past"));
            this.create(window, cx);
            this.spawn_pending = false;
            this.new_session = false;
            this.open_feature(Screen::Model, window, cx);
            assert_eq!(
                this.model_picker
                    .read(cx)
                    .selected_value()
                    .map(String::as_str),
                Some("__custom")
            );
        })
    });
    let mut resumed = false;
    while let Ok(command) = commands.try_recv() {
        if let Command::Create(request) = command {
            assert_eq!(request.resume_session_id.as_deref(), Some("past"));
            resumed = true;
        }
    }
    assert!(resumed);
}

#[gpui::test]
fn unsupported_provider_is_never_silently_resumed_as_claude(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.demo = false;
            let old = Session {
                id: "other".into(),
                provider: "pi".into(),
                state: "stopped".into(),
                ..Default::default()
            };
            this.resume_session(&old, window, cx);
            assert!(!this.new_session);
            assert!(this.extras.notice.contains("cannot be resumed"));
        })
    });
    assert!(commands.try_recv().is_err());
}

#[gpui::test]
fn model_and_resume_keep_the_requested_context_pair(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    visual.update(|window, cx| workspace.update(cx, |this, cx| {
        this.demo = false;
        let mut next = state("a");
        let session = &mut Arc::make_mut(&mut next.sessions)[0];
        session.merge(&serde_json::json!({"provider":"claude","requestedSelection":{"model":"opus","contextWindow":1000000}}));
        let session = session.clone();
        this.update_view(Arc::new(next), window, cx);
        this.open_feature(Screen::Model, window, cx);
        assert_eq!(this.context_window, Some(1000000));
        assert_eq!(this.model.read(cx).value().as_ref(), "opus");
        this.resume_session(&session, window, cx);
        assert_eq!(this.context_window, Some(1000000));
        assert_eq!(this.model.read(cx).value().as_ref(), "opus");
        this.resume_session(&Session { provider:"claude".into(), id:"old".into(), cwd:"/project".into(), ..Default::default() }, window, cx);
        assert!(this.model_choice.is_empty());
        assert_eq!(this.context_window, None);
    }));
}

#[gpui::test]
fn context_window_choices_are_one_even_row(cx: &mut TestAppContext) {
    let (workspace, mut visual, _, _) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.demo = false;
            let mut next = state("a");
            let sessions = Arc::make_mut(&mut next.sessions);
            sessions[0].provider = "codex".into();
            sessions[0].model = "gpt-6.1-sol".into();
            sessions[0].context_window = Some(1_000_000);
            this.update_view(Arc::new(next), window, cx);
            this.open_feature(Screen::Model, window, cx);
            assert_eq!(this.model_choice, "__custom");
        })
    });
    visual.run_until_parked();
    let default = visual.debug_bounds("context-window-0").unwrap();
    let million = visual.debug_bounds("context-window-1").unwrap();
    assert_eq!(default.size.height, million.size.height);
    assert_eq!(default.top(), million.top());
    assert!(default.size.height <= px(32.), "{:?}", default.size);
    visual.simulate_click(default.center(), gpui::Modifiers::default());
    workspace.read_with(&visual, |this, _| assert_eq!(this.context_window, None));
    visual.simulate_click(million.center(), gpui::Modifiers::default());
    workspace.read_with(&visual, |this, _| {
        assert_eq!(this.context_window, Some(1_000_000))
    });
}

#[gpui::test]
fn model_page_switches_a_running_sessions_access_live(cx: &mut TestAppContext) {
    use wks_native::controller::Action;
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.simulate_resize(size(px(1200.), px(900.)));
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.demo = false;
            let mut view = state("a");
            Arc::make_mut(&mut view.sessions)[0].provider = "claude".into();
            Arc::make_mut(&mut view.sessions)[0].permission_mode = "default".into();
            this.update_view(Arc::new(view), window, cx);
            this.open_feature(Screen::Model, window, cx);
        })
    });
    visual.run_until_parked();
    effects(&mut commands);
    // The current mode is shown chosen and is not sent again.
    click(&mut visual, "live-access-Ask to approve");
    assert!(effects(&mut commands).is_empty());
    click(&mut visual, "live-access-Plan mode");
    match effects(&mut commands).as_slice() {
        [
            Command::Act {
                session,
                action: Action::SetPermission { mode, label },
            },
        ] => {
            assert_eq!(session, "a");
            assert_eq!(mode, "plan");
            assert_eq!(label, "Plan mode");
        }
        other => panic!("expected one access switch, got {} commands", other.len()),
    }
    // Codex offers its own two modes, on its own wire ids.
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            let mut view = (*this.view).clone();
            Arc::make_mut(&mut view.sessions)[0].provider = "codex".into();
            Arc::make_mut(&mut view.sessions)[0].permission_mode = "ask".into();
            this.update_view(Arc::new(view), window, cx);
        })
    });
    visual.run_until_parked();
    click(&mut visual, "live-access-Full access");
    assert!(matches!(effects(&mut commands).as_slice(),
        [Command::Act { action: Action::SetPermission { mode, .. }, .. }] if mode == "yolo"));
}
