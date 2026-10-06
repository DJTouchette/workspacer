//! The session sidebar: nesting, selection, filters, names and usage.
use super::*;

#[gpui::test]
fn sidebar_resize_tracks_drag_and_persists_width(cx: &mut TestAppContext) {
    let (workspace, mut visual, _, _) = fixture(cx);
    let path = std::env::temp_dir().join(format!("native-sidebar-{}.json", std::process::id()));
    visual.update(|_, cx| workspace.update(cx, |this, _| this.settings_path = Some(path.clone())));
    visual.run_until_parked();
    assert_eq!(
        visual.debug_bounds("session-sidebar").unwrap().size.width,
        px(304.)
    );
    let start = visual.debug_bounds("sidebar-resize").unwrap().center();
    let end = start + gpui::point(px(70.), px(0.));
    visual.simulate_mouse_down(start, gpui::MouseButton::Left, gpui::Modifiers::default());
    visual.run_until_parked();
    visual.simulate_mouse_move(
        end,
        Some(gpui::MouseButton::Left),
        gpui::Modifiers::default(),
    );
    visual.run_until_parked();
    visual.simulate_mouse_up(end, gpui::MouseButton::Left, gpui::Modifiers::default());
    visual.run_until_parked();
    assert_eq!(
        visual.debug_bounds("session-sidebar").unwrap().size.width,
        px(374.)
    );
    assert_eq!(Settings::load(&path).unwrap().sidebar_width, 374.);
    workspace.read_with(&visual, |this, _| assert!(this.sidebar_drag.is_none()));
    visual.simulate_resize(size(px(720.), px(480.)));
    visual.run_until_parked();
    assert_eq!(
        visual.debug_bounds("session-sidebar").unwrap().size.width,
        px(288.)
    );
    visual.simulate_resize(size(px(1000.), px(700.)));
    visual.run_until_parked();
    assert_eq!(
        visual.debug_bounds("session-sidebar").unwrap().size.width,
        px(374.)
    );
    let _ = std::fs::remove_file(path);
}

#[gpui::test]
fn sidebar_model_metadata_stays_inside_clickable_session_rows(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            let mut view = state("a");
            let session = &mut Arc::make_mut(&mut view.sessions)[0];
            session.provider = "codex".into();
            session.model = "a-model-with-a-long-name-that-needs-to-fit-in-the-sidebar".into();
            session.cwd = "/work/project".into();
            this.update_view(Arc::new(view), window, cx);
        });
    });
    visual.run_until_parked();
    let row = visual.debug_bounds("sidebar-session-0").unwrap();
    let model = visual.debug_bounds("sidebar-model-0").unwrap();
    assert!(row.contains(&model.origin));
    assert!(row.contains(&model.bottom_right()));
    let second = visual.debug_bounds("sidebar-session-1").unwrap();
    visual.simulate_click(second.center(), gpui::Modifiers::default());
    assert!(matches!(commands.try_recv().unwrap(), Command::Select(id) if id == "b"));
}

#[gpui::test]
fn empty_sidebar_filters_can_be_cleared_without_switching_sessions(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.simulate_resize(size(px(720.), px(480.)));
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(state("a")), window, cx);
            this.project_filter = Some("/missing-project".into());
            this.search
                .update(cx, |input, cx| input.set_value("missing", window, cx));
            cx.notify();
        });
    });
    visual.run_until_parked();
    workspace.read_with(&visual, |this, cx| {
        assert!(this.visible_sessions(cx).is_empty())
    });
    let reset = visual.debug_bounds("clear-session-filters").unwrap();
    let sidebar = visual.debug_bounds("session-sidebar").unwrap();
    assert!(reset.left() >= sidebar.left() && reset.right() <= sidebar.right());
    assert!(reset.top() >= px(0.) && reset.bottom() <= px(480.));
    visual.simulate_click(reset.center(), gpui::Modifiers::default());
    workspace.read_with(&visual, |this, cx| {
        assert!(this.project_filter.is_none());
        assert!(this.search.read(cx).value().is_empty());
        assert_eq!(this.visible_sessions(cx), vec![0, 1]);
        assert_eq!(this.view.selected.as_deref(), Some("a"));
    });
    assert!(commands.try_recv().is_err());
}

