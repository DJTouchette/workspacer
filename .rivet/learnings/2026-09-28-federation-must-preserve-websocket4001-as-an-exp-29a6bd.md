---
title: Federation must preserve WebSocket4001 as an explicit wake pause
date: 2026-09-28
suggested_doc: hub-federation
related_paths:
  - services/hub-rs/src/client.rs
  - services/hub-rs/src/federation.rs
promoted: false
---

# Federation must preserve WebSocket4001 as an explicit wake pause

## Observation
The Go bus server closes interactive clients, including outbound federation callers, with4001 before machine stop; Go busclient itself still reconnects every loss, while the TS browser pauses on4001. Rust Client now preserves typed RemoteClose for embedded sockets, remote sockets and prehello closes. Federation latches powerPaused, keeps unchanged peer reloads paused, and refuses forwarding until explicit authenticated-owner federation.resumePeer. No polling or backoff may dial a paused peer, and pending calls retain unknown-outcome errors rather than replay. Event fanout lag triggers same-socket reseeding so it cannot discard a queued stop close by forcing a new handshake.
