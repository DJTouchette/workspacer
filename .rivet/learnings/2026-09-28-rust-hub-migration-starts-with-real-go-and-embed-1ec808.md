---
title: Rust hub migration starts with real Go and embedded transport fixtures
date: 2026-09-28
confidence: high
suggested_doc: native-embedded-backend
related_paths:
  - services/hub-rs
  - services/hub/cmd/brain/contracts_test.go
promoted: false
---

# Rust hub migration starts with real Go and embedded transport fixtures

## Observation
services/hub-rs now has an owned runtime and in-memory client plus an optional loopback WebSocket adapter. Shared bus scenarios execute against both Go and Rust; snapshot cases execute through Go brain compatSnapshot and Rust. The existing contract-loader guard recognized only Rust cfg(test) modules and missed integration test files; it now recognizes test and tokio::test attributes. New backend implementation is still incomplete and production startup remains Go. Model/window normalization reuses claudemon Rust code. Migration inventory is services/hub-rs/migration.json, not a completed-parity assertion.
