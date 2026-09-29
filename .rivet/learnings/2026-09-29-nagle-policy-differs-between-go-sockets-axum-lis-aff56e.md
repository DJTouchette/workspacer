---
title: Nagle policy differs between Go sockets Axum listeners and benchmark defaults
date: 2026-09-29
suggested_doc: hub-bus-control-plane
related_paths:
  - services/hub-rs/src/server.rs
  - services/hub-rs/src/mcp.rs
  - services/claudemon/src/daemon/mod.rs
  - services/hub-rs/tests/bus_latency.rs
  - services/hub-rs/tests/tcp_options.rs
promoted: false
---

# Nagle policy differs between Go sockets Axum listeners and benchmark defaults

## Observation
The real optimized a7e11447 latency guard failed with hub p99 40037us versus echo143us and unchanged5ms budget. Source inspection found production Rust Client already disables Nagle, but benchmark connect_async defaults to disable_nagle=false. Axum0.7.9 Serve defaults tcp_nodelay=None on accepted sockets; Go net.newTCPConn explicitly setsNoDelay(true) for both dial and accept. Hub/MCP and claudemon API/hook are the4 production Axum listeners; readiness/wrapper helper occurrences are tests only. All4 listener builders and both benchmark endpoints now explicitly match Go TCP_NODELAY policy. A Linux socket-option fixture reads actual accepted descriptors with a false/true control; configuration proof remains distinct from optimized timing proof.

## Impact
A bare echo can hide Nagle delays because replies carry ACKs, while unidirectional event subscribers can expose roughly40ms delayed-ACK stalls. The historical fast run does not override the latest failing receipt.

## Recommendation
Keep the original5ms budget/sample workload, retain both pass and failure receipts, verify actual socket options and require a fresh optimized CI pass before declaring the regression resolved.
