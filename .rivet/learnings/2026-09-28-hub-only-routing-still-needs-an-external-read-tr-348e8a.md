---
title: Hub-only routing still needs an external read transport and the actual Claude catalog owner
date: 2026-09-28
author: codex
confidence: high
suggested_doc: hub-bus-control-plane
related_paths:
  - services/hub-rs/src/services/external_claudemon.rs
  - services/hub-rs/src/services/routing/sampler.rs
promoted: false
---

# Hub-only routing still needs an external read transport and the actual Claude catalog owner

## Observation
Rust control_plane_only skips desktop install_config, but routing registration and services::install layout/pacing already run. The missing parity is UsageSampler transport when engine is None. Go routingCatalog calls claude.listModels over the bus (aliases plus seen); a successful empty Claude answer means unknown, whereas successful empty provider models means unavailable. The old lightweight /events bridge publishes agent.state_changed only; it does not own daemon processes, register desktop methods, or reconcile session.resync snapshots.

## Impact
Duplicating full desktop services in Electron hub-only would collide with provider registration. Using static Claude aliases would omit observed concrete models; treating transport errors as an empty catalog would disable providers incorrectly.

## Recommendation
Inject a host-configured read-only external daemon adapter into existing routing, preserve error versus empty responses, query the current bus model owner after readiness, and own/cancel only the SSE connection. External daemon remains independently owned.
