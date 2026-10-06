//! Projects: the shared registry, navigation and bookmarks.
use super::*;

/// `state("a")` with both sessions working in `cwd`.
fn state_at(cwd: &str) -> View {
    let mut view = state("a");
    for session in Arc::make_mut(&mut view.sessions) {
        session.cwd = cwd.into();
    }
    view
}

#[gpui::test]
fn project_navigation_filters_and_seeds_new_sessions(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            let mut view = state("a");
            let rows = Arc::make_mut(&mut view.sessions);
            rows[0].cwd = "/one/app".into();
            rows[1].cwd = "/two/app".into();
            this.demo = false;
            this.settings.default_provider = Provider::Codex;
            this.update_view(Arc::new(view), window, cx);
        })
    });
    visual.simulate_keystrokes("g p j enter");
    assert!(matches!(next_effect(&mut commands).unwrap(), Command::Select(id) if id == "b"));
    workspace.read_with(&visual, |this, cx| {
        assert_eq!(this.project_filter.as_deref(), Some("/two/app"));
        assert_eq!(this.visible_sessions(cx), vec![1]);
    });
    visual.simulate_keystrokes("n");
    workspace.read_with(&visual, |this, _| {
        assert!(this.new_session);
        assert_eq!(this.provider, "codex");
        assert_eq!(this.projects.cwd.as_str(), "/two/app");
    });
    visual.simulate_input("jkgn");
    workspace.read_with(&visual, |this, cx| {
        assert!(this.prompt.read(cx).value().ends_with("jkgn"))
    });
    assert!(
        next_effect(&mut commands).is_none(),
        "typing in form must not launch or select agents"
    );
}

/// A project with agents already running still offers New agent (row
/// button, `n` on the highlighted row, the sidebar filter's +), which only
/// opens the form on that folder; opening the project still reaches its
/// existing sessions.
#[gpui::test]
fn projects_with_running_agents_can_start_another(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            let mut view = state("a");
            let rows = Arc::make_mut(&mut view.sessions);
            rows[0].cwd = "/one/app".into();
            rows[0].state = "responding".into();
            rows[1].cwd = "/one/app".into();
            this.demo = false;
            this.update_view(Arc::new(view), window, cx);
        })
    });
    visual.simulate_keystrokes("g p");
    visual.run_until_parked();
    while commands.try_recv().is_ok() {}
    let button = visual
        .debug_bounds("new-agent-in-project-0")
        .expect("New agent on a project that already has agents");
    visual.simulate_click(button.center(), gpui::Modifiers::none());
    visual.run_until_parked();
    workspace.read_with(&visual, |this, cx| {
        assert!(this.new_session, "the New Agent form opens");
        assert_eq!(this.projects.cwd.as_str(), "/one/app");
        assert_eq!(this.project_filter.as_deref(), Some("/one/app"));
        assert_eq!(
            this.visible_sessions(cx),
            vec![0, 1],
            "both sessions listed"
        );
    });
    assert!(
        next_effect(&mut commands).is_none(),
        "nothing launches or switches until the form is confirmed"
    );
    // The existing sessions stay navigable from the filtered sidebar.
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.new_session = false;
            this.show_screen(Screen::Projects, window, cx);
        })
    });
    visual.simulate_keystrokes("enter");
    assert!(matches!(
        next_effect(&mut commands),
        Some(Command::Select(_))
    ));
    let plus = visual
        .debug_bounds("new-agent-in-filter")
        .expect("the project filter offers New agent");
    visual.simulate_click(plus.center(), gpui::Modifiers::none());
    visual.run_until_parked();
    workspace.read_with(&visual, |this, _| {
        assert!(this.new_session);
        assert_eq!(this.projects.cwd.as_str(), "/one/app");
    });
    assert!(next_effect(&mut commands).is_none());
    // `n` on Projects uses the highlighted row, not the open chat's folder.
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.new_session = false;
            this.project_filter = None;
            this.projects.cwd = "/elsewhere".into();
            this.show_screen(Screen::Projects, window, cx);
            window.focus(&this.focus);
        })
    });
    visual.simulate_keystrokes("n");
    workspace.read_with(&visual, |this, _| {
        assert!(this.new_session);
        assert_eq!(this.projects.cwd.as_str(), "/one/app");
    });
}

