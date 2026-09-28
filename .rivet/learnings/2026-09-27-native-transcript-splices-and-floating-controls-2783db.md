---
title: Native transcript splices and floating controls must preserve scroll anchors
date: 2026-09-27
confidence: high
suggested_doc: chat-tool-rendering
related_paths:
  - apps/native/src/model.rs
  - apps/native/src/ui.rs
  - apps/native/src/ui/scroll.rs
  - apps/native/src/ui/navigation.rs
  - apps/native/src/ui/tools.rs
promoted: false
---

# Native transcript splices and floating controls must preserve scroll anchors

## Observation
GPUI ListState::splice resets the logical top to old_range.start with offset zero when replacing the visible item. Native previously replaced the entire suffix on every Arc change, so reseeding identical snapshots could jump a reader to the beginning. Native now reuses unchanged snapshot rows, maps the paused anchor by stable key/call ID/content before splicing, and restores its within-row offset. Bottom-follow's item_count sentinel must be normalized to the actual top before keyboard scrolling or expanding cards. The Jump to latest button is outside the measured composer dock; putting it inside increases List bottom padding as soon as scrolling pauses. Momentum-end events no longer resume following; Jump to latest explicitly resumes it. Tests cover reseeds with earlier edits plus appends during paused scrolling, zero-delta scroll-stop, constant dock height, repeated page-up, and return to latest.
