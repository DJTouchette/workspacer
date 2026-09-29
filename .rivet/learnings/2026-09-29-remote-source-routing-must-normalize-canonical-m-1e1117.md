---
title: Remote source routing must normalize canonical models and distinguish paired local project from remote cwd
date: 2026-09-29
confidence: high
related_paths:
  - services/hub-rs/src/services/remote_dispatch/origin.rs
  - services/hub-rs/src/services/remote_dispatch/paired.rs
  - services/hub-rs/src/services/routing.rs
promoted: false
---

# Remote source routing must normalize canonical models and distinguish paired local project from remote cwd

## Observation
Actual Go named-model classification requires an explicit provider; do not silently invent destination defaults. Rust spawn_plan normalizes model/modelIdentity/contextWindow but the routing sanitizer previously read only legacy model, permitting canonical-only or conflicting carriers to escape source checks. Origin.forward_booked is the shared source boundary for qualified and paired dispatch; paired replaces wire.cwd with remoteCwd, while its private Booking retains the validated local project. New gate must use that trusted project for source policy, restore unchanged remote wire.cwd, and never trust caller JSON sourceRoot. Source and destination audits need distinct routingLocation markers.

## Recommendation
Retain two-real-hub positive destination floors and source-before-forward refusal tests, plus a paired project-vs-remote-cwd regression. Provider-omitted/profile-derived effective selection remains explicitly outside source named-model certainty.
