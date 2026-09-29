---
title: Caller parameter audit needs per-binding provenance and separate binary import scopes
date: 2026-09-29
suggested_doc: hub-shared-cap-event-vocabulary
related_paths:
  - tools/capability-source-check/**
  - apps/desktop/tests/fixtures/capability-parameter-policy.json
promoted: false
---

# Caller parameter audit needs per-binding provenance and separate binary import scopes

## Observation
The Rust scanner initially merged main.rs imports into the library root and let external Options hide the real runtime Options receiver. Its unknown-match result also kept the final default literal, skipping library.save mcp fields. Separate binary scope plus conservative branch joins recover all 84 original Go dangerous bindings. Explicit opaque payload path decisions prevent an unrelated field reason from excusing arbitrary map payloads.

## Impact
Aggregate counts and vocabulary membership cannot prove caller binding coverage; both defects looked like safe empty results.

## Recommendation
Keep source-hashed original per-method binding checks, structural negative scanner tests and explicit opaque path policy in the CI source guard.
