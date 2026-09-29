---
title: Conversation migration replaces SSE reconnection with broker-fenced embedded demand
date: 2026-09-29
related_paths:
  - services/hub-rs/src/services/live_streams.rs
  - services/hub-rs/src/cli/serve.rs
promoted: false
---

# Conversation migration replaces SSE reconnection with broker-fenced embedded demand

## Observation
Full standalone Rust serve owns an embedded engine; plan_serve only allows --external-claudemon with --hub-only. services/live_streams.rs therefore replaces the old brain SSE conversation loop with a typed broadcast receiver owned by exact-topic demand. runtime::Core calls accept_delivery immediately before publication, so generation fencing applies at commit. New stale-ready and actual disconnect/resubscribe tests complement existing stale-delta/visibility/lag tests. The optional borrowed daemon event bridge is not a full conversation producer and must not be documented as one.
