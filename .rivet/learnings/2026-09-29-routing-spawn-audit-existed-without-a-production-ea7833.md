---
title: Routing spawn audit existed without a production caller
date: 2026-09-29
confidence: high
related_paths:
  - services/hub-rs/src/services/routing/audit.rs
  - services/hub-rs/src/runtime.rs
  - services/hub-rs/src/services/agent_spawn.rs
  - services/hub-rs/ROUTING_ADMISSION_AUDIT.md
promoted: false
---

# Routing spawn audit existed without a production caller

## Observation
Rust RoutingService::audit_spawn had only a unit-test caller. External control-plane and owned coordinator paths enforced routing but wrote no spawn-side decisionId receipt. Added operation-scoped SpawnAudit: one safe allow/clamp/refusal row, accumulating multiple effective checks within a resolution while resetting at the owned post-setup re-resolution. Scope/fingerprint comes from actual Caller, and only typed routing/model fields are projected. Full lib334 and agent_spawn15 tests passed. Qualified federation spawn still bypasses the origin routing guard (receiver policy is separate), so complete spawnfresh/spawnceiling/core rows remain pending.

## Recommendation
Keep routing-phase receipt distinct from provider acceptance, preserve exactly-one logging for both owned resolution passes, and audit source-side qualified dispatch before certifying the remaining Go guards.
