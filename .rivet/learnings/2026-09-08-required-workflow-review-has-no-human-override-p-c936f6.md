---
title: Required workflow review has no human-override path
date: 2026-09-08
confidence: high
suggested_doc: fleet-manager
related_paths:
  - apps/desktop/src/main/services/fleetWorkflowRuntime.ts
  - apps/desktop/src/main/services/dispatchHistoryStore.ts
  - services/hub/cmd/mcp/fleet_workflows.go
promoted: false
---

# Required workflow review has no human-override path

## Observation
The existing decide_workflow_step facade explicitly refuses required-review steps, while task history already owns per-task workflow decisions. A UI waiver must be a distinct audited task-state transition—not a fake completed decision or a policy edit.

## Impact
Prevents a UI implementation from claiming a required review passed or silently mutating the workflow default.

## Recommendation
Design an explicit task-scoped human override record and preserve the pinned policy revision/history.
