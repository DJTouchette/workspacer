---
title: GPUI 0.2 X11Window::window_handle panics; find X11 windows by _NET_WM_PID + _NET_WM_NAME
date: 2026-10-02
confidence: high
suggested_doc: filelink-openable-files
related_paths:
  - apps/native/src/ui/window_destroy.rs
  - apps/native/src/ui/file_viewer.rs
promoted: false
---

# GPUI 0.2 X11Window::window_handle panics; find X11 windows by _NET_WM_PID + _NET_WM_NAME

## Observation
gpui-0.2.2 platform/linux/x11/window.rs implements HasWindowHandle/HasDisplayHandle for X11Window as unimplemented!(), so Window::window_handle() panics on the real X11 backend (and also on the test platform). The test platform's compositor_name() is empty, X11's is "X11". GPUI also ignores DestroyNotify, so an externally destroyed window stays in cx.windows() and its handle updates keep succeeding.

## Impact
Any native code asking GPUI for a raw handle crashes on X11; fake-platform tests cannot reveal it. Popout liveness cannot be inferred from cx.windows().

## Recommendation
Gate on cx.compositor_name()=="X11"; identify our window from a separate x11rb connection by _NET_WM_PID + exact _NET_WM_NAME (set with checked requests during open_window), as apps/native/src/ui/window_destroy.rs does. Always verify window-lifecycle work on the real binary under Xvfb+Openbox.
