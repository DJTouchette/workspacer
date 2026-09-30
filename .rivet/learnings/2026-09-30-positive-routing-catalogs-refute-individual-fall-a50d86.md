---
title: Positive routing catalogs refute individual fallback assignments
date: 2026-09-30
suggested_doc: limit-aware-routing
related_paths:
  - services/hub-rs/src/services/routing.rs
  - services/hub-rs/src/services/routing/sampler/catalog_tests.rs
promoted: false
---

# Positive routing catalogs refute individual fallback assignments

## Observation
Go routing service converts answered catalog model/effort mismatches into Matrix.Issues, and its fallover judge skips the affected assignment. The Rust adapter previously checked only provider-level unavailable state, so an available provider could still receive an unsupported model/effort alternative. Rust now checks cached positive model/effort evidence inside the existing fallover walk, with case-insensitive trimmed comparison; unknown stays fail-open. Both actual HTTP producer and candidate-refutation regressions passed in the extended387/389 and388/390 checkpoints; unrelated/fixture failures mean neither is a full-suite green receipt.

## Impact
Provider availability alone does not prove a particular model/effort is launchable.

## Recommendation
Keep measured-empty, unknown transport/decode failure and positive catalog evidence distinct; preserve scoped operator routing reads and trusted host-only preference mutations.
