//! Markdown tables: wrapping, scrolling, and every column reachable.
use super::*;

#[gpui::test]
fn markdown_table_cells_wrap_inside_their_column(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    let long = "apps/native/src/ui/transcript.rs keeps growing with words that must wrap \
                    inside this column rather than run across the next one";
    let mut view = state("a");
    view.transcript.snapshot(ConversationSnapshot {
        seq: 1,
        first_seq: 1,
        items: vec![Item {
            kind: "assistant_text".into(),
            text: format!("| File | Notes | Status |\n|---|---|---|\n| {long} | short | ok |"),
            ..Default::default()
        }],
    });
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| this.update_view(Arc::new(view), window, cx))
    });
    visual.run_until_parked();
    let long_cell = visual.debug_bounds("prose-table-cell-1-0").unwrap();
    let next = visual.debug_bounds("prose-table-cell-1-1").unwrap();
    let header = visual.debug_bounds("prose-table-cell-0-0").unwrap();
    assert!(
        long_cell.right() <= next.left() + px(0.5),
        "{long_cell:?} vs {next:?}"
    );
    assert!(
        long_cell.size.height > header.size.height * 2.,
        "the long cell wraps onto several lines: {long_cell:?}"
    );
}

/// Show `markdown` as the selected session's one assistant message.
fn show_assistant_markdown(
    workspace: &Entity<Workspace>,
    visual: &mut VisualTestContext,
    markdown: &str,
) {
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            // Later calls replace the message: a newer snapshot of the
            // same one-item conversation.
            let mut view = if this.view.selected.is_some() {
                (*this.view).clone()
            } else {
                state("a")
            };
            let seq = view.transcript.seq.unwrap_or(0) + 1;
            view.transcript.snapshot(ConversationSnapshot {
                seq,
                first_seq: 1,
                items: vec![Item {
                    kind: "assistant_text".into(),
                    text: markdown.into(),
                    ..Default::default()
                }],
            });
            this.update_view(Arc::new(view), window, cx)
        })
    });
    visual.run_until_parked();
    // Text views re-parse changed Markdown after a 200ms real-time
    // debounce (a smol timer, not the test clock).
    for _ in 0..2 {
        std::thread::sleep(std::time::Duration::from_millis(250));
        visual.update(|window, _| window.refresh());
        visual.run_until_parked();
    }
}

/// A table's frame selector: tables are keyed by their first cell's
/// source offset (`text` must start that cell).
fn table_frame(markdown: &str, first_cell: &str) -> (String, String) {
    let key = markdown.find(first_cell).expect("first cell in the source");
    (
        format!("prose-table-frame-{key}"),
        format!("prose-table-content-{key}"),
    )
}

