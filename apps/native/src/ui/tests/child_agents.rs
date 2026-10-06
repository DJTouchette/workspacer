//! Child agents: nesting under their parent, viewing, clearing and spawn receipts.
use super::*;

#[gpui::test]
fn sidebar_nests_sessions_and_opens_native_child_from_another_parent(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _) = fixture(cx);
    visual.update(|window, cx| workspace.update(cx, |this, cx| {
        let mut next = state("b");
        let sessions = Arc::make_mut(&mut next.sessions);
        sessions[0].subagents = serde_json::json!([{
            "id": "codex-child", "description": "Native research", "status": "running", "model": "gpt-5"
        }]);
        sessions.insert(0, Session { id: "grandchild".into(), parent_session_id: "child".into(), ..Default::default() });
        sessions.insert(0, Session { id: "child".into(), parent_session_id: "a".into(), ..Default::default() });
        this.update_view(Arc::new(next), window, cx);
        assert_eq!(this.visible_sessions(cx), vec![2, 0, 1, 3]);
    }));
    visual.run_until_parked();
    let parent = visual.debug_bounds("sidebar-session-0").unwrap();
    let native = visual.debug_bounds("sidebar-provider-1").unwrap();
    let child = visual.debug_bounds("sidebar-session-2").unwrap();
    let grandchild = visual.debug_bounds("sidebar-session-3").unwrap();
    assert!(native.left() > parent.left());
    assert!(child.left() > parent.left());
    assert!(grandchild.left() > child.left());
    assert!(
        parent.top() < native.top() && native.top() < child.top() && child.top() < grandchild.top()
    );
    // uniform_list gives every row the session-card height; a taller
    // child row would paint over its neighbours.
    assert!(parent.bottom() <= native.top() && native.bottom() <= child.top());
    // Workspacer-spawned children are sessions: they archive like parents.
    assert!(visual.debug_bounds("sidebar-archive-2").is_some());
    assert!(visual.debug_bounds("sidebar-archive-3").is_some());
    // Child rows carry the same brand model badge as session cards.
    assert!(visual.debug_bounds("sidebar-child-model-1").is_some());
    // A provider-native child opens as its own chat; the controller
    // selects its parent first.
    visual.simulate_click(native.center(), gpui::Modifiers::default());
    visual.run_until_parked();
    assert!(matches!(commands.try_recv().unwrap(),
        Command::ViewChild(Some(t)) if t.parent == "a" && t.agent == "codex-child"));
    assert!(commands.try_recv().is_err());

    visual.update(|_, cx| {
        workspace.update(cx, |this, cx| {
            this.sidebar_collapsed = true;
            cx.notify();
        })
    });
    visual.run_until_parked();
    let rail_child = visual.debug_bounds("sidebar-provider-1").unwrap();
    assert!(rail_child.right() <= px(56.));
    visual.simulate_click(rail_child.center(), gpui::Modifiers::default());
    visual.run_until_parked();
    assert!(matches!(commands.try_recv().unwrap(),
        Command::ViewChild(Some(t)) if t.parent == "a" && t.agent == "codex-child"));
}

#[gpui::test]
fn viewed_subagent_reads_as_its_own_chat_without_a_composer(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            let mut next = state("a");
            Arc::make_mut(&mut next.sessions)[0].subagents = serde_json::json!([{
                "id": "task-1", "description": "Audit the parser", "status": "running", "model": "claude-sonnet-4-6"
            }]);
            next.child = Some(wks_native::controller::ChildTarget {
                parent: "a".into(),
                agent: "task-1".into(),
            });
            next.transcript.snapshot(ConversationSnapshot {
                seq: 1,
                first_seq: 1,
                items: vec![Item {
                    kind: "assistant_text".into(),
                    text: "Reading the parser now.".into(),
                    ..Default::default()
                }],
            });
            this.update_view(Arc::new(next), window, cx)
        })
    });
    visual.run_until_parked();
    assert!(visual.debug_bounds("child-title-bar").is_some());
    assert!(visual.debug_bounds("child-read-only-bar").is_some());
    assert!(
        visual.debug_bounds("chat-composer").is_none(),
        "subagents take no input"
    );
    let back = visual.debug_bounds("child-back-parent").unwrap();
    visual.simulate_click(back.center(), gpui::Modifiers::default());
    visual.run_until_parked();
    assert!(matches!(
        commands.try_recv().unwrap(),
        Command::ViewChild(None)
    ));
}

