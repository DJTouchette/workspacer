---
title: usage.report's zero-parameter contract is machine-pinned, not just documented
date: 2026-09-04
confidence: high
suggested_doc: limit-aware-routing
related_paths:
  - services/hub/cmd/hub/usagereport.go
  - services/hub/internal/capspec/composition.go
  - services/hub/internal/capspec/capspec.go
promoted: false
---

# usage.report's zero-parameter contract is machine-pinned, not just documented

## Observation
services/hub/cmd/hub/usagereport.go explicitly rejects any non-empty params (`if len(args) != 0 { return nil, fmt.Errorf(...) }`). capspec/composition.go:843-845 classifies usage.report as an inertMethod ("accepts no caller values") specifically because it carries no parameter, and says cmd/hub/usagereport_test.go "pins its no-parameter/read-only seam". capspec.go:339 also states "no parameters" in the method's Reason string. This was an independent-review-approved acceptance criterion (docs/reviews/overview-usage-pacing.md line 21).

## Impact
A feature that needs the Overview usage.report projection to vary per caller (e.g. a 5-day-vs-7-day work-week pacing schedule chosen in Settings) cannot be implemented by adding a request parameter to usage.report — that would break a machine-pinned contract test and the inertMethods/compositionActors classification. Any personalization of what usage.report returns must come from hub-side persisted state that the zero-param handler reads internally, not from caller-supplied arguments.

## Recommendation
For a pacing-schedule (or similar) Settings toggle, add a separate small persisted preference (its own hub-owned file, its own read/write RPC pair) that usage.report's handler consults internally when building PaceConfig for the projection. Keep usage.report itself at zero parameters.
