//! The agent's task list pinned under the title, its transcript markers,
//! and the daemon's notices.
use super::*;

fn plan_item(steps: serde_json::Value, at: &str) -> Item {
    Item {
        kind: "plan".into(),
        steps,
        timestamp: Some(at.into()),
        ..Default::default()
    }
}

fn with_items(items: Vec<Item>) -> View {
    let mut next = state("a");
    next.transcript.snapshot(ConversationSnapshot {
        seq: items.len() as u64,
        first_seq: 1,
        items,
    });
    next
}

fn show(workspace: &Entity<Workspace>, visual: &mut VisualTestContext, view: View) {
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| this.update_view(Arc::new(view), window, cx))
    });
    visual.run_until_parked();
}

/// The newest plan is pinned under the title with its progress, open while
/// work remains; folding it holds while steps tick off, and a finished
/// list starts folded. Each change in the transcript is a quiet marker.
#[gpui::test]
fn the_task_list_is_pinned_with_progress_and_folds(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    let steps = |second: &str, third: &str| {
        serde_json::json!([
            {"content":"Explore","status":"completed"},
            {"content":"Build","status":second,"activeForm":"Building"},
            {"content":"Ship","status":third}
        ])
    };
    let first = plan_item(steps("in_progress", "pending"), "2026-10-01T10:00:00Z");
    show(&workspace, &mut visual, with_items(vec![first.clone()]));
    let pinned = bounds_of(&mut visual, "pinned-plan");
    let title = bounds_of(&mut visual, "title-bar");
    assert!(pinned.top() >= title.bottom(), "pinned under the title");
    bounds_of(&mut visual, "pinned-plan-steps");
    bounds_of(&mut visual, "pinned-plan-progress");
    bounds_of(&mut visual, "plan-marker");
    // Debug bounds outlive the elements that drew them; ask the state.
    let open = |visual: &mut VisualTestContext| {
        workspace.read_with(visual, |this, _| {
            let steps = this.pinned_plan().expect("a pinned plan");
            this.plan_open("a", &steps, false)
        })
    };
    assert!(open(&mut visual), "open while work remains");
    click(&mut visual, "pinned-plan-toggle");
    assert!(!open(&mut visual), "folded");
    // A step ticks off: same list, so it stays as the reader left it.
    let ticked = plan_item(steps("completed", "in_progress"), "2026-10-01T10:05:00Z");
    show(
        &workspace,
        &mut visual,
        with_items(vec![first.clone(), ticked]),
    );
    assert!(!open(&mut visual));
    // A different, finished list starts folded.
    let done = plan_item(
        serde_json::json!([{"content":"Release","status":"completed"}]),
        "2026-10-01T11:00:00Z",
    );
    show(&workspace, &mut visual, with_items(vec![first, done]));
    assert!(!open(&mut visual));
    workspace.read_with(&visual, |this, _| {
        assert_eq!(this.pinned_plan().unwrap()[0].content, "Release")
    });
    // The row's own plan pins when the transcript window holds none.
    let mut next = with_items(vec![Item {
        kind: "assistant_text".into(),
        text: "Earlier work scrolled out".into(),
        ..Default::default()
    }]);
    Arc::make_mut(&mut next.sessions)[0].merge(&serde_json::json!({
        "plan": {"steps":[{"content":"From the row","status":"pending"}]}
    }));
    show(&workspace, &mut visual, next);
    workspace.read_with(&visual, |this, _| {
        assert_eq!(this.pinned_plan().unwrap()[0].content, "From the row")
    });
    // No plan, no card.
    show(&workspace, &mut visual, with_items(Vec::new()));
    workspace.read_with(&visual, |this, _| assert!(this.pinned_plan().is_none()));
}

/// A transcript marker opens to the list as it was at that point.
#[gpui::test]
fn plan_markers_open_to_the_list_they_recorded(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    show(
        &workspace,
        &mut visual,
        with_items(vec![plan_item(
            serde_json::json!([{"content":"Explore","status":"in_progress"}]),
            "2026-10-01T10:00:00Z",
        )]),
    );
    let key = workspace.read_with(&visual, |this, _| {
        format!("live:{}:0-plan", this.chat_owner())
    });
    click(&mut visual, "plan-marker-toggle");
    workspace.read_with(&visual, |this, _| {
        assert_eq!(this.chat.open.get(&key), Some(&true))
    });
}

/// A driver notice is a subdued row of its own, never merged into the
/// agent's text around it.
#[gpui::test]
fn driver_notices_are_their_own_subdued_rows(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    let items: Vec<Item> = serde_json::from_value(serde_json::json!([
        {"kind":"assistant_text","text":"Checking the server."},
        {"kind":"assistant_text","text":"An MCP server asked for input Workspacer cannot show.","notice":true},
        {"kind":"assistant_text","text":"Carrying on."}
    ]))
    .unwrap();
    let mut next = with_items(Vec::new());
    next.transcript = Transcript::default();
    next.transcript.snapshot_for_transport(
        ConversationSnapshot {
            seq: 3,
            first_seq: 1,
            items,
        },
        true,
    );
    let roles = next
        .transcript
        .rows
        .iter()
        .map(|r| r.role.clone())
        .collect::<Vec<_>>();
    assert_eq!(roles, ["Assistant", "Notice", "Assistant"]);
    show(&workspace, &mut visual, next);
    bounds_of(&mut visual, "notice-row");
}