#[gpui::test]
fn subagent_overview_holds_its_place_and_collapses_when_all_finish(cx: &mut TestAppContext) {
    let (workspace, mut visual, _, _updates) = fixture(cx);
    let say = |text: &str, at: &str| Item {
        kind: "assistant_text".into(),
        text: text.into(),
        timestamp: Some(format!("2026-10-02T10:00:{at}Z")),
        ..Default::default()
    };
    let ask = |text: &str, at: &str| Item {
        kind: "user_message".into(),
        text: text.into(),
        timestamp: Some(format!("2026-10-02T10:00:{at}Z")),
        ..Default::default()
    };
    let render = |items: Vec<Item>, status: &str, visual: &mut gpui::VisualTestContext| {
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                let mut next = state("a");
                Arc::make_mut(&mut next.sessions)[0].subagents = serde_json::json!([
                    {"id":"t1","description":"Audit the parser","status":status,"startedAt":"2026-10-02T10:00:15Z"},
                    {"id":"t2","description":"Check the tests","status":status,"startedAt":"2026-10-02T10:00:16Z"}
                ]);
                next.transcript.snapshot(ConversationSnapshot {
                    seq: items.len() as u64,
                    first_seq: 1,
                    items,
                });
                this.update_view(Arc::new(next), window, cx)
            })
        });
        visual.run_until_parked();
    };
    render(
        vec![
            ask("Audit everything", "10"),
            say("Spawning two agents.", "12"),
        ],
        "running",
        &mut visual,
    );
    // Both started after row 1 and before anything later: one card there.
    workspace.read_with(&visual, |this, _| {
        assert_eq!(
            this.child_ui.overview.keys().copied().collect::<Vec<_>>(),
            [1]
        );
        assert_eq!(this.child_ui.overview[&1].len(), 2);
    });
    let running = visual
        .debug_bounds("subagent-overview")
        .unwrap()
        .size
        .height;
    // New messages land below the card instead of under it.
    render(
        vec![
            ask("Audit everything", "10"),
            say("Spawning two agents.", "12"),
            say("Still waiting on both.", "40"),
        ],
        "running",
        &mut visual,
    );
    workspace.read_with(&visual, |this, _| {
        assert_eq!(
            this.child_ui.overview.keys().copied().collect::<Vec<_>>(),
            [1]
        );
    });
    // All finished: a collapsed summary.
    render(
        vec![
            ask("Audit everything", "10"),
            say("Spawning two agents.", "12"),
            say("Still waiting on both.", "40"),
        ],
        "completed",
        &mut visual,
    );
    let finished = visual
        .debug_bounds("subagent-overview")
        .unwrap()
        .size
        .height;
    assert!(finished < running, "the finished overview collapses");
    // It can be reopened.
    let toggle = visual.debug_bounds("subagent-overview-toggle").unwrap();
    visual.simulate_click(toggle.center(), gpui::Modifiers::default());
    visual.run_until_parked();
    assert!(
        visual
            .debug_bounds("subagent-overview")
            .unwrap()
            .size
            .height
            > finished
    );
}

/// `state(selected)` with parent `a` in `parent_state` and one native
/// child `t1` in `child_status`.
fn native_child_state(
    selected: &str,
    parent_state: &str,
    child_status: &str,
    completed_at: i64,
) -> View {
    let mut next = state(selected);
    let parent = &mut Arc::make_mut(&mut next.sessions)[0];
    parent.state = parent_state.into();
    parent.merge(&serde_json::json!({"subagents":[{
        "id":"t1","description":"Audit","status":child_status,
        "startedAt":1000,"completedAt":completed_at
    }]}));
    next
}

