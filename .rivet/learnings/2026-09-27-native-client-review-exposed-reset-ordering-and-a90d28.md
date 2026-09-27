---
title: Native client review exposed reset ordering and hidden allocation retention
date: 2026-09-27
confidence: high
suggested_doc: renderer-backend-seam
related_paths:
  - apps/native
promoted: false
---

# Native client review exposed reset ordering and hidden allocation retention

## Observation
Independent review of apps/native found that String::drain retains oversized capacities after visible text clipping; clipping now copies the bounded UTF-8 suffix and tests capacity. Hub events and RPC replies arrive through different worker queues, so resets always trigger a fresh snapshot even outside an apparent pending read. Readiness also triggers a reseed to close the asynchronous demand gap. GPUI list invalidation compares row identities and counts because revisions can collide after reselection.

## Impact
Length-only budgets and happy-path sequence tests miss memory retention and stale transcript/list states.

## Recommendation
Preserve the allocation, reset-ordering, readiness, and equal-revision UI regressions in apps/native.
