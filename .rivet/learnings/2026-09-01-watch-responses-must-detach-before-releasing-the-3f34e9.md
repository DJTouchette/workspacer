---
title: Watch responses must detach under the sweep mutex before delivery
date: 2026-09-01
confidence: high
suggested_doc: session-lifecycle
related_paths:
  - services/hub/cmd/brain/agentops.go
  - services/hub/cmd/brain/agentops_race_test.go
  - apps/desktop/src/main/services/thresholdWatch.ts
promoted: false
---

# Watch responses must detach under the sweep mutex before delivery

## Observation
Hub threshold watches are mutable while armed because sweepThresholds binds context telemetry and changes state. notifyWhen must deep-copy every pointer-bearing field while watchMu is held, then marshal and deliver that detached copy after unlocking. Returning or marshalling the map-owned *thresholdWatch after unlock races the sweep; marshalling the detached value does not.

## Impact
A live watch pointer can be read by JSON encoding concurrently with sweep mutation, producing a Go data race and an incoherent arm response.

## Recommendation
For any lock-protected mutable state returned over an API, deep-copy it under the lock, release the lock, then serialize and deliver only the detached copy; add a detached-snapshot regression and run the package with -race.
