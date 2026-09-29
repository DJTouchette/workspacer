---
title: Rust cross-repo guards reject missing checkout markers without Go sources
date: 2026-09-29
suggested_doc: hub-shared-cap-event-vocabulary
related_paths:
  - services/hub-rs/tests/support/repo.rs
promoted: false
---

# Rust cross-repo guards reject missing checkout markers without Go sources

## Observation
The original sweepguard distinguishes absent checkout from moved markers because Go callers skip absent checkout. Rust corpus guards use manifest-anchored roots and never skip extracted inputs; now their shared reader requires Makefile plus services/hub-rs/Cargo.toml, and missing or moved runtime fixture reads are explicit errors. Cargo reruns runtime reads rather than inheriting Go cached-test external-input semantics.

## Impact
Renaming a root marker must not silently weaken corpus ownership checks, and deleting retained Go sources must not turn the Rust reader into a no-checkout skip.

## Recommendation
Use tests/support/repo.rs for corpus guard roots. Keep GateCounter/TestMain retirement separately pending until the old handwritten host-gated groups and their Rust replacements are mapped.
