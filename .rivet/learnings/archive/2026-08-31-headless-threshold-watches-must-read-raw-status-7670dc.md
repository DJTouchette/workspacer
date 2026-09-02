---
title: Headless threshold watches must read raw status_line before compat overlay
date: 2026-08-31
confidence: high
suggested_doc: fleet-manager
related_paths:
  - services/hub/cmd/brain/fleetview.go
  - services/hub/cmd/brain/agentops.go
  - apps/desktop/src/main/services/thresholdWatch.ts
promoted: true
promoted_to: fleet-manager
---

# Headless threshold watches must read raw status_line before compat overlay

## Observation
The Go brain merges high-frequency daemon status-line updates only into raw snake_case status_line; its camelCase compatibility statusLine can remain stale until a full snapshot. fleetSession intentionally prefers raw for total tokens and cost. The desktop reads its live store statusLine directly.

## Impact
A new context or quota predicate that only reads the camelCase headless projection will fire late or miss a threshold despite fresh telemetry.

## Recommendation
Extend fleetSession raw and compat fields together and use a single raw-first accessor for every status-line-derived health predicate; test a raw-only update case.

## Disposition
Promoted into `.rivet/context/modules/fleet-manager.md`. Re-verified against master `0bac5799`: `fleetSession.pick`, `contextHealth` and `outOfCredits` all read `status_line` before the camelCase overlay, for the reason the learning gives. Still live guidance.
