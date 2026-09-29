---
title: Synthetic evidence clocks must match production read-start timestamps
date: 2026-09-29
related_paths:
  - services/hub-rs/src/services/quiescence/sampler_tests.rs
  - services/hub-rs/src/services/quiescence/source.rs
promoted: false
---

# Synthetic evidence clocks must match production read-start timestamps

## Observation
The macOS preview at73a81f13 failed sampler_tests.rs stale-sample assertion. Production NativeSources records evidence.now_ms before awaited provider I/O, but the fixture overwrote it with the synthetic clock after I/O. A provider read already in flight across the simulated15-minute cutoff could therefore appear fresh at the advanced time. The test source now captures its clock before awaiting, matching production semantics.

## Impact
This is a fixture timestamp race rather than a production demand-sampling defect; increasing sleeps or weakening stale assertions would hide it.

## Recommendation
Injected async evidence sources should preserve production observation timestamp boundaries when tests advance synthetic time.
