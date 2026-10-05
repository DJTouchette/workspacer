---
title: GPUI debug_bounds never clear; flex() after hidden() re-displays
date: 2026-10-05
confidence: high
related_paths:
  - apps/native/src/ui.rs
  - vendor/gpui/src/window.rs
promoted: false
---

# GPUI debug_bounds never clear; flex() after hidden() re-displays

## Observation
VisualTestContext::debug_bounds reads rendered_frame.debug_bounds, which Frame::clear does not reset, so a debug_selector element that rendered once keeps returning its last bounds after it disappears; absence checks via debug_bounds(..).is_none() only hold if it never rendered, and geometry reads must be gated on test state. Separately, Styled::hidden() sets display None but a later .flex() sets display Flex again: the native compact composer hint line uses .when(compact, |d| d.hidden()).occlude()...flex(), so it was never hidden below 620px (adds 32px to the compact dock).

## Impact
Stale debug bounds give false positives in UI tests; style order silently cancels hidden().

## Recommendation
Gate debug_bounds reads on known state or measured app state; put .hidden() after display setters. The compact hint line is a pending product decision (it carries the Working/elapsed status), not changed in #26.
