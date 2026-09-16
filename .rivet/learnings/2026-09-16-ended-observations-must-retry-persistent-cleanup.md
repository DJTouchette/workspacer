---
title: Ended observations must retry persistent cleanup independently of lifecycle edges
date: 2026-09-16
---

`sessionStore.onEnd` is edge-triggered by design: repeating a stopped snapshot
must not repeat manager wakes or other lifecycle effects. Persistent cleanup has
a different contract. If an authtoken save fails on that edge, the token remains
and a duplicate stopped snapshot (including an SSE reseed after reconnect) is
the next recovery opportunity.

Keep retryable end cleanup on a separate callback with per-session success
state. Reserve an in-flight attempt, clear the reservation on failure, remember
durable success, and reset it only when the same session id is observed live
again. This preserves exactly-once lifecycle effects while making token
revocation eventually successful and idempotent.
