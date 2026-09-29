---
title: Caller source guard distinguishes key validation from payload forwarding
date: 2026-09-29
suggested_doc: hub-shared-cap-event-vocabulary
related_paths:
  - tools/capability-source-check/**
promoted: false
---

# Caller source guard distinguishes key validation from payload forwarding

## Observation
Nested Rust functions resolve helper names in lexical declaration scope. Known empty method-specific key arrays must not generate unknown field reads. New Git validation traverses map keys only; separate per-method inspection decisions accept that only if no opaque transformation at the same path is observed.

## Impact
A generic whole-payload review for validation would silently permit later serialization or forwarding; treating finite empty arrays as unknown hides useful scanner precision.

## Recommendation
Keep lexical helper and empty-array regression fixtures, and fail key-only reviews when serialization/value inspection or unknown raw receivers are introduced.
