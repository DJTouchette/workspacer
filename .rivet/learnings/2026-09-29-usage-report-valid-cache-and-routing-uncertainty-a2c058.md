---
title: Usage report valid cache and routing uncertainty are separate contracts
date: 2026-09-29
suggested_doc: usage-accounting
related_paths:
  - services/hub-rs/src/services/routing/sampler.rs
  - services/hub-rs/src/services/routing/sampler/report_tests.rs
promoted: false
---

# Usage report valid cache and routing uncertainty are separate contracts

## Observation
A full read of Go usageWatcher.runFetch is essential: it sets haveSnap=false on a failed fetch. LatestWithin's fresh-cache fast path therefore does NOT allow a known failed refresh to revive old observations. Rust previously kept cache.report after that failure; it now clears it for the current fetch generation. A cache with no failed refresh remains eligible for 60 seconds. usage.report returns an error on an uncached transport failure; routing.select separately converts missing quota into unknown capacity plus warning.

The usage.report production branch returns before catalog refresh, decision selection, logging and publication. A new actor/HTTP fixture covers twelve view-scoped reads, exact validity, unknown measurements, and unchanged policy/log/event/catalog state. A separate raw-sampler assertion proves failed-refresh invalidation on the same sampler. Tests remain subject to their recorded validation checkpoint.

## Impact
Keeping an old cached report after this sampler has observed failure can turn an outage into a successful stale quota observation. The earlier conclusion drawn from LatestWithin alone was incomplete; runFetch owns the validity transition.

## Recommendation
Trace both cache lookup and mutation paths. Preserve valid-cache reuse until a known failure, clear it on failure, and keep usage read errors distinct from routing's explicit unknown-capacity fallback.