/// Projects → Edit name and icon writes the shared registry's identity
/// (a new icon URL leaves iconFile for the hub to fill), closes on the
/// verified save, and the downloaded icon then draws as the mark while
/// the name follows the session into the title bar.
#[gpui::test]
fn project_identity_edits_the_shared_registry_and_shows_everywhere(cx: &mut TestAppContext) {
    use base64::Engine;
    use wks_native::features::{Request, RequestState};
    use wks_native::projects::{Identity, Patch};
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    let registry = |projects: serde_json::Value, revision: u64| RequestState {
        number: revision,
        request: Request::Projects,
        loading: false,
        value: Arc::new(serde_json::json!({"projects":projects,"favourites":[],
            "recent":[],"configured":[],"revision":revision})),
        error: None,
    };
    let with = |state: RequestState, extra: Option<(&'static str, RequestState)>| {
        let mut view = state_at("/work/app");
        view.requests.insert("projects", state);
        if let Some((key, extra)) = extra {
            view.requests.insert(key, extra);
        }
        Arc::new(view)
    };
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.demo = false;
            this.update_view(
                with(
                    registry(serde_json::json!({"/work/app":{"favourite":true}}), 1),
                    None,
                ),
                window,
                cx,
            );
            this.show_screen(Screen::Projects, window, cx);
        })
    });
    visual.run_until_parked();
    while commands.try_recv().is_ok() {}
    let edit = visual
        .debug_bounds("edit-project-0")
        .expect("edit identity");
    visual.simulate_click(edit.center(), gpui::Modifiers::none());
    visual.run_until_parked();
    assert!(visual.debug_bounds("project-identity-editor").is_some());
    visual.simulate_input("My App");
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            let editor = this.projects.editor.as_ref().unwrap();
            editor
                .icon
                .update(cx, |i, cx| i.set_value("🦀", window, cx));
            editor.favicon.update(cx, |i, cx| {
                i.set_value("https://example.com/i.png", window, cx)
            });
        })
    });
    let save = visual.debug_bounds("project-identity-save").unwrap();
    visual.simulate_click(save.center(), gpui::Modifiers::none());
    let sent = std::iter::from_fn(|| commands.try_recv().ok()).find_map(|c| match c {
        Command::Request(Request::SaveProject { path, change }) => Some((path, change)),
        _ => None,
    });
    let identity = Identity {
        label: "My App".into(),
        icon: "🦀".into(),
        favicon: "https://example.com/i.png".into(),
        icon_file: String::new(),
    };
    assert_eq!(
        sent,
        Some(("/work/app".to_owned(), Patch::Identity(identity.clone())))
    );
    let file = "0123456789abcdef0123456789abcdef.png";
    let stored = serde_json::json!({"/work/app":{"favourite":true,"label":"My App",
        "icon":"🦀","favicon":"https://example.com/i.png","iconFile":file}});
    let saved = RequestState {
        number: 1,
        request: Request::SaveProject {
            path: "/work/app".into(),
            change: Patch::Identity(identity),
        },
        loading: false,
        value: registry(stored.clone(), 2).value,
        error: None,
    };
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(
                with(registry(stored.clone(), 1), Some(("project-save", saved))),
                window,
                cx,
            );
            assert!(this.projects.editor.is_none(), "closes once verified");
            assert_eq!(this.projects.notice, "Name and icon saved.");
            assert_eq!(this.project_name("/work/app/"), "My App");
        })
    });
    let wanted = std::iter::from_fn(|| commands.try_recv().ok()).find_map(|c| match c {
        Command::Request(Request::ProjectIcons { files }) => Some(files),
        _ => None,
    });
    assert_eq!(wanted, Some(vec![file.to_owned()]));
    assert!(visual.debug_bounds("project-icon-image").is_none());
    let mut png = std::io::Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(8, 8)
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    let icons = RequestState {
        number: 1,
        request: Request::ProjectIcons {
            files: vec![file.into()],
        },
        loading: false,
        value: Arc::new(serde_json::json!({file:{"png":
            base64::engine::general_purpose::STANDARD.encode(png.into_inner())}})),
        error: None,
    };
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(
                with(registry(stored.clone(), 2), Some(("project-icons", icons))),
                window,
                cx,
            )
        })
    });
    visual.run_until_parked();
    assert!(
        visual.debug_bounds("project-icon-image").is_some(),
        "the downloaded icon draws as the project mark"
    );
    // Unchanged form: nothing is sent; an unchanged URL keeps its file.
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.open_identity_editor("/work/app".into(), window, cx);
            while commands.try_recv().is_ok() {}
            this.save_identity(false, cx);
            assert_eq!(this.projects.notice, "No changes to save.");
            let editor = this.projects.editor.as_ref().unwrap();
            editor
                .name
                .update(cx, |i, cx| i.set_value("App", window, cx));
            this.save_identity(false, cx);
        })
    });
    let kept = std::iter::from_fn(|| commands.try_recv().ok()).find_map(|c| match c {
        Command::Request(Request::SaveProject {
            change: Patch::Identity(identity),
            ..
        }) => Some(identity),
        _ => None,
    });
    assert_eq!(
        kept.map(|i| (i.label, i.icon_file)),
        Some(("App".into(), file.into()))
    );
}

