---
title: Routing harness parks missing routing.select
date: 2026-08-30
promoted: false
---

# Routing harness parks missing routing.select

## Observation
The limit-aware routing runtime harness in services/hub/scripts/routing-limit-harness.mjs runs fake claudemon plus scratch-hub checks today, but treats the routing decision assertions as explicit PENDING when the bus reports no provider for routing.select. Set ROUTING_HARNESS_REQUIRE_ROUTING=1 on make test-routing-harness after the routing layer lands to make that absence fatal and require the stale-window, reset-now, unavailable-quota, and absent-provider assertions to run.
