---
title: GPUI mouse-down focus leaves .focus() rings behind; native controls use focus_visible
date: 2026-10-06
confidence: high
related_paths:
  - apps/native/src/ui/chrome.rs
  - vendor/gpui/src/window.rs
  - vendor/gpui/src/elements/div.rs
promoted: false
---

# GPUI mouse-down focus leaves .focus() rings behind; native controls use focus_visible

## Observation
GPUI 0.2.2 focuses any focusable div on mouse-down (`paint_mouse_listeners`), so a
`.focus(|s| s.border_color(accent))` ring outlived every click: a segmented chip or
disclosure toggled off by mouse kept an accent border that read as still "on" until
focus moved. gpui-component `Switch` is not focusable and stops propagation, so
clicking one never cleared another control's ring. Vendored GPUI now has
`Window::last_input_was_keyboard()` and `.focus_visible(..)`; `chrome::interactive_control`
and tool-card headers use it. New native focus styles should use `focus_visible`, not
`focus`, except for text fields. UI tests can assert painted borders via the test-only
`Window::rendered_borders()`.

Repainting on an input-kind change must happen after the event is dispatched and
must only dirty the focused element's view: `dispatch_key_event` draws a dirty window
before dispatch, and both that extra frame and a full `refresh()` broke
`page_up_leaves_the_tail_and_repeated_pages_keep_moving` and
`reaching_the_tail_after_layout_resumes_follow_without_an_extra_wheel_event`.
`cx.notify(view)` is not a pure repaint either (it runs the entity's observers).
