---
title: GPUI Windows drag regions must not contain interactive chrome
date: 2026-10-02
confidence: high
suggested_doc: native-embedded-backend
related_paths:
  - apps/native/src/ui/sidebar.rs
  - apps/native/src/ui/chrome.rs
promoted: false
---

# GPUI Windows drag regions must not contain interactive chrome

## Observation
GPUI 0.2.2 translates WindowControlArea::Drag to HTCAPTION during WM_NCHITTEST. The callback checks every matching control hitbox before normal GPUI click dispatch, so an ancestor Drag hitbox remains eligible behind occluding interactive descendants.

## Impact
A broad app-chrome drag region can steal caption-button and content interactions or make hit-testing ambiguous.

## Recommendation
Apply WindowControlArea::Drag only to disjoint, noninteractive chrome bounds; do not rely on start_window_move on Windows because the GPUI Windows platform has no override for it.
