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

The race scheduler cannot promise to interleave a tiny JSON marshal with every
sweep on every machine. The Hub regression therefore combines real concurrent
notifyWhen/sweepThresholds race stress with a Go-AST contract over notifyWhen:
the result of `snapshotThresholdWatch` must be produced before unlock and the
same result must reach `jsonResult` after it. This makes the exact live-pointer
return mutation deterministically red without a production hook or mutable test
seam.

## Impact
A live watch pointer can be read by JSON encoding concurrently with sweep mutation, producing a Go data race and an incoherent arm response.

## Recommendation
For any lock-protected mutable state returned over an API, deep-copy it under the lock, release the lock, then serialize and deliver only the detached copy; add a detached-snapshot regression and run the package with -race.
