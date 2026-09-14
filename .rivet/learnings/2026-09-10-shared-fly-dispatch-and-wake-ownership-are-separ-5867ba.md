---
title: Shared-Fly dispatch and wake ownership are separate
date: 2026-09-10
confidence: medium
suggested_doc: session-lifecycle
related_paths:
  - apps/desktop/src/main/services/remoteDispatchRegistry.ts
  - apps/desktop/src/main/services/pairedDispatch.ts
  - services/hub/cmd/brain/remotedispatch.go
  - services/hub/cmd/brain/finishwake.go
  - services/hub/cmd/brain/blockwake.go
promoted: false
---

# Shared-Fly dispatch and wake ownership are separate

## Observation
For shared-Fly coexistence, the repo has separate desktop paired-dispatch and headless brain remote-dispatch paths: paired dispatch identifies workers with protocol-2 remoteOrigin dispatch IDs, while finish/block wakes are parent/wake-target keyed. The key verification target is origin/owner filtering, not a single global manager.

## Impact
A second local Fleet Manager should not be assumed to adopt or receive another client's work merely because all workers share one host.

## Recommendation
Trace remoteOrigin/owner checks and parent isWakeTarget checks independently, then verify shared-layout and capability-tier behavior.
