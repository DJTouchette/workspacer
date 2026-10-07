//! Settings pages, hub settings, remote sharing, updates and alerts.
use super::*;

/// Settings → Agents shows the hub's child-agent access setting as read
/// (never assumed), and the switch asks the hub to change it.
#[gpui::test]
fn child_agent_full_access_is_a_hub_setting_toggled_from_agents(cx: &mut TestAppContext) {
    use wks_native::features::{Request, RequestState};
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.demo = false;
            this.update_view(Arc::new(state("a")), window, cx);
            this.show_screen(Screen::Settings, window, cx);
            this.settings_section = settings::SettingsSection::Agents;
        })
    });
    let read = std::iter::from_fn(|| commands.try_recv().ok())
        .any(|c| matches!(c, Command::Request(Request::ChildAccess { set: None })));
    assert!(read, "opening Settings reads the hub's setting");
    visual.run_until_parked();
    assert!(visual.debug_bounds("setting-child-access").is_some());
    // The Agents card is taller than the default test window.
    visual.simulate_resize(size(px(1000.), px(1600.)));
    workspace.read_with(&visual, |this, _| {
        assert_eq!(this.extras.child_access, None)
    });
    let mut next = state("a");
    next.requests.insert(
        "child-access",
        RequestState {
            number: 1,
            request: Request::ChildAccess { set: None },
            loading: false,
            value: Arc::new(serde_json::json!({"childFullAccess":false,"fleetFullAccess":true})),
            error: None,
        },
    );
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| this.update_view(Arc::new(next), window, cx))
    });
    visual.run_until_parked();
    workspace.read_with(&visual, |this, _| {
        assert_eq!(this.extras.child_access, Some((false, true)))
    });
    let switch = visual.debug_bounds("child-access-switch").unwrap();
    visual.simulate_click(
        gpui::point(switch.left() + px(12.), switch.center().y),
        gpui::Modifiers::none(),
    );
    visual.run_until_parked();
    let set = std::iter::from_fn(|| commands.try_recv().ok()).find_map(|c| match c {
        Command::Request(Request::ChildAccess { set }) => Some(set),
        _ => None,
    });
    assert_eq!(set, Some(Some(true)));
    // A refused save leaves the last confirmed value in place.
    let mut refused = state("a");
    refused.requests.insert(
        "child-access",
        RequestState {
            number: 2,
            request: Request::ChildAccess { set: Some(true) },
            loading: false,
            value: Arc::new(serde_json::Value::Null),
            error: Some("operator scope required".into()),
        },
    );
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(refused), window, cx);
            assert_eq!(this.extras.child_access, Some((false, true)));
        })
    });
}

