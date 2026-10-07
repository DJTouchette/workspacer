//! Session history: grouped by project, searchable on the page, keyboard
//! driven, with the All / Archived toggle and per-group folding.
use super::*;
use serde_json::json;

/// What the hub's `sessions.recent` answers (newest first, as it sorts).
fn recent_rows() -> serde_json::Value {
    let row = |id: &str, name: &str, cwd: &str, provider: &str, mode: &str, at: i64| {
        json!({"sessionId": id, "name": name, "cwd": cwd, "provider": provider,
               "mode": mode, "transport": "stream", "updatedAt": at, "model": ""})
    };
    json!([
        row(
            "w1",
            "Fix sidebar",
            "/home/u/.workspacer/worktrees/shop/fix-sidebar",
            "claude",
            "stopped",
            900
        ),
        row(
            "n1",
            "Release notes",
            "/srv/notes",
            "claude",
            "stopped",
            800
        ),
        row("s1", "Checkout flow", "/work/shop", "codex", "input", 700),
        row(
            "s2",
            "Payments API",
            "/work/shop/api",
            "claude",
            "stopped",
            600
        ),
        row(
            "live-child",
            "Worker",
            "/trees/elsewhere/worker",
            "claude",
            "input",
            500
        ),
        row("o1", "Scratch", "/tmp/scratch", "claude", "stopped", 100),
    ])
}

/// A workspace on Session history with the rows above, the registry
/// naming `/work/shop` "Shop", and a live manager in `/srv/notes` whose
/// worker runs in an unrelated worktree root.
fn history_fixture(
    cx: &mut TestAppContext,
) -> (
    Entity<Workspace>,
    VisualTestContext,
    tokio::sync::mpsc::Receiver<Command>,
) {
    let (workspace, mut visual, commands, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.projects.registry = Some(Arc::new(json!({
                "projects": {"/work/shop": {"label": "Shop", "lastOpened": 5}},
                "favourites": [], "recent": [], "configured": [],
                "worktreeRoot": "/trees",
            })));
            let mut view = state("a");
            let sessions = Arc::make_mut(&mut view.sessions);
            sessions.push(Session {
                id: "manager".into(),
                label: "Manager".into(),
                cwd: "/srv/notes".into(),
                state: "input".into(),
                ..Default::default()
            });
            sessions.push(Session {
                id: "live-child".into(),
                label: "Worker".into(),
                cwd: "/trees/elsewhere/worker".into(),
                parent_session_id: "manager".into(),
                model: "claude-opus-5-5".into(),
                state: "input".into(),
                ..Default::default()
            });
            this.update_view(Arc::new(view), window, cx);
            this.open_feature(Screen::Recent, window, cx);
        })
    });
    request_state(&workspace, &mut visual, Request::Recent, 1, recent_rows());
    (workspace, visual, commands)
}

fn page(workspace: &Entity<Workspace>, visual: &VisualTestContext) -> Vec<(String, Vec<String>)> {
    workspace.read_with(visual, |this, cx| {
        this.recent_page(cx)
            .groups
            .iter()
            .map(|g| {
                (
                    g.name.clone(),
                    g.entries.iter().map(|e| e.session.id.clone()).collect(),
                )
            })
            .collect()
    })
}

fn names(groups: &[(&str, &[&str])]) -> Vec<(String, Vec<String>)> {
    groups
        .iter()
        .map(|(name, ids)| {
            (
                name.to_string(),
                ids.iter().map(|id| id.to_string()).collect(),
            )
        })
        .collect()
}

#[gpui::test]
fn history_groups_sessions_by_project_newest_first(cx: &mut TestAppContext) {
    let (workspace, mut visual, _) = history_fixture(cx);
    assert_eq!(
        page(&workspace, &visual),
        names(&[
            // The hub worktree of `shop` and a subfolder join the project.
            ("Shop", &["w1", "s1", "s2"]),
            // A live worker in another root counts toward its manager's folder.
            ("notes", &["n1", "live-child"]),
            ("scratch", &["o1"]),
        ])
    );
    for selector in [
        "history-search",
        "history-group-0",
        "history-group-2",
        "recent-row-0",
        "recent-row-5",
    ] {
        bounds_of(&mut visual, selector);
    }
    // Opening the page puts the cursor in its search field.
    let focused = workspace.read_with(&visual, |this, _| this.extras.recent.query.clone());
    visual.update(|window, cx| {
        assert!(focused.read(cx).focus_handle(cx).is_focused(window));
    });
}

