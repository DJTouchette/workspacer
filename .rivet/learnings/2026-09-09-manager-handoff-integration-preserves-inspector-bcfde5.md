---
title: Manager handoff integration preserves Inspector reservation and readiness isolation
date: 2026-09-09
confidence: high
suggested_doc: fleet-manager
related_paths:
  - apps/desktop/src/main/services/managerReplacement.integration.test.ts
  - apps/desktop/src/main/services/dispatchHistoryStore.ts
promoted: false
---

# Manager handoff integration preserves Inspector reservation and readiness isolation

## Observation
Automatic replacement waits for source dispatch reservations before checkpoint/spawn; manual reparent refuses synchronously under the task transaction before moving any parent. Inspector permits waiving a different future step during that reservation. Lifecycle observation can legitimately advance revisions and derived wall time while the replacement waits. Provider readiness runs a separate disposable CLI through providerReadinessRuntime, not a manager kickoff hook.

## Recommendation
Keep all-task adoption in the serialized transaction and verify immutable attempt identity, audit and links across live observation; use the byte-for-byte store refusal test for atomic nonmutation. Keep readiness independent of parked successor message delivery.
