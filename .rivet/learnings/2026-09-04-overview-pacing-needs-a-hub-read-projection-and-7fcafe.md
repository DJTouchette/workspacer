---
title: Overview pacing needs a hub read projection and coherent window readings
date: 2026-09-04
confidence: high
suggested_doc: limit-aware-routing
related_paths:
  - apps/desktop/src/renderer/src/panes/OverviewPane.tsx
  - apps/desktop/src/main/shared/usageReport.ts
  - apps/desktop/src/renderer/src/backend/webBackend.ts
  - services/hub/cmd/hub/routingselect.go
  - services/hub/cmd/hub/routing.go
promoted: false
---

# Overview pacing needs a hub read projection and coherent window readings

## Observation
At 43ba1dc0 Overview RateLimitCard merges cached live percentage/reset/duration independently and reportWindowsFor only fills fields never seen live. A hub PaceReport tick attached to this existing merged fill can describe a different observation or reset. usageReport IPC reads claudemon /usage/report directly; webBackend usageReport returns null. Hub routingSelect is unsuitable for polling: it requires role, refreshes availability (can boot provider CLIs), appends decision log and publishes routing.decision. Reuse usageWatcher.Latest + Snapshot.Buckets(now) + Matrix.PaceConfig + limits.PaceFor in a dedicated hub read projection; return canonical provider/account/window identity and the consumed value with the expected tick.
