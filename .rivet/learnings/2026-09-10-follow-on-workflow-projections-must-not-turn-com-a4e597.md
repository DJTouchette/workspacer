---
title: Follow-on workflow projections must not turn committed mutations into failures
date: 2026-09-10
confidence: high
suggested_doc: fleet-manager
related_paths:
  - apps/desktop/src/main/services/fleetWorkflowService.ts
promoted: false
---

# Follow-on workflow projections must not turn committed mutations into failures

## Observation
resolve_manager_request and accept_task_outcome commit changes before optional next-step context is built. If deriving that context throws (e.g. historical/missing pinned template), propagating the error would falsely report the mutation failed and invite a repeat. New nextActions projections are bounded to four tasks and isolate per-task read/derivation failures; the committed acknowledgement remains intact.
