//! area-transcript: the v2 transcript (compact tool rows, collapsing fleet
//! wakes, hugging user bubbles, reading widths) keeps the shipped behaviour.
use super::*;
use wks_native::appearance::Design;
use wks_native::navigation::ReadingWidth;
use wks_native::tokens::Tokens;

fn v2(workspace: &Entity<Workspace>, visual: &mut VisualTestContext, view: View) {
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.force_design(Some(Design::V2));
            this.update_view(Arc::new(view), window, cx);
        })
    });
    visual.run_until_parked();
}

fn reads(n: usize) -> View {
    let mut view = state("a");
    view.transcript.snapshot(ConversationSnapshot {
        seq: n as u64,
        first_seq: 1,
        items: (0..n)
            .map(|i| Item {
                kind: "tool_use".into(),
                id: format!("call-{i}"),
                name: "Read".into(),
                input: serde_json::json!({"file_path":format!("src/file-{i}.rs")}),
                ..Default::default()
            })
            .collect(),
    });
    view
}

#[gpui::test]
fn v2_tool_calls_are_compact_rows_that_open_in_place(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _) = fixture(cx);
    // Running calls: the block in progress stays open.
    v2(&workspace, &mut visual, reads(3));
    assert!(visual.debug_bounds("tool-activity-group").is_some());
    let row = visual.debug_bounds("tool-toggle-0").unwrap();
    // CLI-dense: one small-control-height line per call.
    assert_eq!(f32::from(row.size.height), Tokens::V2.transcript().tool_row);
    click(&mut visual, "tool-toggle-0");
    workspace.read_with(&visual, |this, _| {
        assert_eq!(this.tool_expansion.get("call:call-0"), Some(&true));
    });
    assert!(visual.debug_bounds("tool-details").is_some());
    // The summary line folds the run again.
    let open = visual
        .debug_bounds("last-transcript-row")
        .unwrap()
        .size
        .height;
    click(&mut visual, "work-card-toggle");
    assert!(
        visual
            .debug_bounds("last-transcript-row")
            .unwrap()
            .size
            .height
            < open
    );
    assert!(commands.try_recv().is_err());
}

#[gpui::test]
fn v2_folds_older_steps_behind_a_count(cx: &mut TestAppContext) {
    let (workspace, mut visual, _, _) = fixture(cx);
    let visible = Tokens::V2.transcript().visible_steps;
    v2(&workspace, &mut visual, reads(visible + 3));
    assert!(visual.debug_bounds("work-earlier-steps").is_some());
    assert!(visual.debug_bounds("tool-toggle-0").is_none());
    click(&mut visual, "work-earlier-steps");
    assert!(visual.debug_bounds("tool-toggle-0").is_some());
}

const BLOCKED_WAKE: &str = "[supervisor] An agent is now blocked on a decision:\n- Native editor · terminal · Git (session:worker-1, approval)\nRun a /supervise pass now.";

fn wake_view(approval: bool) -> View {
    let mut view = state("manager");
    let mut sessions = fleet_sessions();
    if approval {
        sessions[2].approval = Some(serde_json::json!({"toolName":"Bash"}));
    }
    view.sessions = Arc::new(sessions);
    view.transcript.snapshot(ConversationSnapshot {
        seq: 1,
        first_seq: 1,
        items: vec![Item {
            kind: "user_message".into(),
            text: BLOCKED_WAKE.into(),
            timestamp: Some(chrono::Utc::now().to_rfc3339()),
            ..Default::default()
        }],
    });
    view
}

fn tap(visual: &mut VisualTestContext, selector: &'static str) {
    click(visual, selector);
    visual.update(|window, _| window.refresh());
    visual.run_until_parked();
}