#[gpui::test]
fn vim_navigation_never_steals_composer_text(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(state("a")), window, cx)
        })
    });
    visual.simulate_keystrokes("j k");
    assert!(matches!(commands.try_recv().unwrap(), Command::Select(id) if id == "b"));
    assert!(matches!(commands.try_recv().unwrap(), Command::Select(id) if id == "a"));
    visual.simulate_keystrokes("shift-g g g");
    assert!(matches!(commands.try_recv().unwrap(), Command::Select(id) if id == "b"));
    assert!(matches!(commands.try_recv().unwrap(), Command::Select(id) if id == "a"));
    visual.simulate_keystrokes("ctrl-d ctrl-u i");
    visual.simulate_input("jkgn/hello");
    workspace.read_with(&visual, |this, cx| {
        assert_eq!(this.composer.read(cx).value().as_ref(), "jkgn/hello")
    });
    assert!(commands.try_recv().is_err());
    visual.simulate_keystrokes("escape g s");
    workspace.read_with(&visual, |this, _| assert_eq!(this.screen, Screen::Settings));
    visual.simulate_keystrokes("ctrl-enter");
    assert!(
        commands.try_recv().is_err(),
        "settings must never send a hidden draft"
    );
    visual.simulate_keystrokes("escape i");
    workspace.read_with(&visual, |this, cx| {
        assert_eq!(this.composer.read(cx).value().as_ref(), "jkgn/hello")
    });
}

#[gpui::test]
fn search_and_disabled_vim_keep_input_and_modifier_shortcuts(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(state("a")), window, cx)
        })
    });
    visual.simulate_keystrokes("/");
    visual.simulate_input("Beta");
    visual.simulate_keystrokes("escape j");
    assert!(matches!(commands.try_recv().unwrap(), Command::Select(id) if id == "b"));
    visual.update(|_, cx| {
        workspace.update(cx, |this, cx| {
            this.settings.vim_navigation = false;
            cx.notify();
        })
    });
    visual.simulate_keystrokes("j g s");
    workspace.read_with(&visual, |this, _| {
        assert_eq!(this.screen, Screen::Conversation)
    });
    assert!(commands.try_recv().is_err());
    visual.simulate_keystrokes("ctrl-,");
    workspace.read_with(&visual, |this, _| assert_eq!(this.screen, Screen::Settings));
    visual.simulate_keystrokes("ctrl-p");
    workspace.read_with(&visual, |this, _| assert_eq!(this.screen, Screen::Projects));
}

#[gpui::test]
fn failed_selection_does_not_leave_navigation_pending(cx: &mut TestAppContext) {
    let (workspace, mut visual, commands, _updates) = fixture(cx);
    drop(commands);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(state("a")), window, cx);
            this.command(Command::Select("b".into()), cx);
            assert!(this.navigation_selected.is_none());
            assert!(!this.local_notice.is_empty());
        })
    });
}

#[gpui::test]
fn keyboard_session_selection_uses_one_explicit_command(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(state("a")), window, cx);
            this.settings.vim_navigation = false;
            window.focus(&this.focus);
        })
    });
    visual.run_until_parked();
    // Projects, history, toggle, search, then the first session.
    visual.simulate_keystrokes("tab tab tab tab tab enter");
    visual.simulate_event(gpui::KeyUpEvent {
        keystroke: gpui::Keystroke::parse("enter").unwrap(),
    });
    visual.run_until_parked();
    assert!(matches!(commands.try_recv().unwrap(), Command::Select(id) if id == "a"));
    assert!(commands.try_recv().is_err());
}

