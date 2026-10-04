---
title: GPUI UI tests - TextView reparse is real-time, next-frame callbacks never run, offscreen cells are unpainted
date: 2026-10-03
confidence: high
related_paths:
  - vendor/gpui-component/src/text/text_view.rs
  - vendor/gpui-component/src/text/node.rs
  - apps/native/src/ui.rs
promoted: false
---

# GPUI UI tests - TextView reparse is real-time, next-frame callbacks never run, offscreen cells are unpainted

## Observation
- `TextView` re-parses changed Markdown after a 200ms smol `Timer` debounce (`UpdateFuture`), which is real time. `advance_clock` does not fire it. A test that replaces a message's Markdown must sleep ~250ms, `window.refresh()` and `run_until_parked` (see `show_assistant_markdown` in ui.rs tests).
- The gpui 0.2.2 test platform's `on_request_frame` is a no-op, so `window.on_next_frame` and `request_animation_frame` callbacks never run in UI tests. A notify during draw only marks dirty views and does not schedule a redraw. To redraw after a layout-dependent decision, use `cx.defer(move |cx| cx.notify(view))` from prepaint. That works in tests (flush redraws dirty windows) and in production. `ProseTableFrame` uses it.
- Elements scrolled outside a scroll container's viewport are not painted, so `debug_bounds` returns None for them. Geometry tests of horizontally scrolled content must scroll across and check each cell where it is painted.

## Impact
Without these, UI tests silently assert against stale Markdown or the first-frame layout.
