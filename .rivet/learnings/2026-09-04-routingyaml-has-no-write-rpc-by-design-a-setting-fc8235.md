---
title: routing.yaml has no write RPC by design — a settings-driven pacing toggle must use a sibling file, never patch routing.yaml over the bus
date: 2026-09-04
confidence: high
suggested_doc: limit-aware-routing
related_paths:
  - services/hub/cmd/hub/main.go
  - services/hub/internal/routing/service.go
  - services/hub/internal/jobs/*.go
promoted: false
---

# routing.yaml has no write RPC by design — a settings-driven pacing toggle must use a sibling file, never patch routing.yaml over the bus

## Observation
services/hub/cmd/hub/main.go (around the usage/routing wiring, ~line 654-662) states explicitly: "The usage sampler is shared by routing decisions and Overview's usage.report reads... Neither exposes a routing configuration write RPC. Together with fs.write refusing the hub's state directory, that keeps routing.yaml ceilings outside the authority of bus callers." routing/service.go's Service only ever reads routing.yaml (30s content-hash poll for hand edits); there is no bus method anywhere that writes to it. This is a stated architectural invariant, not an oversight.

## Impact
Any feature that wants a user/Settings-driven change to pacing behavior (e.g. workweek curve: calendar vs workdays) must NOT be implemented as a hub RPC that edits routing.yaml, even though routing.yaml already has the exact curve/weekend_weight/weekend/weekend_reserve_pct knobs (internal/routing/pacing.go, routing.default.yaml ~line 499-548) that would otherwise be the obvious place to put it. Doing so would reverse a deliberate security boundary and needs an explicit decision, not a quiet workaround.

## Recommendation
Follow the jobs.json precedent (services/hub/internal/jobs, trusted-only jobs.* RPCs, file at <user-config-dir>/workspacer-hub/jobs.json) instead: a new small sibling file (e.g. usage-pacing.json) with its own trusted-only read/write RPCs, left entirely separate from routing.yaml and from routing.select's decision path (services/hub/internal/routing/policy.go calls m.PaceConfig() directly off the Matrix and would be unaffected).
