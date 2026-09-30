---
title: Routing mirror sampling and caller cancellation have separate owners
date: 2026-09-29
suggested_doc: limit-aware-routing
related_paths:
  - services/hub-rs/src/services/routing/sampler.rs
  - services/hub-rs/src/services/routing/preview.rs
promoted: false
---

# Routing mirror sampling and caller cancellation have separate owners

## Observation
The legacy hub sampler detached a ten-second fetch from each three-second caller and published late cache results even when every waiter left. Rust now restores that independence with a spawned producer; full library checkpoint382 passed both cancellation and caller-timeout tests. The legacy thirty-second hub mirror prefetch is redundant with daemon-owned account polling and the renderer useUsageReport sixty-second subscriber timer; selection and preview still request zero-age observations, Overview permits at most sixty seconds. Preview previously returned the private full decision and reused completed usage; its new allowlist and zero-age path require separate actual-bus proof before migration certification.

## Impact
Cancelling one UI request must not discard a quota observation; private directory ceiling paths must not escape the preview DTO.

## Recommendation
Keep caller and fetch deadlines distinct, retain unknown/error invalidation semantics, and verify preview has no audit or catalog-probe side effects.