#[gpui::test]
fn history_search_matches_any_field_and_esc_clears_then_leaves(cx: &mut TestAppContext) {
    let (workspace, mut visual, _) = history_fixture(cx);
    visual.simulate_input("PAYMENTS");
    visual.run_until_parked();
    assert_eq!(page(&workspace, &visual), names(&[("Shop", &["s2"])]));
    let summary = workspace.read_with(&visual, |this, cx| {
        let page = this.recent_page(cx);
        (page.matches(), page.total, page.groups[0].total)
    });
    assert_eq!(summary, (1, 6, 3), "1 match of 6, \"1 of 3\" in Shop");
    let search = |q: &str, visual: &mut VisualTestContext| {
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.extras
                    .recent
                    .query
                    .update(cx, |input, cx| input.set_value(q.to_owned(), window, cx))
            })
        });
        visual.run_until_parked();
        page(&workspace, visual)
    };
    assert_eq!(
        search("shop", &mut visual),
        names(&[("Shop", &["w1", "s1", "s2"])]),
        "project name"
    );
    assert_eq!(
        search("codex", &mut visual),
        names(&[("Shop", &["s1"])]),
        "provider"
    );
    assert_eq!(
        search("opus", &mut visual),
        names(&[("notes", &["live-child"])]),
        "model the live fleet knows"
    );
    assert_eq!(
        search("srv notes", &mut visual),
        names(&[("notes", &["n1", "live-child"])]),
        "folder, every term"
    );
    assert!(search("nothing-like-this", &mut visual).is_empty());
    bounds_of(&mut visual, "history-summary");
    press(&mut visual, "escape");
    workspace.read_with(&visual, |this, cx| {
        assert!(this.extras.recent.query.read(cx).value().is_empty());
        assert_eq!(this.screen, Screen::Recent);
    });
    assert_eq!(page(&workspace, &visual).len(), 3);
    press(&mut visual, "escape");
    workspace.read_with(&visual, |this, _| {
        assert_eq!(this.screen, Screen::Conversation)
    });
}

#[gpui::test]
fn history_archived_toggle_and_search_work_together(cx: &mut TestAppContext) {
    let (workspace, mut visual, _) = history_fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            for id in ["s2", "o1"] {
                this.extras.archive_pending.insert(id.into(), true);
            }
            this.extras
                .recent
                .query
                .update(cx, |input, cx| input.set_value("api", window, cx));
        })
    });
    visual.run_until_parked();
    // The archived Payments API is not under All sessions.
    assert!(page(&workspace, &visual).is_empty());
    visual.update(|_, cx| {
        workspace.update(cx, |this, cx| {
            this.extras.show_archived = true;
            cx.notify();
        })
    });
    visual.run_until_parked();
    assert_eq!(page(&workspace, &visual), names(&[("Shop", &["s2"])]));
    let counts = workspace.read_with(&visual, |this, cx| {
        let page = this.recent_page(cx);
        (page.total, page.groups[0].total)
    });
    assert_eq!(counts, (2, 1), "counts are of archived sessions only");
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.extras
                .recent
                .query
                .update(cx, |input, cx| input.set_value("", window, cx))
        })
    });
    visual.run_until_parked();
    assert_eq!(
        page(&workspace, &visual),
        names(&[("Shop", &["s2"]), ("scratch", &["o1"])])
    );
}