fn provider_rows(this: &Workspace, cx: &App) -> usize {
    this.sidebar_rows(cx)
        .iter()
        .filter(|row| matches!(row, sidebar::SidebarRow::Provider { .. }))
        .count()
}

// #29: a finished provider-native child stays under its parent through
// turn ends, parent/sibling focus changes, opening and leaving it, and
// replayed snapshots; nothing flashes in or out.
#[gpui::test]
fn finished_native_subagents_stay_under_their_parent_across_focus_and_turns(
    cx: &mut TestAppContext,
) {
    let (workspace, mut visual, _, _updates) = fixture(cx);
    let show = |view: View, visual: &mut gpui::VisualTestContext| {
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.update_view(Arc::new(view), window, cx);
                (this.sidebar_rows(cx).len(), provider_rows(this, cx))
            })
        })
    };
    let steps = [
        ("b", "responding", "running", 0, "running, parent mid-turn"),
        (
            "b",
            "responding",
            "complete",
            2000,
            "finished, parent mid-turn",
        ),
        (
            "b",
            "input",
            "complete",
            2000,
            "finished and the turn is over",
        ),
        ("a", "input", "complete", 2000, "parent focused"),
        ("b", "input", "complete", 2000, "sibling focused again"),
        ("b", "input", "complete", 2000, "the same snapshot replayed"),
        ("a", "responding", "complete", 2000, "parent's next turn"),
        ("a", "input", "complete", 2000, "parent idle again"),
    ];
    for (selected, parent, child, completed, step) in steps {
        assert_eq!(
            show(
                native_child_state(selected, parent, child, completed),
                &mut visual
            ),
            (3, 1),
            "{step}"
        );
    }
    // Open the child, then go back to the parent: it stays both times.
    let mut viewing = native_child_state("a", "input", "complete", 2000);
    viewing.child = Some(wks_native::controller::ChildTarget {
        parent: "a".into(),
        agent: "t1".into(),
    });
    assert_eq!(show(viewing, &mut visual), (3, 1));
    assert_eq!(
        show(
            native_child_state("a", "input", "complete", 2000),
            &mut visual
        ),
        (3, 1)
    );
    // Its row keeps the truthful finished status.
    visual.run_until_parked();
    assert!(visual.debug_bounds("sidebar-provider-1").is_some());
    assert!(visual.debug_bounds("sidebar-clear-provider-1").is_some());
}

#[gpui::test]
fn clearing_a_finished_native_child_is_stable_until_it_works_again(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    let path = std::env::temp_dir().join(format!("native-clear-{}.json", std::process::id()));
    let show = |view: View, visual: &mut gpui::VisualTestContext| {
        visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.update_view(Arc::new(view), window, cx);
                provider_rows(this, cx)
            })
        })
    };
    visual.update(|_, cx| workspace.update(cx, |this, _| this.settings_path = Some(path.clone())));
    assert_eq!(
        show(
            native_child_state("b", "input", "complete", 2000),
            &mut visual
        ),
        1
    );
    visual.run_until_parked();
    let _ = archive_effects(&mut commands);
    let clear = visual.debug_bounds("sidebar-clear-provider-1").unwrap();
    visual.simulate_click(clear.center(), gpui::Modifiers::default());
    visual.run_until_parked();
    workspace.read_with(&visual, |this, cx| {
        assert_eq!(provider_rows(this, cx), 0);
        assert_eq!(
            this.view.selected.as_deref(),
            Some("b"),
            "selection unchanged"
        );
        assert!(!this.archived("a"), "clearing is not archiving");
    });
    assert!(
        archive_effects(&mut commands).is_empty(),
        "no stop, close, archive, delete or selection is sent"
    );
    let saved = Settings::load(&path).unwrap();
    assert!(
        saved.cleared_children["test"]
            .contains_key(&wks_native::child_agents::clear_key_provider("a", "t1"))
    );
    assert!(saved.archived.is_empty());
    // Replays, focus changes and turn boundaries keep it cleared.
    for (selected, parent) in [
        ("b", "input"),
        ("a", "input"),
        ("a", "responding"),
        ("b", "input"),
    ] {
        assert_eq!(
            show(
                native_child_state(selected, parent, "complete", 2000),
                &mut visual
            ),
            0,
            "{selected} {parent}"
        );
    }
    // Opening it from the chat still shows it while it is open.
    let mut viewing = native_child_state("a", "input", "complete", 2000);
    viewing.child = Some(wks_native::controller::ChildTarget {
        parent: "a".into(),
        agent: "t1".into(),
    });
    assert_eq!(show(viewing, &mut visual), 1);
    assert_eq!(
        show(
            native_child_state("a", "input", "complete", 2000),
            &mut visual
        ),
        0
    );
    // Reused: it shows while it runs and stays after the new finish.
    assert_eq!(
        show(native_child_state("a", "input", "running", 0), &mut visual),
        1
    );
    workspace.read_with(&visual, |this, _| {
        assert!(
            this.settings.cleared_children.is_empty(),
            "the clear is lifted"
        );
    });
    assert_eq!(
        show(
            native_child_state("a", "input", "complete", 9000),
            &mut visual
        ),
        1
    );
    let _ = std::fs::remove_file(path);
}