/// Settings → Workspace offers the hub host's shells for new terminals,
/// marks the hub's configured one, saves a pick to the hub, and keeps the
/// last confirmed choice when the hub refuses.
#[gpui::test]
fn default_terminal_shell_is_a_hub_setting_picked_from_its_hosts_shells(cx: &mut TestAppContext) {
    use wks_native::features::{Request, RequestState};
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.demo = false;
            this.update_view(Arc::new(state("a")), window, cx);
            this.show_screen(Screen::Settings, window, cx);
            this.settings_section = settings::SettingsSection::Workspace;
        })
    });
    let read = std::iter::from_fn(|| commands.try_recv().ok())
        .any(|c| matches!(c, Command::Request(Request::TerminalShell { set: None })));
    assert!(read, "opening Settings reads the hub's shells");
    visual.simulate_resize(size(px(1000.), px(1600.)));
    visual.run_until_parked();
    assert!(visual.debug_bounds("setting-terminal-shell").is_some());
    let shells = |number, request, value: serde_json::Value, error: Option<&str>| {
        let mut next = state("a");
        next.requests.insert(
            "terminal-shell",
            RequestState {
                number,
                request,
                loading: false,
                value: Arc::new(value),
                error: error.map(str::to_owned),
            },
        );
        Arc::new(next)
    };
    let listed = serde_json::json!({"shell":"wsl.exe","default":"powershell.exe","listError":null,"shells":[
        {"name":"default","path":"","label":"System default"},
        {"name":"powershell","path":"powershell.exe","label":"PowerShell"},
        {"name":"wsl","path":"wsl.exe","label":"WSL"},
        {"name":"gitbash","path":"C:\\Program Files\\Git\\bin\\bash.exe","label":"Git Bash"},
    ]});
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(
                shells(
                    1,
                    Request::TerminalShell { set: None },
                    listed.clone(),
                    None,
                ),
                window,
                cx,
            )
        })
    });
    visual.run_until_parked();
    workspace.read_with(&visual, |this, _| {
        assert_eq!(this.extras.terminal_shell.as_ref(), Some(&listed))
    });
    // Picking the configured shell again sends nothing.
    let wsl = visual.debug_bounds("terminal-shell-2").unwrap();
    visual.simulate_click(wsl.center(), gpui::Modifiers::none());
    visual.run_until_parked();
    assert!(
        !std::iter::from_fn(|| commands.try_recv().ok())
            .any(|c| matches!(c, Command::Request(Request::TerminalShell { set: Some(_) })))
    );
    let git = visual.debug_bounds("terminal-shell-3").unwrap();
    visual.simulate_click(git.center(), gpui::Modifiers::none());
    visual.run_until_parked();
    let set = std::iter::from_fn(|| commands.try_recv().ok()).find_map(|c| match c {
        Command::Request(Request::TerminalShell { set: Some(path) }) => Some(path),
        _ => None,
    });
    assert_eq!(set.as_deref(), Some(r"C:\Program Files\Git\bin\bash.exe"));
    // A refused save (not the hub's owner) keeps the confirmed choice and
    // says why.
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(
                shells(
                    2,
                    Request::TerminalShell {
                        set: Some(r"C:\Program Files\Git\bin\bash.exe".into()),
                    },
                    serde_json::Value::Null,
                    Some("desktop services require the authenticated server owner's connection"),
                ),
                window,
                cx,
            );
            assert_eq!(this.extras.terminal_shell.as_ref(), Some(&listed));
        })
    });
    visual.run_until_parked();
    assert!(visual.debug_bounds("terminal-shell-status").is_some());
}

