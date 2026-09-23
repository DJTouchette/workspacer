---
title: History timing covers all writes and inspector reads share one snapshot
date: 2026-09-23
confidence: high
suggested_doc: session-lifecycle
related_paths:
  - apps/desktop/src/main/services/dispatchHistoryStore.ts
  - apps/desktop/src/main/headless/desktopHost.ts
  - apps/desktop/src/renderer/src/hooks/useDispatchHistory.ts
promoted: false
---

# History timing covers all writes and inspector reads share one snapshot

## Observation
Audit fixes move slow history logging into transaction(), measuring lockWaitMs, writeMs, total duration, success and trigger for synchronous lifecycle/admission/mutation and timer paths. readForHostUser reads task and request summaries from one loaded file, used by Electron IPC and headless handlers. Headless internal.observe fingerprints only history metrics and commits changed observations in one synchronous transaction; observation digests advance only after successful persistence. Renderer TaskInspector and RecentAgentsPane subscribe to one poller with a single three-second timer and in-flight request; authoritative task edits invalidate earlier reads, and last unsubscribe clears state/timer and fences late responses.

## Impact
Many managers or inspector panes no longer multiply full history parses or polling. Headless fleet refresh no longer fsyncs once per changed session. Critical lifecycle durability remains synchronous and is now visible in slow logs.

## Recommendation
Retain replay-under-lock and generation guards when extending polling. Measure common transaction logs before moving writes to an async worker. Tests cover shared polling, stale read suppression, one history read, one batch write, retry after failure, and synchronous lifecycle timing.
