---
title: Native self-measured layout loops forever on half-pixel snapping
date: 2026-10-05
confidence: high
suggested_doc: pane-system
related_paths:
  - apps/native/src/ui/island.rs
  - apps/native/src/ui/chrome.rs
promoted: false
---

# Native self-measured layout loops forever on half-pixel snapping

## Observation
The title island measures its actions' width with a canvas inside a box sized from the previous measurement. In the short_physical_windows UI test (fractional scale) the measurement alternated 277.5px <-> 278px forever; each cx.notify() re-laid out and re-snapped, so run_until_parked never parked and the test burned CPU indefinitely (not a slow test: a hang).

## Impact
Any GPUI measure-then-notify feedback (canvas prepaint -> defer -> compare -> notify) can livelock under fractional scale factors; the full UI suite then hangs on one test with no failure output.

## Recommendation
Compare measured sizes with hysteresis (accept growth, require >=1px to shrink), as island.rs settle_width does. Strip debug eprintln before committing.
