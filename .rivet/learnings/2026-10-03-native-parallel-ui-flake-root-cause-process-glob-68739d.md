---
title: Native parallel UI flake root cause: process-global zoom and caption preview
date: 2026-10-03
confidence: high
related_paths:
  - apps/native/src/ui.rs
  - apps/native/src/ui/chrome.rs
promoted: false
---

# Native parallel UI flake root cause: process-global zoom and caption preview

## Observation
ZOOM_BITS (ui.rs zoom/px) and chrome::FORCE_CAPTION were process-wide statics. interface_size_zooms_layout_and_widgets_together and the caption tests set them, so every GPUI test running concurrently laid out at another zoom or with Windows caption chrome (extra drag strip, caption-inset pages, sheet backdrop starting below 32px): geometry moved between frames, simulated clicks missed, caption right edge measured 719.5px. Both are now thread_local in cfg(all(test, feature=ui-tests)) builds; each GPUI test app lives on one thread. Production keeps the atomic zoom.

## Impact
Parallel cargo test --features ui-tests --bin wks-native went from 4/4 failing runs to 4/4 passing (102 tests). Supersedes the 'root cause not found' note in 2026-10-02-native-ui-tests-flake-in-parallel.

## Recommendation
Any new test-only override that render code reads must be per thread, not a static atomic; check that a red parallel run is not a leaked global before calling it flaky.