#[gpui::test]
fn sidebar_collapse_preserves_draft_filter_and_search_focus(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(state("a")), window, cx);
            this.composer.update(cx, |input, cx| {
                input.set_value("Keep this draft", window, cx)
            });
            this.search
                .update(cx, |input, cx| input.set_value("Alpha", window, cx));
        });
    });
    visual.run_until_parked();
    let expanded = visual.debug_bounds("session-sidebar").unwrap();
    let toggle = visual.debug_bounds("sidebar-toggle").unwrap();
    visual.simulate_click(toggle.center(), gpui::Modifiers::default());
    visual.run_until_parked();
    assert_eq!(
        visual.debug_bounds("session-sidebar").unwrap().size.width,
        px(56.)
    );
    assert!(expanded.size.width > px(56.));
    workspace.read_with(&visual, |this, cx| {
        assert!(this.sidebar_collapsed);
        assert_eq!(this.composer.read(cx).value().as_str(), "Keep this draft");
        assert_eq!(this.search.read(cx).value().as_str(), "Alpha");
        assert_eq!(this.visible_sessions(cx), vec![0]);
        assert_eq!(this.view.selected.as_deref(), Some("a"));
    });
    assert!(commands.try_recv().is_err());
    let search = visual.debug_bounds("sidebar-search").unwrap();
    visual.simulate_click(search.center(), gpui::Modifiers::default());
    visual.run_until_parked();
    assert!(visual.update(|window, cx| {
        workspace
            .read(cx)
            .search
            .read(cx)
            .focus_handle(cx)
            .is_focused(window)
    }));
    workspace.read_with(&visual, |this, cx| {
        assert!(!this.sidebar_collapsed);
        assert_eq!(this.search.read(cx).value().as_str(), "Alpha");
        assert_eq!(this.composer.read(cx).value().as_str(), "Keep this draft");
    });
    assert_eq!(
        visual.debug_bounds("session-sidebar").unwrap().size.width,
        expanded.size.width
    );
}

#[gpui::test]
fn collapsed_sidebar_selects_the_explicit_session(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(state("a")), window, cx);
            this.sidebar_collapsed = true;
            cx.notify();
        })
    });
    visual.run_until_parked();
    let row = visual.debug_bounds("sidebar-session-1").unwrap();
    visual.simulate_click(row.center(), gpui::Modifiers::default());
    visual.run_until_parked();
    assert!(matches!(commands.try_recv().unwrap(), Command::Select(id) if id == "b"));
    assert!(commands.try_recv().is_err());
}

#[gpui::test]
fn sidebar_shows_measured_usage_windows_per_account(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(state("a")), window, cx)
        })
    });
    visual.run_until_parked();
    assert!(visual.debug_bounds("sidebar-usage").is_none());
    let now = chrono::Utc::now().timestamp();
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            let mut next = state("a");
            next.usage = Some(Arc::new(
                serde_json::json!({"providers":[{"provider":"claude","accounts":[
                    {"label":"default","is_default":true,"windows":{"five_hour":{
                        "used_percent":{"state":"ok","value":42.0},"resets_at":now + 3600}}}
                ]}]}),
            ));
            this.update_view(Arc::new(next), window, cx)
        })
    });
    visual.run_until_parked();
    let usage = visual.debug_bounds("sidebar-usage").unwrap();
    assert!(usage.size.height > px(0.));
    // Hover shows every window as a card.
    visual.simulate_mouse_move(usage.center(), None, gpui::Modifiers::default());
    visual
        .executor()
        .advance_clock(std::time::Duration::from_millis(800));
    visual.run_until_parked();
    assert!(visual.debug_bounds("usage-hover-card").is_some());
    // Click opens the detail modal and asks for a fresh reading.
    visual.simulate_click(usage.center(), gpui::Modifiers::default());
    visual.run_until_parked();
    workspace.read_with(&visual, |this, _| assert!(this.usage_open));
    assert!(matches!(commands.try_recv(), Ok(Command::RefreshUsage)));
    // Clicks inside the card do not dismiss it; the close button does.
    let modal = visual.debug_bounds("usage-modal").unwrap();
    visual.simulate_click(modal.center(), gpui::Modifiers::default());
    visual.run_until_parked();
    workspace.read_with(&visual, |this, _| assert!(this.usage_open));
    let close = visual.debug_bounds("usage-close").unwrap();
    visual.simulate_click(close.center(), gpui::Modifiers::default());
    visual.run_until_parked();
    workspace.read_with(&visual, |this, _| assert!(!this.usage_open));
    // Esc closes it too.
    visual.simulate_click(usage.center(), gpui::Modifiers::default());
    visual.run_until_parked();
    visual.simulate_keystrokes("escape");
    workspace.read_with(&visual, |this, _| assert!(!this.usage_open));
}

