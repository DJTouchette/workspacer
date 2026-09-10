---
title: Manager replacement delivery IDs cannot serve as logical request IDs
date: 2026-09-09
confidence: high
suggested_doc: fleet-manager
related_paths:
  - apps/desktop/src/main/services/claudemonSessionClient.ts
  - apps/desktop/src/main/services/managerReplacementState.ts
  - apps/desktop/src/main/services/managerReplacementService.ts
  - services/claudemon/src/daemon/api.rs
  - services/claudemon/src/session/store.rs
promoted: false
---

## Observation

At baseline 6d9a6b86, normal sends allocate an ephemeral frame ID only after
holdMessage has declined the send. Held messages allocate their own durable ID.
The explicit duplicate-risk replacement retry allocates a NEW delivery ID while
retaining the original text. Daemon message admission and managed provider input
channels carry text only; successful admission is not consumption evidence.

## Recommendation

Keep immutable logical request identity separate from delivery-attempt identity.
Do not use replacement delivery IDs, transcript UUIDs, optimistic FIFO state, or
text/timestamp matching as the sole authority. The unresolved source-of-work
contract and concrete alternatives are recorded in
docs/features/automatic-task-capture-contract.md. No capture implementation has
been enabled.