#[gpui::test]
fn a_finished_native_child_with_a_later_finish_returns_after_a_restart(cx: &mut TestAppContext) {
    // Cleared on one run, then the child worked and finished again while
    // this client was away: newer evidence brings it back.
    let (workspace, mut visual, _, _updates) = fixture(cx);
    let rows = visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(
                Arc::new(native_child_state("b", "input", "complete", 2000)),
                window,
                cx,
            );
            let marks = this.clearable_children(&this.view.sessions[0].clone());
            assert_eq!(marks.len(), 1);
            this.clear_children(marks, cx);
            let cleared = provider_rows(this, cx);
            this.update_view(
                Arc::new(native_child_state("b", "input", "complete", 7000)),
                window,
                cx,
            );
            (cleared, provider_rows(this, cx))
        })
    });
    assert_eq!(rows, (0, 1));
}

#[gpui::test]
fn finished_workspacer_children_offer_clear_in_place_of_archive(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    let with_child = |child_state: &str, selected: &str| {
        let mut next = state(selected);
        Arc::make_mut(&mut next.sessions).push(Session {
            id: "c".into(),
            label: "Worker".into(),
            parent_session_id: "a".into(),
            state: child_state.into(),
            ..Default::default()
        });
        Arc::new(next)
    };
    let ids = |this: &Workspace, cx: &App| -> Vec<String> {
        this.visible_sessions(cx)
            .into_iter()
            .map(|ix| this.view.sessions[ix].id.clone())
            .collect()
    };
    // Debug bounds outlive their frame, so absence is checked on a
    // window's first frame. Rows: a, c (nested), b.
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(with_child("input", "b"), window, cx)
        })
    });
    visual.run_until_parked();
    assert!(visual.debug_bounds("sidebar-clear-1").is_some());
    assert!(visual.debug_bounds("sidebar-archive-1").is_none());
    // Main sessions keep their Archive button.
    assert!(visual.debug_bounds("sidebar-archive-0").is_some());
    assert!(visual.debug_bounds("sidebar-archive-2").is_some());
    assert!(visual.debug_bounds("sidebar-clear-finished-0").is_some());
    let _ = archive_effects(&mut commands);
    let clear = visual.debug_bounds("sidebar-clear-1").unwrap();
    visual.simulate_click(clear.center(), gpui::Modifiers::default());
    visual.run_until_parked();
    assert!(archive_effects(&mut commands).is_empty());
    workspace.read_with(&visual, |this, cx| {
        assert_eq!(ids(this, cx), ["a", "b"]);
        assert!(!this.archived("c"));
        assert!(this.clear_marked("c"), "History offers Show in sidebar");
    });
    // Replayed snapshot and focus changes keep it cleared; opening it
    // shows it while open.
    for (selected, expected) in [
        ("b", vec!["a", "b"]),
        ("a", vec!["a", "b"]),
        ("c", vec!["a", "c", "b"]),
        ("b", vec!["a", "b"]),
    ] {
        let shown = visual.update(|window, cx| {
            workspace.update(cx, |this, cx| {
                this.update_view(with_child("input", selected), window, cx);
                ids(this, cx)
            })
        });
        assert_eq!(shown, expected, "selected {selected}");
    }
    // A new message makes it work again: it returns, and stays after.
    let shown = visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(with_child("responding", "b"), window, cx);
            let working = ids(this, cx);
            this.update_view(with_child("input", "b"), window, cx);
            (working, ids(this, cx))
        })
    });
    assert_eq!(shown.0, ["a", "c", "b"]);
    assert_eq!(shown.1, ["a", "c", "b"]);
    // History's Show in sidebar undoes a clear.
    visual.update(|_, cx| {
        workspace.update(cx, |this, cx| {
            let marks = this.clearable_children(&this.view.sessions[0].clone());
            this.clear_children(marks, cx);
            assert_eq!(ids(this, cx), ["a", "b"]);
            this.unclear_session("c", cx);
            assert_eq!(ids(this, cx), ["a", "c", "b"]);
        })
    });
}