#[gpui::test]
fn enter_saves_the_session_name_on_this_device(cx: &mut TestAppContext) {
    let (workspace, mut visual, _, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(state("a")), window, cx);
            this.open_feature(Screen::Session, window, cx);
        })
    });
    visual.run_until_parked();
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.extras
                .name
                .update(cx, |input, cx| input.set_value("", window, cx))
        })
    });
    visual.run_until_parked();
    visual.simulate_input("Release prep");
    visual.run_until_parked();
    // A single-line Input propagates Enter after emitting PressEnter; the
    // test platform then types the unhandled key as "\n", which a real
    // platform does not. Dispatch the Input's own Enter action instead.
    visual.dispatch_action(gpui_component::input::Enter { secondary: false });
    visual.run_until_parked();
    workspace.read_with(&visual, |this, _| {
        let saved = this
            .settings
            .names
            .get(&this.project_scope)
            .and_then(|names| names.get("a"));
        assert_eq!(saved.map(String::as_str), Some("Release prep"));
        assert_eq!(this.extras.notice, "Name saved");
        assert_eq!(this.screen, Screen::Session, "saving keeps the page open");
    });
}

/// A name the user saved for a session outranks the hub's automatic
/// title, including one that lands later (a late result never wins).
#[gpui::test]
fn a_saved_name_outranks_a_late_hub_title(cx: &mut TestAppContext) {
    let (workspace, mut visual, _, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            let mut untitled = state("a");
            Arc::make_mut(&mut untitled.sessions)[0].label = String::new();
            this.update_view(Arc::new(untitled), window, cx);
            this.settings
                .names
                .entry(this.project_scope.clone())
                .or_default()
                .insert("a".into(), "Release prep".into());
            // The hub's title arrives after the rename.
            let mut titled = state("a");
            Arc::make_mut(&mut titled.sessions)[0].label = "Fix login redirect".into();
            this.update_view(Arc::new(titled), window, cx);
            let session = this.selected_session().unwrap().clone();
            assert_eq!(session.label, "Fix login redirect");
            assert_eq!(this.session_title(&session), "Release prep");
            // Clearing the saved name shows the hub's title.
            this.settings.names.clear();
            assert_eq!(this.session_title(&session), "Fix login redirect");
        })
    });
}