/// Settings → Agents shows the hub's automatic-title settings as read,
/// changes one field per action, offers the edited harness's live
/// catalog (plus a configured ID it does not list, never substituted), and
/// keeps the last confirmed state when the hub refuses a change.
#[gpui::test]
fn automatic_titles_are_hub_settings_with_provider_and_catalog_model(cx: &mut TestAppContext) {
    use wks_native::features::{Request, RequestState, TitleChange};
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.demo = false;
            this.settings.default_provider = wks_native::navigation::Provider::Codex;
            this.update_view(Arc::new(state("a")), window, cx);
            this.show_screen(Screen::Settings, window, cx);
            this.settings_section = settings::SettingsSection::Agents;
        })
    });
    let opened: Vec<_> = std::iter::from_fn(|| commands.try_recv().ok()).collect();
    assert!(
        opened
            .iter()
            .any(|c| matches!(c, Command::Request(Request::Titles { set: None })))
    );
    // The default agent's harness is edited first, with its own catalog.
    assert!(opened.iter().any(|c| matches!(c, Command::LoadModels { key, .. } if key.provider == "codex" && key.cwd.is_empty())));
    visual.simulate_resize(size(px(1000.), px(1900.)));
    visual.run_until_parked();
    assert!(visual.debug_bounds("setting-auto-title").is_some());
    assert!(visual.debug_bounds("setting-title-model").is_some());
    let titles = |number, value: serde_json::Value, error: Option<&str>| {
        let mut next = state("a");
        next.catalog = wks_native::launch::Catalog {
            key: CatalogKey {
                provider: "codex".into(),
                cwd: String::new(),
            },
            models: vec![ModelChoice {
                id: "gpt-5.4-mini".into(),
                label: "GPT-5.4 Mini".into(),
                is_default: true,
                ..Default::default()
            }],
            ..Default::default()
        };
        next.requests.insert(
            "titles",
            RequestState {
                number,
                request: Request::Titles { set: None },
                loading: false,
                value: Arc::new(value),
                error: error.map(str::to_owned),
            },
        );
        Arc::new(next)
    };
    let read = serde_json::json!({"enabled":true,"provider":"","models":{"codex":"gpt-legacy-pinned"},"legacyModel":"haiku"});
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(titles(1, read.clone(), None), window, cx)
        })
    });
    visual.run_until_parked();
    workspace.read_with(&visual, |this, cx| {
        assert_eq!(this.extras.titles.as_ref(), Some(&read));
        assert_eq!(this.title_harness(), "codex");
        let picker = this.extras.title_picker.read(cx);
        // The configured ID is shown as chosen even though the catalog
        // does not list it — never silently swapped for the default.
        assert_eq!(
            picker.selected_value().map(String::as_str),
            Some("gpt-legacy-pinned")
        );
    });
    // Pin titles to Codex: one field, verified by the hub.
    let pin = visual.debug_bounds("title-provider-2").unwrap();
    visual.simulate_click(pin.center(), gpui::Modifiers::none());
    visual.run_until_parked();
    let sent: Vec<_> = std::iter::from_fn(|| commands.try_recv().ok()).collect();
    assert!(sent.iter().any(|c| matches!(c, Command::Request(Request::Titles { set: Some(TitleChange::Provider(p)) }) if p == "codex")));
    // Choose the catalog model from the dropdown.
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.extras
                .title_picker
                .update(cx, |picker, cx| picker.focus(window, cx));
        })
    });
    // The menu opens on the configured row (listed last); one row up is
    // the catalog model.
    visual.simulate_keystrokes("enter up enter");
    visual.run_until_parked();
    let picked = std::iter::from_fn(|| commands.try_recv().ok()).find_map(|c| match c {
        Command::Request(Request::Titles { set: Some(change) }) => Some(change),
        _ => None,
    });
    assert_eq!(
        picked,
        Some(TitleChange::Model {
            provider: "codex".into(),
            model: "gpt-5.4-mini".into()
        })
    );
    // The switch turns titles off on the hub.
    let switch = visual.debug_bounds("auto-title-switch").unwrap();
    visual.simulate_click(
        gpui::point(switch.left() + px(12.), switch.center().y),
        gpui::Modifiers::none(),
    );
    visual.run_until_parked();
    let off = std::iter::from_fn(|| commands.try_recv().ok()).find_map(|c| match c {
        Command::Request(Request::Titles { set: Some(change) }) => Some(change),
        _ => None,
    });
    assert_eq!(off, Some(TitleChange::Enabled(false)));
    // A refused change keeps the last confirmed settings and says why.
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(
                titles(2, serde_json::Value::Null, Some("config busy")),
                window,
                cx,
            );
            assert_eq!(this.extras.titles.as_ref(), Some(&read));
        })
    });
    visual.run_until_parked();
    assert!(visual.debug_bounds("title-status").is_some());
}

