---
title: GPUI native window needs explicit Wayland title and app ID
date: 2026-09-26
confidence: high
related_paths:
  - apps/native/src/main.rs
promoted: false
---

# GPUI native window needs explicit Wayland title and app ID

## Observation
On this Hyprland host, apps/native's GPUI 0.2.2 window rendered and connected normally but hyprctl reported an empty title and class despite WindowOptions.titlebar.title. Call window.set_window_title and window.set_app_id during window construction so OS window discovery/switching can identify it. The apparent missing window was a title-filter miss, not startup failure.