#[gpui::test]
fn ending_a_session_asks_first_and_cancel_sends_nothing(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(state("a")), window, cx);
            this.open_feature(Screen::Session, window, cx);
        })
    });
    visual.run_until_parked();
    while commands.try_recv().is_ok() {}
    let click = |visual: &mut VisualTestContext, selector: &'static str| {
        let bounds = visual
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} is rendered"));
        visual.simulate_click(bounds.center(), gpui::Modifiers::default());
        visual.run_until_parked();
    };
    let confirming = |visual: &mut VisualTestContext| {
        workspace.read_with(visual, |this, _| this.extras.confirm_end.clone())
    };
    assert_eq!(confirming(&mut visual), None);
    click(&mut visual, "end-session");
    assert_eq!(confirming(&mut visual).as_deref(), Some("a"));
    assert!(visual.debug_bounds("confirm-end-panel").is_some());
    click(&mut visual, "cancel-end");
    assert_eq!(confirming(&mut visual), None);
    assert!(
        std::iter::from_fn(|| commands.try_recv().ok()).all(|c| !matches!(c, Command::Act { .. })),
        "cancelling never reaches the agent"
    );
    click(&mut visual, "end-session");
    click(&mut visual, "confirm-end");
    assert!(
        std::iter::from_fn(|| commands.try_recv().ok()).any(|c| matches!(
            c,
            Command::Act { ref session, action: Action::Terminate } if session == "a"
        )),
        "confirming ends exactly the selected session"
    );
}

#[test]
fn working_badges_understand_daemon_modes_and_pending_states() {
    let p = Appearance::Dark.palette();
    let mut session = Session {
        state: "responding".into(),
        ..Default::default()
    };
    assert_eq!(session_status(&session, p).0, "Working");
    assert!(session.working());
    session.approval = Some(serde_json::json!({"toolName":"Bash"}));
    assert_eq!(session_status(&session, p).0, "Needs approval");
    assert!(!session.working());
    session.state = "stopped".into();
    assert_eq!(session_status(&session, p).0, "Ended");
    assert!(!session.working());
    session.approval = None;
    session.state = "input".into();
    assert_eq!(session_status(&session, p).0, "Ready");
    assert!(!session.working());
}

#[gpui::test]
fn reselecting_the_same_session_updates_list_even_when_revisions_collide(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            for count in [1, 4, 2] {
                let mut view = state("a");
                view.transcript.snapshot(ConversationSnapshot {
                    seq: count,
                    first_seq: 1,
                    items: (0..count)
                        .map(|_| Item {
                            kind: "assistant_text".into(),
                            text: "Fresh snapshot".into(),
                            ..Default::default()
                        })
                        .collect(),
                });
                assert_eq!(view.transcript.revision, 1);
                this.update_view(Arc::new(view), window, cx);
                assert_eq!(this.list.item_count(), count as usize);
            }
        })
    });
}

#[gpui::test]
fn sidebar_usage_lists_unmeasured_logins_and_read_failures(cx: &mut TestAppContext) {
    let (workspace, mut visual, _, _) = fixture(cx);
    let now = chrono::Utc::now().timestamp();
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            let mut next = state("a");
            next.usage = Some(Arc::new(serde_json::json!({"providers":[
                {"provider":"claude","accounts":[{"label":"default","is_default":true,"source":"oauth_poll",
                    "failure":{"kind":"needs_reauth","detail":"oauth token expired"},
                    "windows":{"five_hour":{"used_percent":{"state":"unknown","reason":"NeedsReauth"}}}}]},
                {"provider":"codex","accounts":[{"label":"pro","is_default":true,"windows":{
                    "seven_day":{"used_percent":{"state":"ok","value":9.0},"resets_at":now + 86_400,"is_current":true}}}]}
            ]})));
            this.update_view(Arc::new(next), window, cx)
        })
    });
    visual.run_until_parked();
    let strip = visual.debug_bounds("sidebar-usage").unwrap();
    let claude = visual.debug_bounds("sidebar-usage-unmeasured-0").unwrap();
    assert!(strip.contains(&claude.center()));
    // The measured Codex row sits below it, so both providers show.
    assert!(strip.size.height > claude.size.height + px(8.));
    // A failed read with nothing cached says so instead of vanishing.
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            let mut next = state("a");
            next.usage_error = Some("usage report did not arrive".into());
            this.update_view(Arc::new(next), window, cx)
        })
    });
    visual.run_until_parked();
    assert!(visual.debug_bounds("sidebar-usage-error").is_some());
}
