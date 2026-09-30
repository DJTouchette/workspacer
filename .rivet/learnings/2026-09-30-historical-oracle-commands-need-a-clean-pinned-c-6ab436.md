---
title: Historical oracle commands need a clean pinned checkout before any compiler starts
date: 2026-09-30
confidence: high
related_paths:
  - scripts/hub-reference.py
  - scripts/test_hub_reference.py
  - Makefile
  - apps/desktop/package.json
promoted: false
---

# Historical oracle commands need a clean pinned checkout before any compiler starts

## Observation
Prep-only hub-reference.py centralizes Make/npm optional Go oracle commands behind WKS_HUB_REFERENCE_ROOT. It verifies the sealed capture, pinned Git HEAD/tree, clean checkout, tracked-file presence and captured source hashes before execution; ordinary Rust/TS defaults do not call it. Parity preserves full Rust, six Go fixture filters, four desktop tests, ignored compatibility with an explicitly built temporary Go binary, and exact vocabulary comparison. Go modules are readonly; routing harness parking is refused. Unit tests use synthetic historical I/O and mocked subprocesses so they remain runnable without Git/Go or original Go files.

## Recommendation
Keep optional historical execution distinct from portable CI and release gates. Never fall back to current services/hub or overwrite retained vocabulary through a default target; export only explicitly for review.