// r8-conversation (R8.4): another session's wake is a header chip ("1
// update"), never a card or a line in the transcript; its popover row says
// what happened and opens the session, with no internal id.
#[gpui::test]
fn v2_pending_wake_is_a_header_chip_with_open(cx: &mut TestAppContext) {
    let (workspace, mut visual, mut commands, _) = fixture(cx);
    v2(&workspace, &mut visual, wake_view(true));
    let chip = visual.debug_bounds("updates-chip").unwrap();
    let header = visual.debug_bounds("chat-header").unwrap();
    assert!(chip.top() >= header.top() && chip.bottom() <= header.bottom());
    assert!(visual.debug_bounds("fleet-card-header").is_none());
    assert!(visual.debug_bounds("updates-popover").is_none());
    tap(&mut visual, "updates-chip");
    assert!(visual.debug_bounds("updates-popover").is_some());
    assert!(visual.debug_bounds("update-row-0").is_some());
    assert!(visual.debug_bounds("fleet-entry-id-0").is_none());
    click(&mut visual, "update-open-0");
    let effect = next_effect(&mut commands);
    assert!(matches!(&effect, Some(Command::Select(id)) if id == "worker-1"));
}

// R6.6 / R8.4: other sessions' wakes leave the transcript for the header's
// chip; its popover has one row per event, failures first.
#[gpui::test]
fn v2_wakes_leave_the_transcript_for_one_chip(cx: &mut TestAppContext) {
    let (workspace, mut visual, _, _) = fixture(cx);
    let mut view = wake_view(true);
    let wake = |text: &str| Item {
        kind: "user_message".into(),
        text: text.into(),
        timestamp: Some(chrono::Utc::now().to_rfc3339()),
        ..Default::default()
    };
    view.transcript.snapshot(ConversationSnapshot {
        seq: 4,
        first_seq: 1,
        items: vec![
            Item {
                kind: "assistant_text".into(),
                text: "Working on it.".into(),
                ..Default::default()
            },
            wake(BLOCKED_WAKE),
            wake("[fleet] Worker FAILED — did not complete:\n- Queue ordering (session:worker-3, cwd /x) — FAILED: Credit balance is too low\nReview each."),
            wake("[fleet] Worker finished:\n- Native controls (session:worker-2, cwd /x) — last reply: Done\nReview each."),
        ],
    });
    v2(&workspace, &mut visual, view);
    assert!(visual.debug_bounds("wake-strip").is_none());
    assert!(visual.debug_bounds("fleet-card").is_none());
    assert!(visual.debug_bounds("updates-chip").is_some());
    workspace.read_with(&visual, |this, _| {
        assert_eq!(this.strip_wakes().len(), 3);
    });
    tap(&mut visual, "updates-chip");
    let rows: Vec<_> = (0..3)
        .map(|n| {
            let id: &'static str = ["update-row-0", "update-row-1", "update-row-2"][n];
            visual.debug_bounds(id).unwrap()
        })
        .collect();
    assert!(rows[0].top() < rows[1].top() && rows[1].top() < rows[2].top());
    // The chip does not move the reading position (it is not a row
    // opening in place).
    workspace.read_with(&visual, |this, _| assert!(this.follow));
}

#[gpui::test]
fn v2_resolved_wake_is_one_row_in_the_popover(cx: &mut TestAppContext) {
    let (workspace, mut visual, _, _) = fixture(cx);
    v2(&workspace, &mut visual, wake_view(false));
    assert!(visual.debug_bounds("fleet-card").is_none());
    tap(&mut visual, "updates-chip");
    let row = visual.debug_bounds("update-row-0").unwrap();
    assert!(
        f32::from(row.size.height) <= Tokens::V2.layout.row_height + 1.,
        "one line: {:?}",
        row.size
    );
    assert!(visual.debug_bounds("update-row-1").is_none());
}

/// `n` reads that have all finished.
fn reads_done(n: usize) -> View {
    let mut view = state("a");
    view.transcript.snapshot(ConversationSnapshot {
        seq: 2 * n as u64,
        first_seq: 1,
        items: (0..n)
            .flat_map(|i| {
                [
                    Item {
                        kind: "tool_use".into(),
                        id: format!("call-{i}"),
                        name: "Read".into(),
                        input: serde_json::json!({"file_path":format!("src/file-{i}.rs")}),
                        ..Default::default()
                    },
                    Item {
                        kind: "tool_result".into(),
                        tool_use_id: format!("call-{i}"),
                        content: "1 fn main() {}".into(),
                        ..Default::default()
                    },
                ]
            })
            .collect(),
    });
    view
}

