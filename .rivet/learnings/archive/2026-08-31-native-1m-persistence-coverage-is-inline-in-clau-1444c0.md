---
title: Native 1M persistence coverage is inline in claudemon Rust modules
date: 2026-08-31
suggested_doc: claudemon-sqlite-store
related_paths:
  - services/claudemon/src/store/mod.rs
  - services/claudemon/src/store/schema.rs
  - services/claudemon/src/session/usage.rs
promoted: true
promoted_to: claudemon-sqlite-store
---

# Native 1M persistence coverage is inline in claudemon Rust modules

## Observation
The reviewed native-1M persistence change keeps its focused regression coverage inside the touched Rust modules: store/mod.rs covers daemon-restart requested-model round trips, store/schema.rs covers migration replay, and session/usage.rs covers resolving the 1M window from the requested model. Focused validation can therefore use cargo test name filters against the claudemon crate without a separate integration-test target.

## Recommendation
When validating persistence/schema or native-1M regressions, run the targeted claudemon unit tests by name and include the full claudemon crate checks for cross-module compilation.

## Disposition
Promoted into `.rivet/context/modules/claudemon-sqlite-store.md`. Re-verified: the coverage is still inline `#[cfg(test)]` in `store/mod.rs`, `store/schema.rs` and `session/usage.rs`, so a `cargo test` name filter inside the claudemon crate is the right validation.
