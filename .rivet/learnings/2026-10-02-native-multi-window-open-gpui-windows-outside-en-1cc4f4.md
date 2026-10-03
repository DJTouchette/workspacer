---
title: Native multi-window: open GPUI windows outside entity updates; per-window widgets
date: 2026-10-02
confidence: high
suggested_doc: native-embedded-backend
related_paths:
  - apps/native/src/ui/file_viewer.rs
  - apps/native/src/main.rs
promoted: false
---

# Native multi-window: open GPUI windows outside entity updates; per-window widgets

## Observation
cx.open_window draws its root immediately; a root view that reads the Workspace entity panics (already being updated) if the window is opened inside Workspace::update. Open it from cx.defer and report back via the main window handle. InputState/TextView state is window-keyed (TextView via window.use_keyed_state), so a popped-out viewer builds its own widgets from a snapshot of the request state rather than sharing entities. Pane listeners that call workspace methods which read/replace the pane must defer (window.defer) or they double-borrow the pane. main.rs quit-on-close now checks the main window id, since a popped-out window keeps cx.windows() non-empty. TestAppContext simulate_close only calls the should-close handler; a handler whose deferred work removes the window makes simulate_close's own re-registration unwrap fail, so OS-close docking must not remove the window itself.

## Recommendation
Follow file_viewer.rs open_popout/popped_out/dock_closing_popout for new native windows; keep window-bound widgets per window.
