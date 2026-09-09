---
title: Workflow decision replies must reread committed task snapshots
date: 2026-09-09
confidence: high
suggested_doc: fleet-manager
related_paths:
  - apps/desktop/src/main/services/fleetWorkflowService.ts
  - apps/desktop/src/main/services/fleetWorkflowService.test.ts
promoted: false
---

# Workflow decision replies must reread committed task snapshots

## Observation
DispatchHistoryStore.task returns a detached projection outside transactions. fleetWorkflowService decide previously retained that snapshot across workflowDecision, returning stale step state, revision and next instructions despite the disk commit. startWorkflow already rereads after commit; editByHostUser also rereads; workflowSpawn returns its spawn result. The unchanged dispatch-chain assertion at line 558 reproduced planned instead of skipped.

## Recommendation
Reread through ownerTask after workflowDecision succeeds and build both task response and instructions from that committed snapshot. Keep the dispatch-chain assertion and service coverage for run=true, run=false and persistence failure.
