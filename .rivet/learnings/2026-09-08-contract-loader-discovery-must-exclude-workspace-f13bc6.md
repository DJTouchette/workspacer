---
title: Contract loader discovery must exclude Workspacer runtime caches
date: 2026-09-08
confidence: high
related_paths:
  - services/hub/cmd/brain/contracts_test.go
  - docs/release-checks.md
promoted: false
---

# Contract loader discovery must exclude Workspacer runtime caches

## Observation
The brain contract-loader guard read and retained all TS/Go/Rust files below .workspacer/cache, including copied Go modules and Cargo registry sources. In the isolated release check, brain.test reached about 1.6 GiB anonymous memory and was reclaim-throttled at MemoryHigh; lowering GOMEMLIMIT did not fix the retained dataset. Verbose output identified TestEveryContractFixtureHasAtLeastTwoLoaders. Excluding .workspacer prevents both cache memory growth and stale cached files falsely satisfying cross-language loader counts.

## Recommendation
Keep .workspacer in contractSkipDirs. The scratch-tree regression must still discover real TS, Go and Rust source files while rejecting runtime cache and nested-worktree copies. Preserve extinput-backed reads of actual source.
