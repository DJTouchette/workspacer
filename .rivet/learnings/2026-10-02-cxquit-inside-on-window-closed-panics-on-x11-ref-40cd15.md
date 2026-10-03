---
title: cx.quit() inside on_window_closed panics on X11 (RefCell already borrowed)
date: 2026-10-02
confidence: high
suggested_doc: hub-process-supervision
related_paths:
  - apps/native/src/main.rs
promoted: false
---

# cx.quit() inside on_window_closed panics on X11 (RefCell already borrowed)

## Observation
On GPUI 0.2 X11, WM_DELETE_WINDOW close runs X11WindowStatePtr::close inside X11Client::handle_event while the client RefCell is borrowed; an App::on_window_closed callback that calls cx.quit() re-borrows it via with_common and panics, so every main-window close exited 101. Branch commit 2b7d473d introduced it; reviewers saw 'app terminated' and counted it as a clean exit.

## Impact
Shutdown looked fine from outside (process gone) but was a panic; any quit/platform call from window-close callbacks has the same hazard.

## Recommendation
Defer platform calls out of close callbacks with cx.spawn(async move |cx| cx.update(|cx| cx.quit())). Check app logs for 'panicked' when verifying shutdown, not just process exit.
