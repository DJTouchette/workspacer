---
title: Manager full-access mechanically ORs skipPermissions from config, both spawn paths
date: 2026-08-22
confidence: high
suggested_doc: agent-spawn
related_paths:
  - apps/desktop/src/main/services/claudeSpawn.ts
  - apps/desktop/src/main/services/managedSpawn.ts
  - apps/desktop/src/main/services/fullAccessGrants.ts
promoted: false
---

# Manager full-access mechanically ORs skipPermissions from config, both spawn paths

## Observation
claudeSpawn.ts:153 and managedSpawn.ts:214 both compute `const skipPermissions = !!opts.skipPermissions || supervisorFullAccess;` where supervisorFullAccess reads config.supervisor.fullAccess live (fullAccessGrants.ts). This mechanically forces skipPermissions/bypassPermissions when the flag is on, not just when a caller passes it explicitly. For the Fleet Manager role specifically (not supervisor), the equivalent mechanism is different: agents.fleetFullAccess / per-project yolo is NOT applied to the manager's own spawn skipPermissions — it's carried as a grant on the manager's session facade TOKEN (mintSessionFacadeToken via managerFullAccessFromConfig()) and applied live by the MCP facade when the manager DISPATCHES a worker, re-read per request via reconcileSessionFacadeGrants. So supervisor full-access is spawn-time-mechanical; manager/fleet full-access is dispatch-time-mechanical via the token grant, not the spawn opts.

## Impact
Someone auditing "does fleetFullAccess mechanically add skipPermissions" by grepping claudeSpawn/managedSpawn alone will only find the supervisor path and could wrongly conclude the manager path is unwired.
