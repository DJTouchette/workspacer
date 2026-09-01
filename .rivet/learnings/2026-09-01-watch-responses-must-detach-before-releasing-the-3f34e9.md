---
title: Watch responses must detach before releasing the sweep mutex
date: 2026-09-01
confidence: high
suggested_doc: session-lifecycle
related_paths:
  - services/hub/cmd/brain/agentops.go
  - services/hub/cmd/brain/agentops_race_test.go
  - apps/desktop/src/main/services/thresholdWatch.ts
promoted: false
---

# Watch responses must detach before releasing the sweep mutex

## Observation
Hub threshold watches are mutable while armed because sweepThresholds binds context telemetry and changes state. notifyWhen must snapshot every pointer-bearing field and marshal the value while watchMu is held; returning or marshalling the map-owned *thresholdWatch after unlock races the sweep.

## Impact
A live watch pointer can be read by JSON encoding concurrently with sweep mutation, producing a Go data race and an incoherent arm response.

## Recommendation
For any lock-protected mutable state returned over an API, construct and serialize a detached value under the lock; add a detached-snapshot regression and run the package with -race.
