---
title: Bus parity distinguishes retired tokenless listeners from missing authentication behavior
date: 2026-09-29
confidence: high
related_paths:
  - services/hub-rs/tests/bus_remaining.rs
  - services/hub-rs/src/runtime.rs
  - services/hub/internal/bus/bench_test.go
promoted: false
---

# Bus parity distinguishes retired tokenless listeners from missing authentication behavior

## Observation
Go bus fixtures permit unconfigured-token WebSockets and detailed public health, but Rust Hub::start explicitly refuses all network listeners without an explicit host token. This is a deliberate startup invariant, so removing only handshake rejection would be an incorrect parity fix. Embedded trusted owners remain supported without a network credential. New bus_remaining tests cover the refusal alongside authenticated owner/operator/peer caller identity. Go bench_test.go also contains a real baseline-adjusted5ms p99 assertion, not just optional benchmarks; do not mark that file retired without retaining or deliberately reviewing the budget.

## Recommendation
Record the tokenless-network architecture difference explicitly in migration evidence; retain caller provenance and the performance guard as separate obligations.
