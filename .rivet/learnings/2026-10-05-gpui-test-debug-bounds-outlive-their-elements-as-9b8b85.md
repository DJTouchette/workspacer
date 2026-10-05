---
title: GPUI test debug_bounds outlive their elements; assert absence on state
date: 2026-10-05
confidence: high
suggested_doc: native-embedded-backend
related_paths:
  - apps/native/src/ui.rs
  - vendor/gpui/src/window.rs
promoted: false
---

# GPUI test debug_bounds outlive their elements; assert absence on state

## Observation
vendor/gpui Frame::clear() never clears debug_bounds, so VisualTestContext::debug_bounds(selector) keeps returning the last bounds after the element stops rendering (a closed terminal panel, a dismissed banner). Presence checks are reliable; absence checks are not. Also: under the private Xvfb rig a window placed partly off the 1700px screen silently clamps xdotool clicks (looked like a dead close button).

## Impact
False failures when asserting a panel/banner closed; false bug hunts in visual rigs.

## Recommendation
Assert absence through workspace/pane state (accessors under cfg(ui-tests)); in rigs, xdotool windowmove the window to 0,0 before clicking.
