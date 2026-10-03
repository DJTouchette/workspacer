---
title: gpui-component InputState::set_cursor_position steals keyboard focus
date: 2026-10-02
confidence: high
suggested_doc: native-embedded-backend
related_paths:
  - apps/native/src/ui/file_viewer.rs
  - vendor/gpui-component/src/input/state.rs
promoted: false
---

# gpui-component InputState::set_cursor_position steals keyboard focus

## Observation
InputState::set_cursor_position (gpui-component 0.5.1, state.rs) ends with self.focus(window, cx). Building a read-only source editor and placing its cursor (including a deferred re-apply after first layout) therefore moves window focus into that editor even when it is not rendered or is beside an active draft. A docked native file viewer stole the composer's focus this way; modal-only viewers hid it because they focus the editor anyway.

## Impact
Any non-modal editor whose cursor is placed programmatically takes the keyboard from the user, and focus can land on an editor not in the rendered tree, so keys go nowhere.

## Recommendation
Use file_viewer::move_cursor (captures window.focused before and restores it after) for programmatic cursor moves; focus the editor explicitly only when it should take the keyboard.
