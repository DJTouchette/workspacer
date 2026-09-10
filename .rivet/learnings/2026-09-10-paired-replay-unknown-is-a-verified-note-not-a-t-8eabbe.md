---
title: Paired replay unknown is a verified note, not a transport failure or completion
date: 2026-09-10
confidence: high
suggested_doc: hub-federation
related_paths:
  - apps/desktop/src/main/services/pairedDispatch.ts
  - apps/desktop/src/main/services/remoteDispatchRegistry.ts
  - apps/desktop/tests/integration/dispatchChain.integration.ts
promoted: false
---

# Paired replay unknown is a verified note, not a transport failure or completion

## Observation
The paired reconnect path discarded the agents.dispatchReplay RPC result even though the production brain returns state unknown when its journal is unavailable. Reusing RemoteDispatchRegistry.markLost preserves open ownership and completion semantics; the method must skip an identical note to avoid rewriting it on every reconnect. A verified note must be bound to the requested dispatch ID and still-current pairing, whereas rejected calls and disconnections must not manufacture that note.

## Impact
Without consuming the response, an unresolved dispatch remains silently open. Treating unknown as completion or retrying spawn would be unsafe.

## Recommendation
Keep the real paired reconnect regression for unknown, duplicate unknown, RPC failure, mismatched dispatch ID, known replay recovery and terminal-record preservation.
