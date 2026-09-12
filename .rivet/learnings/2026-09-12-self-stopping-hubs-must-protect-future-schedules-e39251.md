---
title: Self-stopping hubs must protect future schedules and running shell jobs
date: 2026-09-12
confidence: high
suggested_doc: hub-jobs
related_paths:
  - services/hub/internal/quiescence/quiescence.go
  - services/hub/cmd/hub/quiescence.go
promoted: false
---

# Self-stopping hubs must protect future schedules and running shell jobs

## Observation
The generic quiescence predicate only considers near-term jobs and deliberately skips shell jobs so an advisory idle-check shell job does not veto itself. That policy is unsuitable once a hub automatically powers itself off: an enabled job hours away would be stranded, and a long shell job could be killed. KeepJobsAwake is set only for WKS_MACHINE_IDLE_MODE=stop; it blocks all scheduled jobs and all running jobs, while preserving the ordinary diagnostic defaults.