/// Every column of the (only) table in view can be brought fully inside
/// its frame: wide tables scroll sideways rather than clip. Scrolls
/// across in half-viewport steps and requires each header cell to be
/// seen whole inside the viewport; adjacent painted cells never overlap.
/// Returns whether the table had to scroll, leaving it back at the start.
fn assert_every_column_reachable(
    visual: &mut VisualTestContext,
    (frame, content): &(String, String),
    cols: usize,
    window_width: gpui::Pixels,
) -> bool {
    let tolerance = gpui::px(0.5);
    let view = bounds_of(visual, frame);
    assert!(
        view.right() <= window_width + tolerance,
        "the table's viewport {view:?} stays inside the {window_width:?} window"
    );
    let content_start = bounds_of(visual, content);
    assert!(
        (content_start.left() - view.left()).abs() <= gpui::px(1.5),
        "the table starts unscrolled: {content_start:?} vs {view:?}"
    );
    let scrolls = content_start.right() > view.right() + gpui::px(1.5);
    assert_eq!(
        visual.debug_bounds("prose-table-scrollbar").is_some(),
        scrolls,
        "a scrollbar shows exactly when columns lie beyond the viewport \
             ({content_start:?} in {view:?})"
    );
    let mut seen = vec![false; cols];
    let mut header_heights = vec![None; cols];
    for _ in 0..60 {
        let mut painted = Vec::new();
        for (col, seen) in seen.iter_mut().enumerate() {
            let key: &'static str = Box::leak(format!("prose-table-cell-0-{col}").into_boxed_str());
            if let Some(cell) = visual.debug_bounds(key) {
                *seen |= cell.left() >= view.left() - tolerance
                    && cell.right() <= view.right() + tolerance;
                painted.push((col, cell));
                header_heights[col] = Some(cell.size.height);
            }
        }
        for pair in painted.windows(2) {
            let ((a_col, a), (b_col, b)) = (pair[0], pair[1]);
            if b_col == a_col + 1 {
                assert!(a.right() <= b.left() + tolerance, "{a:?} overlaps {b:?}");
            }
        }
        if bounds_of(visual, content).right() <= view.right() + gpui::px(1.5) {
            break;
        }
        // Sideways (trackpad or Shift+wheel) scrolling, over the header.
        visual.simulate_event(gpui::ScrollWheelEvent {
            position: gpui::point(view.center().x, view.top() + gpui::px(12.)),
            delta: gpui::ScrollDelta::Pixels(gpui::point(-view.size.width / 2., gpui::px(0.))),
            ..Default::default()
        });
        visual.run_until_parked();
    }
    assert!(
        seen.iter().all(|s| *s),
        "every column is reachable whole inside {view:?}: {seen:?}"
    );
    // These tests' headers are short words: one line each, never broken.
    let heights: Vec<_> = header_heights.into_iter().flatten().collect();
    let low = heights.iter().copied().fold(heights[0], |a, b| a.min(b));
    assert!(
        heights.iter().all(|h| *h <= low + tolerance),
        "a header wrapped onto another line: {heights:?}"
    );
    let end = bounds_of(visual, content);
    assert!(
        (end.right() - view.right()).abs() <= gpui::px(1.5),
        "scrolling stops at the table's end: {end:?} vs {view:?}"
    );
    visual.simulate_event(gpui::ScrollWheelEvent {
        position: gpui::point(view.center().x, view.top() + gpui::px(12.)),
        delta: gpui::ScrollDelta::Pixels(gpui::point(gpui::px(100_000.), gpui::px(0.))),
        ..Default::default()
    });
    visual.run_until_parked();
    assert_eq!(bounds_of(visual, content).left(), content_start.left());
    scrolls
}

const FOUR_COLUMNS: &str = "| Component | Implementation | Validation | Observation |\n\
        |---|---|---|---|\n| renderer | incremental | regression | consistent |";

