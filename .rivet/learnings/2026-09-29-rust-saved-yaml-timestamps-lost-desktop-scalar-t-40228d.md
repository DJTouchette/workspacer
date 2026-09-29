---
title: Rust saved YAML timestamps lost desktop scalar typing
date: 2026-09-29
suggested_doc: renderer-backend-seam
related_paths:
  - services/hub-rs/src/services/stores.rs
  - services/hub-rs/tests/provider_parity.rs
promoted: false
---

# Rust saved YAML timestamps lost desktop scalar typing

## Observation
Measured actual serde_yaml output leaves generated ISO timestamps unquoted; installed desktop js-yaml parses those scalars as Date while Rust serde_yaml-to-JSON reads strings. The legacy parity fixture expects non-string YAML timestamps to sort after real text rows and sessions.list to expose an empty timestamp, while sessions.load preserves its document projection. The exact new parity integration fails on this ordering and on whitespace-only library title fallback.

## Impact
This affects Rust-written sessions/layouts viewed by Electron, not only an artificial hand-edited fixture. A global timestamp-to-empty rewrite would corrupt load responses, and a generic emitter switch would broaden the change unnecessarily.

## Recommendation
Recover root-field style from safe YAML parser events, keep load document intact, separate list sort keys, quote only generated root timestamp strings, and verify actual saved bytes with the desktop loader.
