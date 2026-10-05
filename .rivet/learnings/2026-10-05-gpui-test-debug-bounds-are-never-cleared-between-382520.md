---
title: GPUI test debug_bounds are never cleared between frames
date: 2026-10-05
confidence: high
related_paths:
  - apps/native/src/ui.rs
  - vendor/gpui/src/window.rs
promoted: false
---

# GPUI test debug_bounds are never cleared between frames

## Observation
vendor/gpui Frame::clear() clears element states, hitboxes, scene etc. but not debug_bounds, so VisualTestContext::debug_bounds(sel) keeps returning the last bounds of an element that has since stopped rendering (including elements drawn in the fixture's very first frame with a default View).

## Impact
Native UI tests that assert an element disappeared via debug_bounds(..).is_none() fail (or worse, pass vacuously in reverse) after the element was ever shown.

## Recommendation
Assert absence on the state that decides rendering (e.g. footer_connection_label()), or use a fresh fixture where the element never rendered; use debug_bounds only for presence/geometry.
