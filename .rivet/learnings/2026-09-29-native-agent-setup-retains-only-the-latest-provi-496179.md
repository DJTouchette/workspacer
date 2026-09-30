---
title: Native agent setup retains only the latest provider check
date: 2026-09-29
confidence: high
suggested_doc: native-embedded-backend
related_paths:
  - apps/native/src/ui/features.rs
  - apps/native/src/features.rs
promoted: false
---

# Native agent setup retains only the latest provider check

## Observation
Native agent setup stores its request and result in the single view.requests[setup] entry. Its installed array covers both providers, but readiness, readinessError and request errors belong only to Request::Setup.provider. Moving feedback into provider cards must scope readiness to that provider, and mark the other provider unverified rather than presenting the latest check as both agents' status.