#[gpui::test]
fn project_bookmark_can_be_saved_without_launching_an_agent(cx: &mut TestAppContext) {
    use wks_native::features::{Request, RequestState};
    use wks_native::projects::Patch;
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.demo = false;
            this.update_view(Arc::new(state("a")), window, cx);
        })
    });
    visual.simulate_keystrokes("g p i");
    visual.simulate_input("/work/jk-project/");
    visual.simulate_keystrokes("ctrl-enter");
    workspace.read_with(&visual, |this, cx| {
        assert!(this.project_path.read(cx).value().is_empty());
        assert!(
            this.settings.bookmarks("test").is_empty(),
            "the hub is asked first"
        );
    });
    let Some(Command::Request(Request::SaveProject { path, change })) = next_effect(&mut commands)
    else {
        panic!("pinning asks the hub's registry")
    };
    assert_eq!(
        (path.as_str(), &change),
        ("/work/jk-project", &Patch::Pin(true))
    );
    assert!(next_effect(&mut commands).is_none(), "nothing launches");
    let receipt = |number: u64, error: Option<&str>, value: serde_json::Value| {
        let (path, change, error) = (path.clone(), change.clone(), error.map(str::to_owned));
        let workspace = workspace.clone();
        move |window: &mut Window, cx: &mut App| {
            workspace.update(cx, |this, cx| {
                let mut view = (*this.view).clone();
                view.requests.insert(
                    "project-save",
                    RequestState {
                        number,
                        request: Request::SaveProject { path, change },
                        loading: false,
                        value: Arc::new(value),
                        error,
                    },
                );
                this.update_view(Arc::new(view), window, cx);
            })
        }
    };
    // A refused write is never shown as saved; the device keeps it on request.
    visual.update(receipt(
        7,
        Some("operator scope required"),
        serde_json::Value::Null,
    ));
    visual.run_until_parked();
    workspace.read_with(&visual, |this, _| {
        assert!(this.projects.notice.contains("operator scope required"));
        assert_eq!(this.projects.fallback.as_deref(), Some("/work/jk-project"));
        assert!(
            !this
                .known_projects()
                .iter()
                .any(|p| p.path == "/work/jk-project")
        );
    });
    let keep = visual.debug_bounds("keep-on-device-projects").unwrap();
    visual.simulate_click(keep.center(), gpui::Modifiers::default());
    workspace.read_with(&visual, |this, _| {
        assert_eq!(this.settings.bookmarks("test"), ["/work/jk-project"]);
        assert!(
            this.known_projects()
                .iter()
                .any(|p| p.path == "/work/jk-project")
        );
    });
    // A verified hub save becomes the registry the list is drawn from.
    visual.update(receipt(
        8,
        None,
        serde_json::json!({"revision":1,"projects":{"/work/hub-only":{"favourite":true}},"favourites":[],"recent":[],"configured":[]}),
    ));
    workspace.read_with(&visual, |this, _| {
        assert_eq!(this.projects.notice, "Pinned.");
        let first = &this.known_projects()[0];
        assert_eq!(
            (first.path.as_str(), first.favourite),
            ("/work/hub-only", true)
        );
    });
}

