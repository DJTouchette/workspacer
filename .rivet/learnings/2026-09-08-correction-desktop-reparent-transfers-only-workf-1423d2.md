---
title: Correction: desktop reparent transfers only workflow task attribution
date: 2026-09-08
confidence: high
suggested_doc: fleet-manager
related_paths:
  - apps/desktop/src/main/services/claudeSessionStore.ts
  - apps/desktop/src/main/services/dispatchHistoryStore.ts
  - services/hub/cmd/brain/agentops.go
promoted: false
---

# Correction: desktop reparent transfers only workflow task attribution

## Observation
The earlier finding that reparenting transfers no task attribution was incomplete. claudeSessionStore.reparentChildren calls dispatchHistoryStore.adoptWorkflowTasks(old,new), which changes ownerSessionId only for tasks with workflow; ordinary dispatch tasks retain the old owner. Headless brain reparent updates only parent metadata and has no equivalent dispatch-history transfer.

## Impact
A seamless replacement must specify whether ordinary task attribution follows the manager and make desktop/headless behavior match; existing workflow ownership transfer is not atomic with all other handoff artifacts.

## Recommendation
Treat the existing workflow-only transfer as a partial contract, extend/replace it under a host transaction if all retained task IDs must remain owned by the successor, and add parity tests.