#[gpui::test]
fn narrow_chat_table_scrolls_to_every_column(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    visual.simulate_resize(size(px(720.), px(480.)));
    show_assistant_markdown(&workspace, &mut visual, FOUR_COLUMNS);
    // Geometry alone: sideways scrolling over the table brings its last
    // column whole into the chat column (it used to be clipped at
    // x≈666–778, past the 720px window, with no way to reach it).
    let chat = bounds_of(&mut visual, "chat-composer");
    let first = bounds_of(&mut visual, "prose-table-cell-0-0");
    visual.simulate_event(gpui::ScrollWheelEvent {
        position: first.center(),
        delta: gpui::ScrollDelta::Pixels(gpui::point(px(-5000.), px(0.))),
        ..Default::default()
    });
    visual.run_until_parked();
    let last = bounds_of(&mut visual, "prose-table-cell-0-3");
    assert!(
        last.left() >= chat.left() && last.right() <= chat.right().min(px(720.)),
        "the last column {last:?} is reachable inside the chat column {chat:?}"
    );
    visual.simulate_event(gpui::ScrollWheelEvent {
        position: first.center(),
        delta: gpui::ScrollDelta::Pixels(gpui::point(px(5000.), px(0.))),
        ..Default::default()
    });
    visual.run_until_parked();
    let table = table_frame(FOUR_COLUMNS, "Component");
    let (frame, content) = table.clone();
    let view = bounds_of(&mut visual, &frame);
    assert!(
        view.left() >= chat.left() - px(1.) && view.right() <= chat.right() + px(1.),
        "the table stays in the chat column: {view:?} vs {chat:?}"
    );
    assert!(
        assert_every_column_reachable(&mut visual, &table, 4, px(720.)),
        "four whole-word columns do not fit beside the sidebar at 720px"
    );
    // A plain vertical wheel over the table scrolls the chat, not the table.
    let start = bounds_of(&mut visual, &content).left();
    visual.simulate_event(gpui::ScrollWheelEvent {
        position: view.center(),
        delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-40.))),
        ..Default::default()
    });
    visual.run_until_parked();
    assert_eq!(bounds_of(&mut visual, &content).left(), start);
    // The keyboard: click (or Tab to) the table, then Left/Right.
    let view = bounds_of(&mut visual, &frame);
    visual.simulate_click(view.center(), gpui::Modifiers::default());
    visual.run_until_parked();
    visual.simulate_keystrokes("right");
    visual.run_until_parked();
    let moved = bounds_of(&mut visual, &content).left();
    assert!(
        moved < start,
        "Right scrolls the table: {moved:?} vs {start:?}"
    );
    for _ in 0..20 {
        visual.simulate_keystrokes("right");
    }
    visual.run_until_parked();
    let view = bounds_of(&mut visual, &frame);
    let last = bounds_of(&mut visual, "prose-table-cell-0-3");
    assert!(
        last.right() <= view.right() + px(0.5),
        "{last:?} vs {view:?}"
    );
    let content_bounds = bounds_of(&mut visual, &content);
    assert!(
        (content_bounds.right() - view.right()).abs() <= px(1.5),
        "Right stops at the end: {content_bounds:?} vs {view:?}"
    );
    for _ in 0..20 {
        visual.simulate_keystrokes("left");
    }
    visual.run_until_parked();
    assert_eq!(bounds_of(&mut visual, &content).left(), start);
    // Tab leaves the table and Shift+Tab comes back to it.
    visual.simulate_keystrokes("tab right");
    visual.run_until_parked();
    assert_eq!(
        bounds_of(&mut visual, &content).left(),
        start,
        "after Tab, Right belongs to another control"
    );
    visual.simulate_keystrokes("shift-tab right");
    visual.run_until_parked();
    assert!(
        bounds_of(&mut visual, &content).left() < start,
        "the table is a keyboard stop"
    );
    visual.simulate_keystrokes("left left left left left left");
    visual.run_until_parked();
    // The mouse: drag the scrollbar's thumb to the right.
    let bar = bounds_of(&mut visual, "prose-table-scrollbar");
    let grab = gpui::point(bar.left() + px(20.), bar.center().y);
    visual.simulate_mouse_down(grab, gpui::MouseButton::Left, gpui::Modifiers::default());
    visual.simulate_mouse_move(
        grab + gpui::point(px(300.), px(0.)),
        Some(gpui::MouseButton::Left),
        gpui::Modifiers::default(),
    );
    visual.simulate_mouse_up(
        grab + gpui::point(px(300.), px(0.)),
        gpui::MouseButton::Left,
        gpui::Modifiers::default(),
    );
    visual.run_until_parked();
    let view = bounds_of(&mut visual, &frame);
    let dragged = bounds_of(&mut visual, &content);
    assert!(
        (dragged.right() - view.right()).abs() <= px(1.5),
        "dragging the thumb to the end shows the last column: {dragged:?} vs {view:?}"
    );
}

#[gpui::test]
fn tables_that_fit_fill_the_width_without_scrolling(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    let md = "| Name | Status |\n|---|---|\n| alpha | ok |";
    show_assistant_markdown(&workspace, &mut visual, md);
    let table = table_frame(md, "Name");
    let frame = table.0.clone();
    assert!(!assert_every_column_reachable(
        &mut visual,
        &table,
        2,
        px(1000.)
    ));
    let view = bounds_of(&mut visual, &frame);
    let last = bounds_of(&mut visual, "prose-table-cell-0-1");
    assert!(
        (last.right() - view.right()).abs() <= px(1.5),
        "columns fill the table: {last:?} vs {view:?}"
    );
    // Not a scroll region, so not a keyboard stop either.
    visual.simulate_click(view.center(), gpui::Modifiers::default());
    visual.run_until_parked();
    visual.simulate_keystrokes("right");
    visual.run_until_parked();
    assert_eq!(bounds_of(&mut visual, &frame), view);
    // The four-column table fits the default window without scrolling.
    show_assistant_markdown(&workspace, &mut visual, FOUR_COLUMNS);
    let table = table_frame(FOUR_COLUMNS, "Component");
    assert!(!assert_every_column_reachable(
        &mut visual,
        &table,
        4,
        px(1000.)
    ));
}