#[gpui::test]
fn provider_row_cells_stay_contained_across_rendered_focus_changes(cx: &mut TestAppContext) {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            set_zoom(1.);
        }
    }
    let _reset = Reset;
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    for scale in [1., 1.5] {
        set_zoom(scale);
        visual.simulate_resize(size(gpui::px(900.), gpui::px(700.)));
        for selected in ["a", "b", "a", "b"] {
            visual.update(|window, cx| workspace.update(cx, |this, cx| {
                this.apply_typography(cx);
                let mut next = native_child_state(selected, "input", "complete", 2000);
                Arc::make_mut(&mut next.sessions)[0].merge(&serde_json::json!({"subagents":[{
                    "id":"t1", "status":"complete", "description":"Long provider child title ".repeat(30), "model":"unknown-model-".repeat(30)
                }]}));
                this.update_view(Arc::new(next), window, cx);
            }));
            visual.run_until_parked();
            let row = visual.debug_bounds("sidebar-provider-1").unwrap();
            let model = visual.debug_bounds("sidebar-child-model-1").unwrap();
            let clear = visual.debug_bounds("sidebar-clear-provider-1").unwrap();
            assert!(row.size.width > gpui::px(100.));
            assert!(model.left() >= row.left() && model.right() <= row.right());
            assert!(model.top() >= row.top() && model.bottom() <= row.bottom());
            assert!(row.contains(&clear.center()));
        }
    }
}

#[gpui::test]
fn stale_clear_clicks_and_replayed_finishes_do_not_hide_new_work(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.update(|window, cx| workspace.update(cx, |this, cx| {
        this.settings_path = None;
        this.update_view(Arc::new(native_child_state("b", "input", "complete", 2000)), window, cx);
        let painted = this.clearable_children(&this.view.sessions[0]);
        this.update_view(Arc::new(native_child_state("b", "input", "waiting_approval", 0)), window, cx);
        this.clear_children(painted, cx);
        assert!(this.settings.cleared_children.is_empty(), "stale Clear cannot clear a child now awaiting approval");
        this.update_view(Arc::new(native_child_state("b", "input", "complete", 3000)), window, cx);
        let marks = this.clearable_children(&this.view.sessions[0]);
        this.clear_children(marks, cx);
        assert_eq!(provider_rows(this, cx), 0);
        // Backfilled calls and activity are not another provider run.
        let mut backfill = native_child_state("b", "input", "complete", 3000);
        Arc::make_mut(&mut backfill.sessions)[0].merge(&serde_json::json!({"subagents":[{"id":"t1","status":"complete","startedAt":1000,"completedAt":3000,"lastActivity":9000,"toolCalls":99}]}));
        this.update_view(Arc::new(backfill), window, cx);
        assert_eq!(provider_rows(this, cx), 0);
        // A newer finish is proof of work even if running was not observed.
        this.update_view(Arc::new(native_child_state("b", "input", "complete", 5000)), window, cx);
        assert_eq!(provider_rows(this, cx), 1);
        assert!(this.settings.cleared_children.is_empty());
        this.update_view(Arc::new(native_child_state("b", "input", "complete", 3000)), window, cx);
        assert_eq!(provider_rows(this, cx), 1, "old finish cannot revive a retired clear mark");
    }));
    assert!(archive_effects(&mut commands).is_empty());
}

