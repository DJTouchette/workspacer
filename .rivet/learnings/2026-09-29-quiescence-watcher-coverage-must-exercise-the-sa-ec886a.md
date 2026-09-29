---
title: Quiescence watcher coverage must exercise the sampler and provider seam
date: 2026-09-29
promoted: false
---

# Quiescence watcher coverage must exercise the sampler and provider seam

## Observation
Read-only audit of cmd/hub/quiescence.go and all eight tests found Rust pureMonitor/corpus and broker activity tests, but no test that instantiates sampler::Watcher or runs its demand-gated source reader. Original tests cover realbus answers, infrastructure filtering, exact poller self-exclusion/reactivation, cold stale shape, jobs, missing/unknown session evidence, and15minute demand expiry. NativeSources currently reads only optional EmbeddedClient; central hub-only deployment with a registered session provider may differ from original bus-based localSessions, which must be resolved before certifying this source pair. No behavioral change or certification made in this audit.
