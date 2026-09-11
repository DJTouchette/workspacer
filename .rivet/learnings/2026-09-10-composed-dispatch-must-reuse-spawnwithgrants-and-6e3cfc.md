---
title: Composed dispatch must reuse spawnWithGrants and bind one step revision
date: 2026-09-10
confidence: high
suggested_doc: fleet-manager
related_paths:
  - services/hub/cmd/mcp/main.go
  - apps/desktop/src/main/services/fleetWorkflowRuntime.ts
  - apps/desktop/src/main/services/hubCapabilities.ts
promoted: false
---

# Composed dispatch must reuse spawnWithGrants and bind one step revision

## Observation
spawnWithGrants is the shared facade grant/default/identity gate used by spawn_agent and respawn_with; desktop agents.spawn is additionally wrapped by managerDispatch(workflowSpawn(...)). A composed workflow tool must call that same facade helper and ordinary bus spawn route, not invoke desktop spawn directly. Binding an exact workflow step and task revision prevents a repeated composite call from drifting to a subsequent eligible step.
