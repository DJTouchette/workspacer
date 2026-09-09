---
title: Manager replacement recovery crosses an unreceipted daemon message boundary
date: 2026-09-08
confidence: high
suggested_doc: fleet-manager
related_paths:
  - apps/desktop/src/main/services/claudeSessionStore.ts
  - apps/desktop/src/main/services/claudemonSessionClient.ts
  - services/claudemon/src/daemon/api.rs
  - services/claudemon/src/session/store.rs
promoted: false
---

# Manager replacement recovery crosses an unreceipted daemon message boundary

## Observation
Verified replacement durability gap: desktop claudeSessionStore.spawnMeta, parentSessionId and isWakeTarget are process memory. Claudemon SessionStore::hydrate rebuilds SQLite rows as Stopped and does not restore these desktop fields. POST /sessions/:id/message accepts only text; native_submit_message returns Sent after an in-memory managed input channel send or pending-message enqueue, and pending_messages is an Arc<DashMap>. A desktop journal cannot distinguish an accepted kickoff/wake whose HTTP acknowledgement was lost from one that never reached the daemon. Retrying can duplicate a manager turn; recording delivery first can lose it. This is separate from durable dispatch-history ownership.

## Impact
Automatic restart recovery cannot promise exactly-once successor activation or wake delivery using existing desktop-only primitives. A daemon restart also cannot restore a live wake target by relabeling a stopped SQLite row.

## Recommendation
Choose an operation-correlated daemon delivery receipt/admission contract with compatible-host negotiation, or explicitly support recovery-required ambiguity with preserved ownership and queued evidence and no automatic replay. Persist ownership overlays before transfer; do not infer liveness from journal lineage. Keep renderer reload recovery distinct from desktop-process and daemon restart recovery.
