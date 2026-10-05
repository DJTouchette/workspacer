---
title: Native child fleet overlays must preserve parsed work counters too
date: 2026-10-05
confidence: high
suggested_doc: session-lifecycle
related_paths:
  - apps/native/src/controller.rs
  - apps/native/tests/protocol.rs
promoted: false
---

# Native child fleet overlays must preserve parsed work counters too

## Observation
A real-WebSocket regression delivered totalToolCalls9 with completed/new children while sessions.snapshots was pending, then returned the old count2. The handoff preserved subagents but rolled count back9-to2, undermining Clear work fingerprints. Copy the parsed Option<u64> to all three counter aliases in the private overlay, avoiding rich unbounded values and preventing stale aliases winning. The extended test also pins completed workflow status.
