---
title: Pinned native windows must retain their target after fleet changes
date: 2026-09-27
confidence: high
related_paths:
  - apps/native/src/ui.rs
  - apps/native/src/live.rs
promoted: false
---

# Pinned native windows must retain their target after fleet changes

## Observation
A startup-only --session selection is unsafe for live UI automation: when its session disappears, the controller can choose another fleet row. The native window now retains the requested ID, refuses other selections, and disables sending while that target is unavailable. Live-harness success checks also require the returned spawn ID, and its fake starts on an unrelated session that sorts first.

## Impact
A live test must not send prompts to an existing work session after a target vanishes.

## Recommendation
Keep target-disappearance GUI and unrelated-default live-harness regressions.
