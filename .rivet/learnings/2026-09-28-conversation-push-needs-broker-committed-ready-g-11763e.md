---
title: Conversation push needs broker-committed ready generations and overflow repair
date: 2026-09-28
confidence: high
suggested_doc: native-embedded-backend
related_paths:
  - services/hub-rs/src/services/live_streams.rs
  - services/hub-rs/tests/live_streams.rs
  - services/hub-rs/src/runtime.rs
promoted: false
---

# Conversation push needs broker-committed ready generations and overflow repair

## Observation
Go brain forwards raw ConversationDelta only while exact conversation topics are demanded, and statusline updates independently refresh admitted rows. Rust now uses the engine-owned typed stores, a broker-final demand-generation check, and a ready barrier committed before deltas. Broadcast lag reopens the receiver and emits ready rather than fabricating retained items. Slow broker consumers must discard all older queued conversation fragments before a new ready marker; applying that drain to PTY-only traffic broke its existing byte-then-desync contract, so that path remains separate.

## Recommendation
Keep real embedded transcript/status tests beside fake-source generation and lag tests. Unknown/dismissed/hidden rows must not publish; Full relay uses upstream layout. Test mixed event queues so unrelated frames survive and no old conversation delta follows ready. Treat witness unmapped new Rust sources as unproven until explicit suites run.