#[gpui::test]
fn history_keyboard_cursor_opens_the_highlighted_session(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands) = history_fixture(cx);
    let _ = effects(&mut commands);
    // Arrows move the cursor while the search field keeps focus.
    press(&mut visual, "down");
    press(&mut visual, "down");
    press(&mut visual, "up");
    press(&mut visual, "down");
    workspace.read_with(&visual, |this, _| assert_eq!(this.extras.recent.cursor, 2));
    // Vim keys walk the same rows once focus leaves the field.
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.settings.vim_navigation = true;
            window.focus(&this.focus);
            cx.notify();
        })
    });
    visual.run_until_parked();
    press(&mut visual, "j");
    workspace.read_with(&visual, |this, _| assert_eq!(this.extras.recent.cursor, 3));
    press(&mut visual, "shift-g");
    workspace.read_with(&visual, |this, _| assert_eq!(this.extras.recent.cursor, 5));
    press(&mut visual, "g");
    press(&mut visual, "g");
    press(&mut visual, "k");
    press(&mut visual, "k");
    press(&mut visual, "k");
    workspace.read_with(&visual, |this, _| assert_eq!(this.extras.recent.cursor, 3));
    press(&mut visual, "k");
    press(&mut visual, "/");
    workspace.read_with(&visual, |this, cx| {
        assert_eq!(this.extras.recent.cursor, 2);
        assert!(
            this.extras.recent.query.read(cx).value().is_empty(),
            "/ typed nothing"
        );
    });
    press(&mut visual, "enter");
    let opened: Vec<String> = effects(&mut commands)
        .into_iter()
        .filter_map(|c| match c {
            Command::OpenRecent(s) => Some(s.id),
            _ => None,
        })
        .collect();
    assert_eq!(opened, ["s2"]);
    workspace.read_with(&visual, |this, _| {
        assert_eq!(this.screen, Screen::Conversation)
    });
}

#[gpui::test]
fn history_groups_fold_and_show_more_instead_of_truncating(cx: &mut TestAppContext) {
    let (workspace, mut visual, _, _updates) = fixture(cx);
    let rows: Vec<_> = (0..(recent::GROUP_ROWS + 3))
        .map(|i| {
            json!({"sessionId": format!("big-{i}"), "cwd": "/work/big", "mode": "stopped",
                        "updatedAt": 1000 - i as i64})
        })
        .chain([
            json!({"sessionId": "small", "cwd": "/work/small", "mode": "stopped", "updatedAt": 1}),
        ])
        .collect();
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(state("a")), window, cx);
            this.open_feature(Screen::Recent, window, cx);
        })
    });
    request_state(&workspace, &mut visual, Request::Recent, 1, json!(rows));
    // Tall enough for every row once the group shows them all.
    visual.simulate_resize(size(px(1000.), px(1400.)));
    visual.run_until_parked();
    let on_screen = |visual: &VisualTestContext| {
        workspace.read_with(visual, |this, cx| this.recent_page(cx).rows.len())
    };
    assert_eq!(on_screen(&visual), recent::GROUP_ROWS + 1);
    click(&mut visual, "history-more-0");
    assert_eq!(on_screen(&visual), recent::GROUP_ROWS + 4);
    // Folding a group takes its rows off the page and out of the cursor.
    click(&mut visual, "history-group-0");
    assert_eq!(on_screen(&visual), 1);
    // A search shows every group with a match, folded or not.
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.extras
                .recent
                .query
                .update(cx, |input, cx| input.set_value("big-2", window, cx))
        })
    });
    visual.run_until_parked();
    assert_eq!(on_screen(&visual), 1);
    bounds_of(&mut visual, "recent-row-0");
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.extras
                .recent
                .query
                .update(cx, |input, cx| input.focus(window, cx))
        })
    });
    press(&mut visual, "escape");
    workspace.read_with(&visual, |this, cx| {
        assert!(this.extras.recent.query.read(cx).value().is_empty())
    });
    click(&mut visual, "history-group-0");
    assert_eq!(on_screen(&visual), recent::GROUP_ROWS + 4);
}

#[test]
fn history_rows_name_only_where_they_differ_from_their_group() {
    let group = wks_native::history::Group {
        key: String::new(),
        name: "Shop".into(),
        path: "/work/shop".into(),
        kind: wks_native::history::GroupKind::Project,
        entries: Vec::new(),
        total: 0,
        updated_at: 0,
    };
    let place = |cwd: &str| recent::recent_place(cwd, &group, "/trees");
    assert_eq!(place("/work/shop/"), "");
    assert_eq!(place("/work/shop/api/v2"), "api/v2");
    assert_eq!(place("/trees/shop/fix"), "worktree fix");
    assert_eq!(
        place("/work/shop/.claude/worktrees/flaky"),
        "worktree flaky"
    );
    assert_eq!(place("/work/shopping"), "/work/shopping");
}
