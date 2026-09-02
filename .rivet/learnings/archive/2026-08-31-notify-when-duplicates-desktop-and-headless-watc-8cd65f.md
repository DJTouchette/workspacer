---
title: notify_when duplicates desktop and headless watch evaluators
date: 2026-08-31
confidence: high
suggested_doc: fleet-manager
related_paths:
  - apps/desktop/src/main/services/thresholdWatch.ts
  - services/hub/cmd/brain/agentops.go
  - services/hub/cmd/mcp/main.go
promoted: true
promoted_to: fleet-manager
---

# notify_when duplicates desktop and headless watch evaluators

## Observation
notify_when has separate TypeScript and Go implementations. Both intentionally use a 15-second sweep, one-shot in-memory watches, 20-watch cap, host-rendered fleet threshold wakes, and must remain behaviorally aligned; adding a predicate requires schema, evaluator, tests, docs, and both wake composers.

## Impact
A one-sided extension works only for either desktop or headless Fleet Managers and silently drifts policy.

## Recommendation
Treat notify_when additions as a dual-provider contract and add parity cases over the same fixtures.

## Disposition
Promoted into `.rivet/context/modules/fleet-manager.md`. Re-verified: the 15s sweep, 20-watch cap and one-shot in-memory policy are still spelled identically in `thresholdWatch.ts` and `cmd/brain/agentops.go`. The `contextUsedPct` predicate added since is the worked example of the five-part dual-provider change the learning describes.