#[gpui::test]
fn settings_categories_and_search_filter_preferences(cx: &mut TestAppContext) {
    let (workspace, mut visual, _, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(state("a")), window, cx)
        })
    });
    visual.simulate_keystrokes("g s");
    visual.run_until_parked();
    // One category at a time; Appearance first.
    assert!(visual.debug_bounds("setting-theme").is_some());
    assert!(visual.debug_bounds("setting-vim").is_none());
    // j / k step through categories in Normal mode.
    visual.simulate_keystrokes("j");
    visual.run_until_parked();
    assert!(visual.debug_bounds("setting-interface-font").is_some());
    assert!(visual.debug_bounds("setting-theme").is_none());
    let keyboard = visual.debug_bounds("settings-nav-Keyboard").unwrap();
    visual.simulate_click(keyboard.center(), gpui::Modifiers::default());
    visual.run_until_parked();
    assert!(visual.debug_bounds("setting-vim").is_some());
    // `/` searches every category at once.
    visual.simulate_keystrokes("/");
    visual.simulate_input("clock");
    visual.run_until_parked();
    // Results span categories (debug bounds outlive removed rows, so
    // filtering itself is pinned by `settings_match`'s unit test).
    assert!(visual.debug_bounds("setting-clock").is_some());
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.settings_search
                .update(cx, |input, cx| input.set_value("font mono", window, cx));
            cx.notify();
        })
    });
    visual.run_until_parked();
    assert!(visual.debug_bounds("setting-code-font").is_some());
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.settings_search
                .update(cx, |input, cx| input.set_value("zzzz", window, cx));
            cx.notify();
        })
    });
    visual.run_until_parked();
    assert!(visual.debug_bounds("settings-no-results").is_some());
    // Picking a category clears the search.
    let chat = visual.debug_bounds("settings-nav-Chat").unwrap();
    visual.simulate_click(chat.center(), gpui::Modifiers::default());
    visual.run_until_parked();
    assert!(visual.debug_bounds("setting-merge-turn").is_some());
    workspace.read_with(&visual, |this, cx| {
        assert!(this.settings_search.read(cx).value().is_empty())
    });
}

#[gpui::test]
fn settings_shortcuts_update_defaults_without_touching_sessions(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.simulate_keystrokes("g s a v");
    workspace.read_with(&visual, |this, _| {
        assert_eq!(this.settings.default_provider, Provider::Codex);
        assert!(!this.settings.vim_navigation);
    });
    assert!(commands.try_recv().is_err());
    visual.simulate_keystrokes("escape g p");
    workspace.read_with(&visual, |this, _| {
        assert_eq!(this.screen, Screen::Conversation)
    });
    visual.simulate_keystrokes("ctrl-p");
    workspace.read_with(&visual, |this, _| assert_eq!(this.screen, Screen::Projects));
}

#[gpui::test]
fn narrow_settings_rail_keeps_every_category_reachable(cx: &mut TestAppContext) {
    let (workspace, mut visual, _, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(state("a")), window, cx);
            this.settings.sidebar_width = 304.;
            this.show_screen(Screen::Settings, window, cx);
        })
    });
    visual.simulate_resize(size(px(760.), px(600.)));
    visual.run_until_parked();
    let rail = visual.debug_bounds("settings-rail").unwrap();
    assert_eq!(rail.size.width, px(40.), "icon rail beside a wide sidebar");
    let keyboard = visual.debug_bounds("settings-nav-Keyboard").unwrap();
    assert!(keyboard.size.width > px(0.) && rail.contains(&keyboard.center()));
    visual.simulate_click(keyboard.center(), gpui::Modifiers::default());
    visual.run_until_parked();
    workspace.read_with(&visual, |this, _| {
        assert_eq!(this.settings_section, settings::SettingsSection::Keyboard)
    });
    visual.simulate_resize(size(px(1400.), px(800.)));
    visual.run_until_parked();
    assert_eq!(
        visual.debug_bounds("settings-rail").unwrap().size.width,
        px(200.),
        "labelled rail when there is room"
    );
}

