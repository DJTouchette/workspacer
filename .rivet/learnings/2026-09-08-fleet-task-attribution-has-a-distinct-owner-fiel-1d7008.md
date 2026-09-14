---
title: Fleet task attribution has a distinct owner field from worker parentage
date: 2026-09-08
confidence: high
suggested_doc: fleet-manager
related_paths:
  - apps/desktop/src/main/services/dispatchHistoryStore.ts
  - apps/desktop/src/main/services/fleetWorkflowRuntime.ts
  - apps/desktop/src/main/services/hubCapabilities.ts
  - services/hub/internal/bus/spawnkeys.go
promoted: false
---

# Fleet task attribution has a distinct owner field from worker parentage

## Observation
Fleet dispatch metadata carries taskId, dispatchId and dispatchOwnerSessionId in addition to parentSessionId. Existing worker adoption reparents only the latter, so it cannot be assumed to transfer task history or workflow continuation authority.

## Impact
A manager replacement can receive finish wakes yet fail task inspector attribution or next-workflow-step validation unless ownership is atomically transferred or defined to remain with the original manager.

## Recommendation
Expose a shared host transaction that validates and updates dispatchOwnerSessionId/task records with the same operation that reparents children, and test stale/duplicate attempts.
