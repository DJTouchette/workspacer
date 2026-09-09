---
title: Local Rust release checks can omit debug symbols to fit the memory high watermark
date: 2026-09-08
confidence: high
related_paths:
  - docs/release-checks.md
  - services/claudemon/Cargo.toml
promoted: false
---

# Local Rust release checks can omit debug symbols to fit the memory high watermark

## Observation
On this Linux host cargo test -j1 with default debug symbols spent minutes reclaim-throttled during the claudemon link at the 1536 MiB MemoryHigh, without hitting MemoryMax. Cancelling only its release-check launcher cleaned up the unique service. Rebuilding with CARGO_PROFILE_DEV_DEBUG=0 and CARGO_PROFILE_TEST_DEBUG=0 finished the unoptimized test profile under the same cap and passed 874 unit plus 8 integration tests (existing ignored fixtures/doctest retained).

## Recommendation
Use these per-command Cargo profile overrides for constrained local validation and retain the resource logs. They remove debug symbols, not assertions or test selection. Hosted CI remains responsible for its default profile and platform builds.