// A background session that finishes its turn raises one OS alert. The
// test build records alerts instead of posting them (see
// `post_attention_alerts`): real WinRT toasts from successive test
// threads crashed the serial Windows suite.
#[gpui::test]
fn background_turn_end_alerts_once_without_posting_a_real_toast(cx: &mut TestAppContext) {
    features::POSTED_ALERTS.take();
    let (workspace, mut visual, _, _updates) = fixture(cx);
    let show = |visual: &mut VisualTestContext, session_state: &str| {
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                let mut next = state("b");
                Arc::make_mut(&mut next.sessions)[0].state = session_state.into();
                this.update_view(Arc::new(next), window, cx);
            })
        });
        visual.run_until_parked();
        features::POSTED_ALERTS.take()
    };
    assert!(
        show(&mut visual, "input").is_empty(),
        "the first view is no transition"
    );
    assert!(
        show(&mut visual, "responding").is_empty(),
        "starting work is quiet"
    );
    assert_eq!(
        show(&mut visual, "input"),
        [("Work completed".to_owned(), "Alpha".to_owned())],
        "the finished turn alerts, named for its session"
    );
    assert!(
        show(&mut visual, "input").is_empty(),
        "one alert per transition"
    );
    visual.update(|window, cx| {
        assert!(
            !window.is_window_active(),
            "alerts are for a background window"
        );
        workspace.update(cx, |this, _| this.settings.notifications = false)
    });
    show(&mut visual, "responding");
    assert!(
        show(&mut visual, "input").is_empty(),
        "notifications off: no alert"
    );
}

#[gpui::test]
fn available_update_shows_a_pill_and_installs_from_about(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    let asset = serde_json::json!({"name":"Workspacer-Native-Rust-Preview-Setup-0.170.0-nightly.1-x64.exe",
        "version":"0.170.0-nightly.1","size":10,"url":"https://github.com/DJTouchette/workspacer/releases/download/nightly/x.exe"});
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            let mut next = state("a");
            next.requests.insert(
                "updates",
                wks_native::features::RequestState {
                    number: 1,
                    request: wks_native::features::Request::Updates,
                    loading: false,
                    value: Arc::new(serde_json::json!({
                        "channel":"nightly","installed":"0.169.0-nightly.1",
                        "latest":"0.170.0-nightly.1","update_available":true,
                        "installable":true,"asset":asset.clone()
                    })),
                    error: None,
                },
            );
            this.update_view(Arc::new(next), window, cx)
        })
    });
    visual.run_until_parked();
    let pill = visual.debug_bounds("update-pill").unwrap();
    visual.simulate_click(pill.center(), gpui::Modifiers::default());
    visual.run_until_parked();
    workspace.read_with(&visual, |this, _| {
        assert_eq!(this.screen, Screen::Settings);
        assert_eq!(this.settings_section, settings::SettingsSection::About);
    });
    let install = visual.debug_bounds("install-update").unwrap();
    visual.simulate_click(install.center(), gpui::Modifiers::default());
    visual.run_until_parked();
    let requested = std::iter::from_fn(|| commands.try_recv().ok()).find_map(|c| match c {
        Command::Request(wks_native::features::Request::DownloadUpdate { asset }) => Some(asset),
        _ => None,
    });
    assert_eq!(requested, Some(asset));
}