#[gpui::test]
fn wide_tables_of_every_shape_keep_all_columns_reachable(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    visual.simulate_resize(size(px(720.), px(480.)));
    let url = "https://example.com/a/very/long/unbroken/path/to/a/resource?with=query";
    let token = "Supercalifragilisticexpialidociousness";
    for cols in [2usize, 3, 5, 6, 8] {
        let header = (0..cols)
            .map(|c| ["Status", "Owner", "Notes", "Severity", "Area"][c % 5].to_owned())
            .collect::<Vec<_>>();
        let aligns = (0..cols)
            .map(|c| [":---", ":---:", "---:"][c % 3])
            .collect::<Vec<_>>();
        let body = (0..cols)
            .map(|c| match c % 4 {
                0 => url.to_owned(),
                1 => "`render_prose_table` keeps `min_w`".to_owned(),
                2 => token.to_owned(),
                _ => "short words that wrap inside the column".to_owned(),
            })
            .collect::<Vec<_>>();
        let md = format!(
            "Before the table.\n\n| {} |\n|{}|\n| {} |\n| {} |\n\nAfter the table.",
            header.join(" | "),
            aligns.join("|"),
            body.join(" | "),
            body.iter().rev().cloned().collect::<Vec<_>>().join(" | "),
        );
        show_assistant_markdown(&workspace, &mut visual, &md);
        // Tall tables open scrolled to their end; bring the header in.
        visual.simulate_event(gpui::ScrollWheelEvent {
            position: gpui::point(px(500.), px(250.)),
            delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(2000.))),
            ..Default::default()
        });
        visual.run_until_parked();
        visual
            .executor()
            .advance_clock(std::time::Duration::from_millis(800));
        visual.run_until_parked();
        let table = table_frame(&md, header[0].as_str());
        let frame = table.0.clone();
        assert_every_column_reachable(&mut visual, &table, cols, px(720.));
        // Rows stack inside the frame, and the table sits in the message
        // flow: it neither overlaps nor collapses into its neighbours.
        let view = bounds_of(&mut visual, &frame);
        let header_row = bounds_of(&mut visual, "prose-table-cell-0-0");
        let last_row = bounds_of(&mut visual, "prose-table-cell-2-0");
        assert!(header_row.top() >= view.top() && last_row.bottom() <= view.bottom() + px(0.5));
        assert!(last_row.top() >= header_row.bottom() - px(0.5));
        for col in 0..cols {
            let a = bounds_of(&mut visual, &format!("prose-table-cell-1-{col}"));
            assert_eq!(
                a.top(),
                bounds_of(&mut visual, "prose-table-cell-1-0").top()
            );
            assert!(
                a.size.height > px(0.),
                "{cols} columns: cell {col} is empty"
            );
        }
    }
}

#[gpui::test]
fn inline_code_stays_whole_in_columns_at_their_minimum(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    visual.simulate_resize(size(px(720.), px(480.)));
    // Too wide to fit, so every column sits at its whole-word minimum;
    // code in centred and right-aligned columns must still not break
    // (its thin-space margins count toward the minimum).
    let md = "| Area | Owner | Component | Implementation |\n|---:|:---:|---|---|\n\
                  | `node.rs` | `render_prose_table` | renderer | incremental |";
    show_assistant_markdown(&workspace, &mut visual, md);
    let table = table_frame(md, "Area");
    assert!(assert_every_column_reachable(
        &mut visual,
        &table,
        4,
        px(720.)
    ));
    let plain = bounds_of(&mut visual, "prose-table-cell-0-0").size.height;
    for col in [0, 1] {
        let code = bounds_of(&mut visual, &format!("prose-table-cell-1-{col}"));
        assert!(
            code.size.height <= plain + px(0.5),
            "code in column {col} wrapped: {code:?} vs one line {plain:?}"
        );
    }
}

