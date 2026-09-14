---
title: Fleet task loops require explicit correlation
date: 2026-09-06
confidence: high
suggested_doc: fleet-manager
related_paths:
  - apps/desktop/src/main/services/hubCapabilities.ts
  - apps/desktop/src/main/services/claudeSessionStore.ts
  - services/hub/cmd/mcp/respawn.go
promoted: false
---

# Fleet task loops require explicit correlation

## Observation
Fleet dispatches persist a parentSessionId for wake routing and routing decisionId for model-selection joins, but parent groups an entire manager and decisionId is intentionally not inherited by respawn_with. Neither can truthfully group scout/implement/review attempts into one durable task, especially after Electron restart.

## Impact
A whole-loop recent-agent view would fabricate membership if it groups rows by parent, label, time, transcript prose, or routing decision.

## Recommendation
Record a bounded host-owned taskId/dispatchId at the accepted manager-owned agents.spawn lifecycle boundary; only explicit task reuse, stage links, and respawn_with retry provenance may create a loop edge.