#[gpui::test]
fn active_workspacer_children_keep_archive_and_offer_no_clear(cx: &mut TestAppContext) {
    let (workspace, mut visual, _, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            let mut next = state("b");
            Arc::make_mut(&mut next.sessions).push(Session {
                id: "c".into(),
                parent_session_id: "a".into(),
                state: "responding".into(),
                ..Default::default()
            });
            this.update_view(Arc::new(next), window, cx);
        })
    });
    visual.run_until_parked();
    assert!(visual.debug_bounds("sidebar-archive-1").is_some());
    assert!(visual.debug_bounds("sidebar-clear-1").is_none());
    assert!(visual.debug_bounds("sidebar-clear-finished-0").is_none());
}

#[gpui::test]
fn clear_finished_on_the_parent_leaves_running_children(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            let mut next = state("b");
            let sessions = Arc::make_mut(&mut next.sessions);
            sessions[0].merge(&serde_json::json!({"subagents":[
                {"id":"done","status":"complete","startedAt":1000,"completedAt":2000},
                {"id":"busy","status":"running","startedAt":1500},
                {"id":"broke","status":"failed","startedAt":1000,"completedAt":1200}
            ]}));
            sessions.push(Session {
                id: "w1".into(),
                parent_session_id: "a".into(),
                state: "input".into(),
                ..Default::default()
            });
            sessions.push(Session {
                id: "w2".into(),
                parent_session_id: "a".into(),
                state: "responding".into(),
                ..Default::default()
            });
            sessions.push(Session {
                id: "g".into(),
                parent_session_id: "w1".into(),
                state: "responding".into(),
                ..Default::default()
            });
            this.update_view(Arc::new(next), window, cx);
        })
    });
    visual.run_until_parked();
    let _ = archive_effects(&mut commands);
    let clear = visual.debug_bounds("sidebar-clear-finished-0").unwrap();
    visual.simulate_click(clear.center(), gpui::Modifiers::default());
    visual.run_until_parked();
    assert!(archive_effects(&mut commands).is_empty());
    workspace.read_with(&visual, |this, cx| {
        let rows: Vec<String> = this
            .sidebar_rows(cx)
            .iter()
            .map(|row| match row {
                sidebar::SidebarRow::Session { index, .. } => this.view.sessions[*index].id.clone(),
                sidebar::SidebarRow::Provider { child, .. } => format!("native:{}", child.id),
            })
            .collect();
        // w1 was cleared but its working child g keeps it as context.
        assert_eq!(rows, ["a", "native:busy", "w1", "g", "w2", "b"]);
        assert!(this.clearable_children(&this.view.sessions[0]).is_empty());
    });
}

#[gpui::test]
fn workspacer_spawn_receipt_opens_only_an_available_child(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    let mut view = state("a");
    view.transcript.snapshot(ConversationSnapshot {
        seq: 2,
        first_seq: 1,
        items: vec![
            Item {
                kind: "tool_use".into(),
                id: "spawn-child".into(),
                name: "mcp__workspacer__spawn_agent".into(),
                input: serde_json::json!({"message":"Review parsing"}),
                ..Default::default()
            },
            Item {
                kind: "tool_result".into(),
                tool_use_id: "spawn-child".into(),
                content: serde_json::json!({"sessionId":"b"}).to_string(),
                ..Default::default()
            },
        ],
    });
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(view), window, cx);
            this.composer
                .update(cx, |input, cx| input.set_value("Parent draft", window, cx));
        })
    });
    visual.run_until_parked();
    let open = visual.debug_bounds("open-spawned-session").unwrap();
    visual.simulate_click(open.center(), gpui::Modifiers::default());
    assert!(matches!(commands.try_recv().unwrap(), Command::Select(id) if id == "b"));
    assert!(commands.try_recv().is_err());
    workspace.read_with(&visual, |this, cx| {
        assert_eq!(this.composer.read(cx).value().as_ref(), "Parent draft")
    });
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            let mut view = (*this.view).clone();
            Arc::make_mut(&mut view.sessions).retain(|s| s.id != "b");
            this.update_view(Arc::new(view), window, cx);
        })
    });
    visual.run_until_parked();
    let open = visual.debug_bounds("open-spawned-session").unwrap();
    visual.simulate_click(open.center(), gpui::Modifiers::default());
    assert!(commands.try_recv().is_err());
}

