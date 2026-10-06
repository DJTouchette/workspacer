---
title: gpui-component TextView selection was a rectangle, not a range
date: 2026-10-06
confidence: high
suggested_doc: chat-tool-rendering
related_paths:
  - vendor/gpui-component/src/text/inline.rs
  - vendor/gpui-component/src/text/text_view.rs
  - apps/native/src/ui.rs
promoted: false
---

# gpui-component TextView selection was a rectangle, not a range

## Observation
Native chat text selection (gpui-component TextView, `.selectable(true)`) did not track caret offsets. Every `Inline` selected the characters whose top-left fell inside the rectangle spanned by the drag start and the pointer (`selection_bounds` and `point_in_text_selection`). The rectangle drops direction, so a backward drag up and to the right (or a forward drag down and to the left) highlighted, and copied, upper-line text left of the pointer. The visible symptom: going backwards highlights the line above. Exact horizontal drags within one line were correct, which is why it looked like an intermittent backward-only bug.

Separately, GPUI `TextLayout::position_for_index` gives a soft-wrap index upstream affinity: it is reported at the end of the line it ends. Hit-testing with it put the first character of every wrapped line on the line above, so dragging back to a wrapped line's start missed that character.

## Fix
`TextViewState::selection_points` and `selection_carets` / `visual_box` in `inline.rs` replace the rectangle with reading-order carets, and correct the wrap affinity. The regression test is the native `chat_selection_follows_the_drag_in_reading_order`, which checks the copied text through secondary-c.

## Recommendation
To run gpui-component's own unit tests (it is not a workspace member and has dev-deps), copy `vendor/gpui-component` into a scratch workspace whose `[patch.crates-io]` points gpui at `vendor/gpui`, copy `apps/native/Cargo.lock`, then run `cargo test -p gpui-component --lib`. The test platform's NoopTextSystem is monospace (0.6em per glyph), so UI tests can compute character positions from the font size.
