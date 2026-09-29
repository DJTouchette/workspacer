---
title: Embedding hub requires separating process lifecycle and provider transport
date: 2026-09-28
confidence: high
suggested_doc: native-embedded-backend
related_paths:
  - apps/native/src/host/local.rs
  - services/hub/cmd/hub/main.go
promoted: false
---

# Embedding hub requires separating process lifecycle and provider transport

## Observation
Native local startup links claudemon as a Rust dependency but explicitly requires workspacer, hub, brain, and mcp executables and connects Backend over WebSocket before attaching EmbeddedClient. Hub main owns signal.NotifyContext, parentwatch, Windows job confinement, and log.Fatalf paths; these must be separated from a host-owned library lifecycle. Embedding only the hub would leave brain provider traffic and claudemon HTTP integration across processes.
