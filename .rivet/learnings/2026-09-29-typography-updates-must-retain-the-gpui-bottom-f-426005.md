---
title: Typography updates must retain the GPUI bottom-follow sentinel
date: 2026-09-29
confidence: high
suggested_doc: theme-system
related_paths:
  - apps/native/src/ui/typography.rs
  - apps/native/src/ui/scroll.rs
  - apps/native/src/ui.rs
promoted: false
---

# Typography updates must retain the GPUI bottom-follow sentinel

## Observation
Workspace::scroll_anchor normalizes GPUI's item_count bottom-follow sentinel into a visible reading anchor. Calling it unconditionally when changing typography loses the list's tail position. Typography invalidation must use ListOffset(item_count, 0) while follow is enabled, and use the reading anchor only while paused. Real render passes may adjust intra-row offsets when header font metrics change.

## Recommendation
Preserve both modes when invalidating transcript geometry for font or size changes; test paused reading and following new activity.