#[gpui::test]
fn verified_update_hands_off_only_without_unsaved_edits(cx: &mut TestAppContext) {
    use wks_native::features::Request;
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    let started: Arc<std::sync::Mutex<Vec<wks_native::updates::Handoff>>> = Default::default();
    let fail = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let helper_alive = Arc::new(std::sync::atomic::AtomicBool::new(true));
    workspace.update(&mut visual, |this, _| {
        let helper_alive = helper_alive.clone();
        let (started, fail) = (started.clone(), fail.clone());
        this.extras.update_starter = Arc::new(move |handoff| {
            started.lock().unwrap().push(handoff.clone());
            if fail.load(std::sync::atomic::Ordering::SeqCst) {
                anyhow::bail!("helper exited before it was ready")
            }
            let alive = helper_alive.clone();
            Ok(wks_native::updates::ReadyHelper::for_test(move || {
                Ok(alive.load(std::sync::atomic::Ordering::SeqCst))
            }))
        });
    });
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
        lib,
        1,
        false,
        None,
        serde_json::json!({"contents": "one\n", "size": 4}),
    );
    settle(&mut visual);
    let pane = pane_of(&workspace, &visual);
    visual.simulate_input("X");
    visual.run_until_parked();
    assert!(pane.read_with(&visual, |p, _| p.dirty()));
    let notice = |visual: &mut VisualTestContext| {
        workspace.read_with(visual, |this, _| this.extras.update_notice.clone())
    };

    // A verified download never closes over unsaved edits.
    let installer = "/tmp/update/Workspacer-Native-Rust-Preview-Setup-0.170.0-x64.exe";
    request_state(
        &workspace,
        &mut visual,
        Request::DownloadUpdate {
            asset: serde_json::json!({"version": "0.170.0"}),
        },
        2,
        serde_json::json!({"installer": installer, "version": "0.170.0"}),
    );
    assert!(
        started.lock().unwrap().is_empty(),
        "no helper over unsaved edits"
    );
    assert!(notice(&mut visual).starts_with("Save or discard your unsaved edits"));
    assert!(pane.read_with(&visual, |p, _| p.dirty()), "edits untouched");

    // Once saved, Install hands off the same verified file; a helper that
    // never becomes ready keeps the app open and says why.
    visual.simulate_keystrokes("ctrl-s");
    visual.run_until_parked();
    request_state(
        &workspace,
        &mut visual,
        Request::SaveFile {
            session: "a".into(),
            path: "/repo/src/lib.rs".into(),
            contents: "Xone\n".into(),
            base: "one\n".into(),
            force: false,
        },
        5,
        serde_json::json!({"saved": true, "contents": "Xone\n"}),
    );
    assert!(!pane.read_with(&visual, |p, _| p.dirty()));
    let _ = effects(&mut commands);
    visual.update(|window, cx| workspace.update(cx, |this, cx| this.install_update(window, cx)));
    visual.run_until_parked();
    {
        let started = started.lock().unwrap();
        assert_eq!(started.len(), 1);
        let handoff = &started[0];
        assert_eq!(handoff.installer, std::path::PathBuf::from(installer));
        assert_eq!(handoff.expected_version, "0.170.0");
        assert_eq!(handoff.pid, std::process::id());
        assert_eq!(
            handoff.args,
            std::env::args().skip(1).collect::<Vec<_>>(),
            "relaunch keeps the original arguments"
        );
    }
    assert_eq!(
        notice(&mut visual),
        "Update could not start: helper exited before it was ready"
    );
    workspace.read_with(&visual, |this, _| assert!(!this.extras.update_handing_off));

    // Retrying reuses the download and closes once the helper is ready.
    fail.store(false, std::sync::atomic::Ordering::SeqCst);
    visual.update(|window, cx| workspace.update(cx, |this, cx| this.install_update(window, cx)));
    visual.run_until_parked();
    assert_eq!(started.lock().unwrap().len(), 2);
    assert_eq!(notice(&mut visual), "Closing to install 0.170.0…");
    assert!(
        !effects(&mut commands)
            .iter()
            .any(|c| matches!(c, Command::Request(Request::DownloadUpdate { .. }))),
        "never downloaded again"
    );

    // While that helper waits, Install only closes again: new unsaved
    // edits get the editor's question, and no second helper starts.
    visual.simulate_input("Q");
    visual.run_until_parked();
    assert!(pane.read_with(&visual, |p, _| p.dirty()));
    visual.update(|window, cx| workspace.update(cx, |this, cx| this.install_update(window, cx)));
    visual.run_until_parked();
    assert_eq!(started.lock().unwrap().len(), 2, "one helper at a time");
    assert!(
        visual.debug_bounds("file-viewer-unsaved").is_some(),
        "asked first"
    );
    assert!(notice(&mut visual).starts_with("Save or discard your unsaved edits to finish"));
    assert!(pane.read_with(&visual, |p, _| p.dirty()), "edits untouched");
    helper_alive.store(false, std::sync::atomic::Ordering::SeqCst);
    visual.update(|window, cx| workspace.update(cx, |this, cx| this.install_update(window, cx)));
    visual.run_until_parked();
    assert_eq!(
        started.lock().unwrap().len(),
        2,
        "no second helper after an ambiguous/stale receipt"
    );
    assert!(notice(&mut visual).contains("no longer confirms readiness"));
    click(&mut visual, "file-viewer-prompt-discard");
    visual.run_until_parked();
    assert_eq!(
        cx.windows().len(),
        1,
        "a stale helper must not turn Discard into a blind quit"
    );
    assert!(notice(&mut visual).contains("no longer confirms readiness"));
    // A deleted temporary installer clears only the download cache so a
    // subsequent Install can fetch it again instead of retrying forever.
    workspace.update(&mut visual, |this, _| {
        this.extras.update_helper_ready = None;
        this.extras.update_starter =
            Arc::new(|_| Err(wks_native::updates::MissingInstaller.into()));
    });
    visual.update(|window, cx| workspace.update(cx, |this, cx| this.install_update(window, cx)));
    visual.run_until_parked();
    workspace.read_with(&visual, |this, _| {
        assert!(this.extras.update_installer.is_none())
    });
    assert!(notice(&mut visual).contains("download it again"));
}