// r3-transcript: finished work folds to one line ("Worked … · 3 tools ›").
#[gpui::test]
fn v2_finished_work_folds_to_one_line(cx: &mut TestAppContext) {
    let (workspace, mut visual, _, _) = fixture(cx);
    v2(&workspace, &mut visual, reads_done(3));
    assert!(visual.debug_bounds("work-card-toggle").is_some());
    assert!(visual.debug_bounds("tool-toggle-0").is_none());
    click(&mut visual, "work-card-toggle");
    assert!(visual.debug_bounds("tool-toggle-0").is_some());
}

fn failed_command_view() -> View {
    let mut view = state("a");
    view.transcript.snapshot(ConversationSnapshot {
        seq: 3,
        first_seq: 1,
        items: vec![
            Item {
                kind: "tool_use".into(),
                id: "call-0".into(),
                name: "Bash".into(),
                input: serde_json::json!({"command":"cargo test"}),
                ..Default::default()
            },
            Item {
                kind: "tool_result".into(),
                tool_use_id: "call-0".into(),
                is_error: true,
                content: "Exit code 101\nerror: could not compile".into(),
                ..Default::default()
            },
            Item {
                kind: "tool_use".into(),
                id: "call-1".into(),
                name: "Read".into(),
                input: serde_json::json!({"file_path":"src/a.rs"}),
                ..Default::default()
            },
            Item {
                kind: "tool_result".into(),
                tool_use_id: "call-1".into(),
                content: "1 fn a() {}".into(),
                ..Default::default()
            },
        ],
    });
    view
}

#[gpui::test]
fn v2_short_output_and_failures_show_inline_and_ctrl_o_opens_everything(cx: &mut TestAppContext) {
    let (workspace, mut visual, _, _) = fixture(cx);
    v2(&workspace, &mut visual, failed_command_view());
    // A failed block stays open; its failure reads inline, in red, with
    // the exit code on the row, without opening the call.
    assert!(visual.debug_bounds("tool-toggle-0").is_some());
    assert!(visual.debug_bounds("event-fault").is_some());
    assert!(visual.debug_bounds("tool-details").is_none());
    let inline = workspace.read_with(&visual, |this, _| {
        format!("tool-inline-output-{}-call:call-0", this.chat_owner())
    });
    let inline: &'static str = Box::leak(inline.into_boxed_str());
    assert!(visual.debug_bounds(inline).is_some(), "{inline}");
    // Folded by hand, then Ctrl+O (the expand-all toggle) and back. The
    // list keeps a measured copy's selectors, so heights tell the state.
    let height = |visual: &mut VisualTestContext| {
        f32::from(
            visual
                .debug_bounds("last-transcript-row")
                .unwrap()
                .size
                .height,
        )
    };
    let open = height(&mut visual);
    click(&mut visual, "work-card-toggle");
    let folded = height(&mut visual);
    assert!(folded < open, "{folded} < {open}");
    let expand = |visual: &mut VisualTestContext| {
        visual.update(|_, cx| workspace.update(cx, |this, cx| this.toggle_expand_all(cx)));
        visual.run_until_parked();
    };
    expand(&mut visual);
    workspace.read_with(&visual, |this, _| assert!(this.chat.expand_all));
    assert_eq!(height(&mut visual), open);
    expand(&mut visual);
    workspace.read_with(&visual, |this, _| assert!(!this.chat.expand_all));
    assert_eq!(height(&mut visual), folded);
}

fn user_view(text: &str) -> View {
    let mut view = state("a");
    view.transcript.snapshot(ConversationSnapshot {
        seq: 2,
        first_seq: 1,
        items: vec![
            Item {
                kind: "user_message".into(),
                text: text.into(),
                ..Default::default()
            },
            Item {
                kind: "assistant_text".into(),
                text: "Done.".into(),
                ..Default::default()
            },
        ],
    });
    view
}

