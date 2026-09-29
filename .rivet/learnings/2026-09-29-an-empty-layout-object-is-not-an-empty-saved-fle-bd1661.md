---
title: An empty layout object is not an empty saved fleet
date: 2026-09-29
related_paths:
  - services/hub-rs/src/services/snapshots.rs
promoted: false
---

# An empty layout object is not an empty saved fleet

## Observation
Go layoutSessionIDs recognizes a layout only when data.agents is a valid present array. Rust layout_ids previously returned hasLayout for any data object, making data:{} or agents:null hide recently stopped sessions instead of using headless fallback. It now validates the legacy known layout field shapes, distinguishes absent/null from an explicit empty agents array, and validates scalar visibility fields instead of treating wrong types as live. snapshots::live provides the separate process-liveness predicate used by readiness/library watchers.
