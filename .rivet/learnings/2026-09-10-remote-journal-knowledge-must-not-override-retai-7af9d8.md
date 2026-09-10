---
title: Remote journal knowledge must not override retained outcome and delivery receipt
date: 2026-09-10
confidence: high
suggested_doc: hub-federation
related_paths:
  - apps/desktop/src/main/services/remoteDispatchRegistry.ts
  - apps/desktop/src/main/services/remoteDispatchRegistry.test.ts
  - apps/desktop/tests/integration/dispatchChain.integration.ts
promoted: false
---

# Remote journal knowledge must not override retained outcome and delivery receipt

## Observation
Replay unknown describes the remote journal, not the origin's retained outcome. With no local update or a nonterminal update the outcome is still unknown. With terminal lastUpdate and an open receipt, the outcome is retained locally while manager wake delivery remains unconfirmed, whether or not deliveringSeq is set. Acknowledged terminal records remain untouched. Project this precedence before note de-duplication and retain rollback on persistence failure.

## Recommendation
Keep the full note-precedence table and the actual paired reconnect fixture with a real retained terminal packet, failed manager send, failed note write, duplicate unknown and guarded known replay. Note changes must not mutate lastUpdate, deliveringSeq, ackedSeq, ownership or execution state.