#[gpui::test]
fn v2_user_bubbles_hug_their_text(cx: &mut TestAppContext) {
    let (workspace, mut visual, _, _) = fixture(cx);
    v2(&workspace, &mut visual, user_view("Short ask."));
    let bubble = visual.debug_bounds("user-bubble").unwrap();
    let column = visual.debug_bounds("chat-content-column").unwrap();
    assert!(
        bubble.size.width < column.size.width * 0.5,
        "a short message hugs its text: {:?} in {:?}",
        bubble.size,
        column.size
    );
    // Right-aligned to the column.
    assert!((bubble.right() - column.right()).abs() < px(2.));
    let long = "A much longer message that keeps going well past one line of text, so the bubble should reach its cap rather than the full width of the reading column. ".repeat(3);
    let (workspace, mut visual, _, _) = fixture(cx);
    v2(&workspace, &mut visual, user_view(&long));
    let bubble = visual.debug_bounds("user-bubble").unwrap();
    let column = visual.debug_bounds("chat-content-column").unwrap();
    assert!(
        bubble.size.width <= column.size.width * Tokens::V2.transcript().user_max + px(1.),
        "capped: {:?} in {:?}",
        bubble.size,
        column.size
    );
}

#[gpui::test]
fn v2_reading_width_widens_the_column(cx: &mut TestAppContext) {
    let width_for = |cx: &mut TestAppContext, window: f32, reading: ReadingWidth| {
        let (workspace, mut visual, _, _) = fixture(cx);
        visual.simulate_resize(size(px(window), px(900.)));
        visual
            .update(|_, cx| workspace.update(cx, |this, _| this.settings.reading_width = reading));
        v2(&workspace, &mut visual, user_view("Hello"));
        f32::from(
            visual
                .debug_bounds("chat-content-column")
                .unwrap()
                .size
                .width,
        )
    };
    let tt = Tokens::V2.transcript();
    // r2-transcript: Comfortable is fluid. It grows with the window, past
    // the old fixed 760 measure, and stops at the ~1100 cap.
    let roomy = width_for(cx, 2400., ReadingWidth::Comfortable);
    assert!(
        roomy <= tt.reading_max + 1.,
        "{roomy} capped at {}",
        tt.reading_max
    );
    assert!(
        roomy >= tt.reading_max - 1.,
        "{roomy} reaches the cap in a wide window"
    );
    // r3: one column capped at 800, and no rail beside it, so a narrow
    // window shows the column below the cap.
    let snug = width_for(cx, 1140., ReadingWidth::Comfortable);
    assert!(
        snug < roomy,
        "{snug} < {roomy}: the column grows with the window"
    );
    assert!(
        snug >= tt.reading_min - 1.,
        "{snug} never under the {} minimum",
        tt.reading_min
    );
    assert!(
        snug > Tokens::V2.layout.chat_width,
        "{snug} wider than the old fixed measure"
    );
    // Wide and Full raise the cap further.
    let wide = width_for(cx, 2400., ReadingWidth::Wide);
    let full = width_for(cx, 2400., ReadingWidth::Full);
    assert!(roomy < wide, "{roomy} < {wide}");
    assert!(
        wide <= tt.reading_wide + 1.,
        "{wide} capped at {}",
        tt.reading_wide
    );
    assert!(wide < full, "{wide} < {full}");
}

#[test]
fn reading_width_reads_and_writes_as_a_lowercase_word() {
    let settings: wks_native::navigation::Settings =
        serde_json::from_str(r#"{"reading_width":"wide"}"#).unwrap();
    assert_eq!(settings.reading_width, ReadingWidth::Wide);
    assert_eq!(
        wks_native::navigation::Settings::default().reading_width,
        ReadingWidth::Comfortable
    );
    let json = serde_json::to_value(&settings).unwrap();
    assert_eq!(json["reading_width"], "wide");
}
