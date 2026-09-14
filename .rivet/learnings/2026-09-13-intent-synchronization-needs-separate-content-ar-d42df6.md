---
title: Intent synchronization needs separate content artifacts and observation events
date: 2026-09-13
confidence: high
related_paths:
  - apps/desktop/src/main/services/intentSourceSyncStore.ts
  - apps/desktop/src/main/services/intentSourceSync.ts
promoted: false
---

# Intent synchronization needs separate content artifacts and observation events

## Observation
A recovered external object can have the same digest as an earlier successful observation. Ordering only deduplicated artifact rows leaves a later missing tombstone displayed as latest after recovery. Schema v7 uses immutable observation events referencing deduplicated payload digests, while projections retain last success separately from failures. ADO work-item sync must preserve legacy read snapshot field shape or reviewed comment preflight falsely reports drift.

## Recommendation
Keep event ordering independent of artifact deduplication; run the recovery and legacy snapshot digest tests when changing provider projection or storage.
