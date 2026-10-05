---
title: agents.childFullAccess: opt-in provider bypass for new children of local sessions
date: 2026-10-05
confidence: high
suggested_doc: agent-spawn
related_paths:
  - services/hub-rs/src/services/spawn_plan.rs
  - services/hub-rs/src/services/agent_spawn.rs
  - apps/desktop/src/main/services/fleetPermissions.ts
  - apps/native/src/ui/settings.rs
promoted: false
---

# agents.childFullAccess: opt-in provider bypass for new children of local sessions

## Observation
agents.fleetFullAccess only covers managers and fleet-lineage descendants (spawn_plan has_fleet_ancestor / desktop fleetSkipsPermissions). New shared key agents.childFullAccess (absent=off) makes NEW launches whose parentSessionId is a known local session (lifecycle record/owner snapshot/replacement, not hub-stamped) skip provider approvals; resumes, managers, unknown/foreign parents excluded. Rust spawn_plan::resolve now takes Lineage{fleet,local_child} (From<bool> keeps old callers); desktop claudeSpawn/managedSpawn OR childSkipsPermissions but hubCapabilities' paired branch does not. Remote dispatch wire stays skipPermissions:false. Native Settings→Agents reads/writes it with a deep-merged config.save + readback. Hub clippy -D warnings is NOT clean at base on clippy 1.94 (249 pre-existing errors).

## Impact
Provider approval policy is separate from Workspacer's approval gate and token scopes; both launchers must agree or a toggle silently does nothing on one host.