#[gpui::test]
fn remote_settings_pair_and_revoke_through_owner_requests(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    // Tall enough that the pairing list is on screen without scrolling.
    visual.simulate_resize(size(px(1000.), px(1400.)));
    let remote = |value: serde_json::Value| wks_native::features::RequestState {
        number: 1,
        request: wks_native::features::Request::Remote,
        loading: false,
        value: Arc::new(value),
        error: None,
    };
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.show_screen(Screen::Settings, window, cx);
            this.enter_settings_section(settings::SettingsSection::Remote, cx);
            let mut next = state("a");
            next.requests.insert(
                "remote",
                remote(serde_json::json!({
                    "tailscale":{"available":true,"magicName":"node.tailnet.ts.net",
                        "serveActive":true,"canServe":true},
                    "pairing":{"scope":"operator","canManageTokens":true},
                    "tokens":[{"token":"t-view","scope":"view",
                        "label":"Remote Control: view","created":"2026-10-02T10:00:00Z"}]
                })),
            );
            this.update_view(Arc::new(next), window, cx)
        })
    });
    visual.run_until_parked();
    let sent = |commands: &mut tokio::sync::mpsc::Receiver<Command>| {
        std::iter::from_fn(|| commands.try_recv().ok())
            .filter_map(|c| match c {
                Command::Request(request) => Some(request),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    assert!(
        sent(&mut commands)
            .iter()
            .any(|r| matches!(r, wks_native::features::Request::Remote)),
        "opening Remote reads live Tailscale state"
    );
    // Triage is the default and has no pairing yet.
    assert!(visual.debug_bounds("pairing-qr").is_none());
    let create = visual.debug_bounds("pairing-create").unwrap();
    visual.simulate_click(create.center(), gpui::Modifiers::default());
    visual.run_until_parked();
    assert!(sent(&mut commands).iter().any(|r| matches!(r,
        wks_native::features::Request::RemoteAction(wks_native::remote::Action::Pair(scope)) if scope == "triage")));

    visual.update(|_, cx| {
        workspace.update(cx, |this, cx| {
            this.remote.scope = "view";
            cx.notify();
        })
    });
    visual.run_until_parked();
    assert!(visual.debug_bounds("pairing-qr").is_some());
    let revoke = visual.debug_bounds("pairing-revoke-0").unwrap();
    visual.simulate_click(revoke.center(), gpui::Modifiers::default());
    visual.run_until_parked();
    workspace.read_with(&visual, |this, _| {
        assert_eq!(this.remote.confirm_revoke.as_deref(), Some("t-view"))
    });
    assert!(
        sent(&mut commands).is_empty(),
        "the first click only asks for confirmation"
    );
    let revoke = visual.debug_bounds("pairing-revoke-0").unwrap();
    visual.simulate_click(revoke.center(), gpui::Modifiers::default());
    visual.run_until_parked();
    assert!(sent(&mut commands).iter().any(|r| matches!(r,
        wks_native::features::Request::RemoteAction(wks_native::remote::Action::Revoke(token)) if token == "t-view")));
}
