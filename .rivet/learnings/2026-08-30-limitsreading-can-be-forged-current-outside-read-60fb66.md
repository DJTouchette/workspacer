---
title: limits.Reading can be forged current outside ReadWindow
date: 2026-08-30
confidence: high
suggested_doc: usage-accounting
related_paths:
  - services/hub/internal/limits/window.go
  - services/hub/internal/limits/bucket.go
promoted: false
---

# limits.Reading can be forged current outside ReadWindow

## Observation
services/hub/internal/limits.Reading exports State while keeping resetsAt/usedPercent/windowMinutes private. ReadWindow preserves the intended invariant, but a caller or JSON round-trip can produce Reading{State: WindowCurrent} with zero private fields; ResetsAt and TimeToReset then return ok=true with a zero or negative duration. Current indexed production callers do not mutate or serialize Reading, so this is an exported-API invariant hazard rather than a live call-site bug.

## Impact
Future routing code could accidentally treat a forged or round-tripped Reading as current and reintroduce the non-positive time-to-reset path the guard is meant to make impossible.

## Recommendation
Keep the verdict unforgeable by making the state private behind a State() accessor, or make ResetsAt/TimeToReset also require a populated private reset flag/time and positive duration.
