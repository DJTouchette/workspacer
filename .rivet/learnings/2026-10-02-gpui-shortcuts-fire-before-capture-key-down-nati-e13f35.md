---
title: GPUI shortcuts fire before capture_key_down; native modals need focus hold plus keystroke interceptor
date: 2026-10-02
confidence: high
suggested_doc: chat-tool-rendering
related_paths:
  - apps/native/src/ui/file_viewer.rs
  - apps/native/src/ui.rs
  - apps/native/src/ui/navigation.rs
promoted: false
---

# GPUI shortcuts fire before capture_key_down; native modals need focus hold plus keystroke interceptor

## Observation
gpui 0.2.2 Window::dispatch_key_event matches keybindings and dispatches actions BEFORE key_down capture listeners run (finish_dispatch_key_event), so an element capture_key_down cannot stop a Workspace shortcut such as Ctrl+Enter. The deferred file-viewer sheet is still a dispatch-tree child of the shell (Workspace context is on its path). Input-context bindings of a focused composer behind the sheet also fire before any key listener. Only App::intercept_keystrokes runs before binding matching; stopping propagation there also suppresses platform text input.

## Impact
A modal that only captures Escape lets every workspace binding (send, session switch, new session) act on covered UI.

## Recommendation
For native modals: hold focus inside the sheet (hold_viewer_focus after every update_view), switch the shell key_context away from Workspace and drop its on_action handlers while open, bind Tab in the modal context, and add a window-scoped intercept_keystrokes guard for focus pulled behind the sheet. See file_viewer_contains_keys_over_a_nonempty_draft.