#[gpui::test]
fn inline_children_update_and_open_parent_scoped_transcripts_without_losing_drafts(
    cx: &mut TestAppContext,
) {
    let (workspace, mut visual, mut commands, _updates) = fixture(cx);
    let mut view = state("a");
    Arc::make_mut(&mut view.sessions)[0].merge(&serde_json::json!({"provider":"codex","subagents":[
        {"id":"native-one","toolUseId":"dispatch","type":"Explore","description":"Inspect parsing","status":"running","model":"runtime-model","tokens":0,"costUSD":0,"toolCalls":2,"startedAt":1000,"lastToolName":"Read"},
        {"id":"native-two","toolUseId":"dispatch","type":"Test","description":"Check aliases","status":"complete","startedAt":1000,"completedAt":4000}
    ]}));
    view.transcript.snapshot(ConversationSnapshot {
        seq: 1,
        first_seq: 1,
        items: vec![Item {
            kind: "tool_use".into(),
            id: "dispatch".into(),
            name: "Agent".into(),
            input: serde_json::json!({"prompt":"Inspect parsing"}),
            ..Default::default()
        }],
    });
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(view), window, cx);
            this.composer
                .update(cx, |input, cx| input.set_value("Parent draft", window, cx));
        })
    });
    visual.run_until_parked();
    assert!(visual.debug_bounds("child-agent-native-one").is_some());
    assert!(visual.debug_bounds("child-agent-native-two").is_some());
    assert!(visual.debug_bounds("native-spawn-icon").is_some());
    assert!(visual.debug_bounds("provider-child-icon").is_some());
    workspace.read_with(&visual, |this, _| {
        let child = &this.child_ui.agents.by_tool["dispatch"][0];
        assert_eq!(child.model, "runtime-model");
        assert_eq!(child.telemetry.tokens, Some(0));
        assert_eq!(child.telemetry.cost_usd, Some(0.));
        assert_eq!(
            this.child_ui.agents.by_tool["dispatch"][1].duration_ms(9000),
            Some(3000)
        );
    });
    let bounds = visual.debug_bounds("child-agent-native-one").unwrap();
    visual.simulate_click(bounds.center(), gpui::Modifiers::default());
    assert!(
        matches!(commands.try_recv().unwrap(),Command::Request(wks_native::features::Request::SubagentHistory{session,agent}) if session=="a" && agent=="native-one")
    );
    assert!(commands.try_recv().is_err());
    visual.update(|window,cx|workspace.update(cx,|this,cx|{
        let mut view=(*this.view).clone();
        Arc::make_mut(&mut view.sessions)[0].merge(&serde_json::json!({"subagents":[
            {"id":"native-one","status":"complete","completedAt":5000,"tokens":2000},
            {"id":"native-two"}
        ]}));
        view.requests.insert("subagent-history",wks_native::features::RequestState{
            request:wks_native::features::Request::SubagentHistory{session:"a".into(),agent:"native-one".into()},number:1,loading:false,error:None,
            value:Arc::new(serde_json::json!({"rows":[{"key":1,"role":"Assistant","text":"The parser handles these aliases.\n```wks-result\n{\"ok\":true,\"caveats\":[]}\n```"}]}))
        });
        this.update_view(Arc::new(view),window,cx);
    }));
    visual.run_until_parked();
    assert!(visual.debug_bounds("child-transcript-panel").is_some());
    assert!(visual.debug_bounds("structured-result-card").is_some());
    workspace.read_with(&visual, |this, cx| {
        let child = &this.child_ui.agents.by_tool["dispatch"][0];
        assert!(!child.running());
        assert_eq!(child.telemetry.tokens, Some(2000));
        assert_eq!(child.duration_ms(9000), Some(4000));
        assert_eq!(this.composer.read(cx).value().as_ref(), "Parent draft");
    });
    let close = visual.debug_bounds("close-child-transcript").unwrap();
    visual.simulate_click(close.center(), gpui::Modifiers::default());
    visual.run_until_parked();
    workspace.read_with(&visual, |this, _| {
        assert_eq!(
            this.child_ui.closed,
            Some(1),
            "close={close:?}, header={:?}, dock={:?}",
            this.header_bounds,
            this.composer_dock_bounds
        )
    });
    // GPUI Frame::clear retains historical debug_bounds. A newly painted
    // closed marker proves the transition; absence of the old selector does not.
    assert!(visual.debug_bounds("closed-child-transcript").is_some());
    assert!(commands.try_recv().is_err());
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            let mut other = state("b");
            Arc::make_mut(&mut other.sessions)[1]
                .merge(&serde_json::json!({"subagents":[{"id":"native-one","status":"running"}]}));
            other.requests = this.view.requests.clone();
            this.update_view(Arc::new(other), window, cx);
        })
    });
    visual.run_until_parked();
    assert!(
        visual.debug_bounds("child-agent-native-one").is_some(),
        "unanchored children remain visible even before transcript arrives"
    );
    assert!(visual.debug_bounds("child-only-list").is_some());
    workspace.read_with(&visual, |this,_| {
        assert_eq!(this.view.selected.as_deref(),Some("b"));
        assert!(matches!(&this.view.requests["subagent-history"].request,wks_native::features::Request::SubagentHistory{session,..} if session!="b"));
        assert!(this.child_ui.agents.by_tool.is_empty());
    });
}