#[gpui::test]
fn project_snapshot_revisions_beat_request_order_in_every_window(cx: &mut TestAppContext) {
    use wks_native::{
        features::{Request, RequestState},
        projects::Patch,
    };
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    let (other, mut second, _commands2, _updates2) = fixture(cx);
    let snapshot = |revision, pinned| {
        Arc::new(serde_json::json!({
            "revision":revision,"projects":{"/work/web":{"favourite":pinned,"lastOpened":10}},
            "favourites":[],"recent":[],"configured":[]
        }))
    };
    let read = |value| RequestState {
        number: 9,
        request: Request::Projects,
        loading: false,
        value,
        error: None,
    };
    let save = |number, change, value| RequestState {
        number,
        request: Request::SaveProject {
            path: "/work/web".into(),
            change,
        },
        loading: false,
        value,
        error: None,
    };
    let mut next = state("a");
    // Newer request 9 returned pre-save state; acknowledged save 6 is
    // authoritative. This is the review's Pinned + empty-star regression.
    next.requests.insert("projects", read(snapshot(1, false)));
    next.requests
        .insert("project-save", save(6, Patch::Pin(true), snapshot(2, true)));
    for (entity, visual) in [(&workspace, &mut visual), (&other, &mut second)] {
        visual.update(|window, cx| {
            entity.update(cx, |this, cx| {
                this.demo = false;
                this.show_new_session(window, cx);
                this.select_project("/work/web", window, cx);
                this.update_view(Arc::new(next.clone()), window, cx);
                assert!(this.known_project("/work/web").unwrap().favourite);
                assert_eq!(this.projects.notice, "Pinned.");
            })
        });
    }
    visual.run_until_parked();
    while commands.try_recv().is_ok() {}
    let pin = visual.debug_bounds("launch-project-pin").unwrap();
    visual.simulate_click(pin.center(), gpui::Modifiers::default());
    assert!(
        std::iter::from_fn(|| commands.try_recv().ok()).any(|command| matches!(
            command,
            Command::Request(Request::SaveProject {
                change: Patch::Pin(false),
                ..
            })
        )),
        "the selected summary offers Unpin after the acknowledged pin"
    );
    // A late old read cannot replace that acknowledged pin either.
    next.requests.remove("project-save");
    next.requests.insert(
        "projects",
        RequestState {
            number: 12,
            ..read(snapshot(1, false))
        },
    );
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(next.clone()), window, cx);
            assert!(this.known_project("/work/web").unwrap().favourite);
        })
    });
    // A later touch includes the unpin and must outlive an older save.
    let touched = snapshot(4, false);
    next.project_registry = Some(touched);
    next.requests.insert(
        "project-save",
        save(10, Patch::Pin(false), snapshot(3, false)),
    );
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(next.clone()), window, cx);
            assert!(!this.known_project("/work/web").unwrap().favourite);
            assert_eq!(this.projects.notice, "Unpinned.");
            assert_eq!(this.projects.registry_revision, 4);
        })
    });
    next.requests.insert(
        "project-save",
        save(
            11,
            Patch::Remove,
            Arc::new(serde_json::json!({"revision":5,"projects":{}})),
        ),
    );
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(next.clone()), window, cx);
            assert!(this.known_project("/work/web").is_none());
            assert_eq!(this.projects.notice, "Removed from projects.");
        })
    });
    next.requests.insert(
        "project-save",
        RequestState {
            error: Some("refused".into()),
            ..save(13, Patch::Pin(true), Arc::new(serde_json::Value::Null))
        },
    );
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(next.clone()), window, cx);
            assert!(this.projects.notice.contains("refused"));
            assert_eq!(this.projects.fallback.as_deref(), Some("/work/web"));
            assert!(this.known_project("/work/web").is_none());
        })
    });
}