#[gpui::test]
fn two_wide_tables_scroll_independently(cx: &mut TestAppContext) {
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    visual.simulate_resize(size(px(720.), px(480.)));
    let second = FOUR_COLUMNS.replace("Component", "Subsystem");
    let md = format!("{FOUR_COLUMNS}\n\nBetween.\n\n{second}");
    show_assistant_markdown(&workspace, &mut visual, &md);
    let (frame_a, content_a) = table_frame(&md, "Component");
    let (frame_b, content_b) = table_frame(&md, "Subsystem");
    let (a, b) = (
        bounds_of(&mut visual, &frame_a),
        bounds_of(&mut visual, &frame_b),
    );
    assert!(a.bottom() <= b.top(), "{a:?} above {b:?}");
    visual.simulate_event(gpui::ScrollWheelEvent {
        position: b.center(),
        delta: gpui::ScrollDelta::Pixels(gpui::point(px(-5000.), px(0.))),
        ..Default::default()
    });
    visual.run_until_parked();
    assert!(bounds_of(&mut visual, &content_b).left() < b.left() - px(1.));
    assert_eq!(bounds_of(&mut visual, &content_a).left(), a.left() + px(1.));
}

#[gpui::test]
fn larger_interface_sizes_keep_table_columns_reachable(cx: &mut TestAppContext) {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            set_zoom(1.);
        }
    }
    let _reset = Reset;
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    show_assistant_markdown(&workspace, &mut visual, FOUR_COLUMNS);
    let table = table_frame(FOUR_COLUMNS, "Component");
    for _ in 0..6 {
        visual.simulate_keystrokes("ctrl-=");
    }
    visual.run_until_parked();
    workspace.read_with(&visual, |this, _| {
        assert!(
            this.settings.interface_scale >= 150,
            "{}",
            this.settings.interface_scale
        )
    });
    assert!(assert_every_column_reachable(
        &mut visual,
        &table,
        4,
        px(1000.)
    ));
}

#[gpui::test]
fn markdown_preview_tables_reach_every_column_docked_sheet_and_window(cx: &mut TestAppContext) {
    let doc = format!("# Plan\n\n{FOUR_COLUMNS}\n\nAfter.\n");
    let table = table_frame(&doc, "Component");
    let frame = table.0.clone();
    let (workspace, mut visual, _commands, _updates) = fixture(cx);
    visual.update(|window, cx| {
        workspace.update(cx, |this, cx| {
            this.update_view(Arc::new(state("a")), window, cx)
        })
    });
    // A narrow window: the viewer is a sheet over the chat.
    visual.simulate_resize(size(gpui::px(720.), gpui::px(480.)));
    preview_state(
        &workspace,
        &mut visual,
        "a",
        file_target("/repo/docs/plan.md"),
        1,
        false,
        None,
        serde_json::json!({"contents": doc, "size": doc.len()}),
    );
    settle(&mut visual);
    workspace.read_with(&visual, |this, cx| {
        assert_eq!(
            this.file_viewer().unwrap().read(cx).mode(),
            file_viewer::Mode::Preview
        )
    });
    assert_every_column_reachable(&mut visual, &table, 4, gpui::px(720.));
    // Docked beside the chat in a wide window.
    visual.simulate_resize(size(gpui::px(1400.), gpui::px(800.)));
    settle(&mut visual);
    let panel = bounds_of(&mut visual, "file-viewer-panel");
    let view = bounds_of(&mut visual, &frame);
    assert!(view.left() >= panel.left() && view.right() <= panel.right() + gpui::px(0.5));
    assert_every_column_reachable(&mut visual, &table, 4, gpui::px(1400.));
    // Popped out into a narrow window of its own.
    let popout = bounds_of(&mut visual, "file-viewer-popout");
    visual.simulate_click(popout.center(), gpui::Modifiers::default());
    visual.run_until_parked();
    let (handle, _pane) =
        workspace.read_with(&visual, |this, _| this.viewer_popout().expect("popped out"));
    let mut window = VisualTestContext::from_window(handle.into(), cx);
    window.simulate_resize(size(gpui::px(420.), gpui::px(480.)));
    window.run_until_parked();
    assert!(
        assert_every_column_reachable(&mut window, &table, 4, gpui::px(420.)),
        "a narrow separate window scrolls the table"
    );
}