#[gpui::test]
fn worktree_workers_stay_nested_under_their_manager_when_its_project_is_open(
    cx: &mut TestAppContext,
) {
    let (workspace, mut visual, mut commands, _) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            let mut next = state("manager");
            next.sessions = Arc::new(fleet_sessions());
            this.update_view(Arc::new(next), window, cx);
            // No filter: every session, workers under their manager.
            assert_eq!(this.visible_sessions(cx), vec![3, 4, 0, 1, 2]);
            // The manager's project was the hiding case (sidebar said 1).
            this.project_filter = Some("/home/u/Work/worky/workspacer".into());
            assert_eq!(this.visible_sessions(cx), vec![4, 0, 1, 2]);
            cx.notify();
        })
    });
    visual.run_until_parked();
    let manager = visual.debug_bounds("sidebar-session-0").unwrap();
    for (ix, selector) in [
        (1, "sidebar-session-1"),
        (2, "sidebar-session-2"),
        (3, "sidebar-session-3"),
    ] {
        let worker = visual.debug_bounds(selector).unwrap();
        assert!(
            worker.left() > manager.left() + px(8.),
            "worker {ix} is not indented"
        );
        assert!(worker.top() >= manager.bottom());
    }
    assert!(
        visual.debug_bounds("sidebar-session-4").is_none(),
        "unrelated project leaked in"
    );
    // Selecting a worker keeps the manager's project filter: it belongs there.
    let worker = visual.debug_bounds("sidebar-session-1").unwrap();
    visual.simulate_click(worker.center(), gpui::Modifiers::default());
    visual.run_until_parked();
    assert!(matches!(commands.try_recv().unwrap(), Command::Select(id) if id == "worker-3"));
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            let mut next = state("worker-3");
            next.sessions = Arc::new(fleet_sessions());
            this.update_view(Arc::new(next), window, cx);
            this.show_screen(Screen::Conversation, window, cx);
            assert_eq!(
                this.project_filter.as_deref(),
                Some("/home/u/Work/worky/workspacer")
            );
            // Searching one worker shows it under its manager only.
            this.search
                .update(cx, |input, cx| input.set_value("controls", window, cx));
            assert_eq!(this.visible_sessions(cx), vec![4, 1]);
        })
    });
}
