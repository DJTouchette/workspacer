---
title: Rust routing migration: Overview schedule is not routing authority
date: 2026-09-28
suggested_doc: limit-aware-routing
related_paths:
  - services/hub-rs/src/services/routing.rs
  - services/hub/internal/usageprefs/usageprefs.go
promoted: false
---

# Rust routing migration: Overview schedule is not routing authority

## Observation
The Go usageprefs.ApplySchedule sidecar is consumed only by usage.report, never routing.select. The new Rust RoutingService retains this distinction: apply_schedule changes report projections while matrix and selection remain identical, covered by overview_schedule_does_not_modify_routing_policy. A malformed managed overlay must preserve valid host ceilings at boot; falling back wholesale to compiled defaults would erase stricter operator ceilings.

## Impact
Wiring the same schedule overlay into routing would silently change provider allocation when an operator edits a display preference; ignoring valid host policy on sidecar errors weakens spawn limits.

## Recommendation
Keep Overview pacing overlays separate from routing policy and retain the host matrix if only managed preferences fail validation. Compare model classification against inherited host authority even after managed assignment edits.
