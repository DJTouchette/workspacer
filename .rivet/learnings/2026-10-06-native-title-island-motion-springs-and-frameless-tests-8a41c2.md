---
title: Native title island motion runs on closed-form springs; UI tests use reduced motion
date: 2026-10-06
confidence: high
suggested_doc: chat-tool-rendering
related_paths:
  - apps/native/src/ui/motion.rs
  - apps/native/src/ui/island.rs
  - apps/native/src/ui/chrome.rs
promoted: false
---

# Native title island motion runs on closed-form springs; UI tests use reduced motion

## Observation
The title capsule's hover reveal and its notice island move on ui/motion.rs Spring: an analytic damped spring sampled at Instant::now(), so a render needs no per-frame state, retargets keep momentum, and settled() snaps exactly to the target. A frame is only requested while IslandMotion::moving() or the reveal is unsettled, so idle CPU stays at zero. IslandMotion keeps leaving rows as non-interactive ghosts (words fade, then the room closes), sizes rows from measured natural heights, and snaps on the first render, on a chat switch (hash of selected + child) and with Settings.reduce_motion. Five traps came up. (1) The GPUI test platform draws no animation frames, and window.refresh() reuses the cached Workspace view, so springs stay mid-flight. The UI fixtures set reduce_motion = true (the instant path is the old behavior); motion tests notify the entity and use freeze_island/settle_island. (2) The tray width used to come from measuring the island, but the tray can hold the island open, so it could never shrink. The bar is now measured (island items_start) as base = bar padding-box width - this frame's action extra, and tray = base + extra in the same frame. (3) Rows wrapped at the in-between width rewrap and mis-measure (a 12px over-grow). While moving, they lay out at the grown width, clipped by the row. (4) New words in a slot hold the old height (pending) until measured, so there is no one-frame jump; a rewrap at rest snaps. (5) Absolutely positioned canvases measure the parent's padding box, not its border box (taffy subtracts the border), so the old outline measure was 2px short.

## Impact
Any native chrome that animates layout must avoid measure-feedback loops (see settle_width) and must not assume the test platform will advance it.

## Recommendation
Reuse motion::Spring and the reduce_motion setting for new chrome motion. Test the motion with real timestamps on the state machine plus a freeze/settle GPUI test, not with sleeps in the general suite.
