---
title: X11 watcher attachment must acknowledge identity before preview titles change
date: 2026-10-02
confidence: high
related_paths:
  - apps/native/src/ui/window_destroy.rs
  - apps/native/src/ui/file_viewer.rs
promoted: false
---

# X11 watcher attachment must acknowledge identity before preview titles change

## Observation
GPUI creates/maps the popout before native starts its independent X11 watcher. PreviewPane::load also changes _NET_WM_NAME as files arrive. Neither a missing PID/title match nor a closed watcher channel proves the logical GPUI window is alive: raw destruction retains its registered handle. Use a unique frozen discovery title, acknowledge checked StructureNotify attachment before releasing it, and dock on every discovery/worker failure.
