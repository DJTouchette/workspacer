---
title: Fleet manager context tax includes duplicated request/workflow schemas and unconditional reads
date: 2026-09-10
confidence: high
suggested_doc: fleet-manager
related_paths:
  - services/hub/cmd/mcp/manager_requests.go
  - services/hub/cmd/mcp/fleet_workflows.go
  - apps/desktop/src/main/shared/managerDoctrine.ts
promoted: false
---

# Fleet manager context tax includes duplicated request/workflow schemas and unconditional reads

## Observation
manager_requests.go registers all four tools with the same managerRequestIn (including the large intents schema even for list/get); fleet_workflows.go likewise shares every edit field across 13 tools. managerDoctrine.ts instructs reading worker conversations after wakes that already include results, and startup reading every project brief. These add context and round trips independently of model choice.
