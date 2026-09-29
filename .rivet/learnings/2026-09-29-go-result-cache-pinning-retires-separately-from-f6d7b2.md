---
title: Go result-cache pinning retires separately from corpus completeness
date: 2026-09-29
promoted: false
---

# Go result-cache pinning retires separately from corpus completeness

## Observation
The extinput lexical traversal trick and cachepin AST tests only defeat cmd/go result caching for out-of-module readers. With default test-hub and CI now running Cargo/Vitest, retain actual source discovery, missing-file failures, mutation checks and corpus floors; do not invent a Rust extinput wrapper or retire the unrelated sweepguard accounting requirements. Rust corpus_ownership derives the checkout from CARGO_MANIFEST_DIR and reads directory listings at execution, with explicit build/runtime cache exclusions.
