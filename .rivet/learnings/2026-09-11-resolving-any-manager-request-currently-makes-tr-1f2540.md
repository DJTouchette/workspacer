---
title: Resolving any manager request currently makes tracked tasks mandatory for later dispatch
date: 2026-09-11
confidence: high
suggested_doc: fleet-manager
related_paths:
  - apps/desktop/src/main/services/dispatchHistoryStore.ts
  - apps/desktop/src/main/services/fleetWorkflowRuntime.ts
  - services/hub/cmd/mcp/workflow_dispatch.go
promoted: false
---

# Resolving any manager request currently makes tracked tasks mandatory for later dispatch

## Observation
DispatchHistoryStore.validate rejects a manager-owned spawn without taskId once that manager has any resolved inbox request, even a status/none request. Separately workflowSpawn requires decisionId, and dispatch_workflow_step exposes provider routing but no explicit model. This combination blocks explicit one-off or cheaper-model requests. Opting out must suppress task admission itself while preserving the worker parent/wake relationship; merely omitting taskId is insufficient.
