---
title: History batching leaves lifecycle and inspector work synchronous and timing-blind
date: 2026-09-23
confidence: high
suggested_doc: session-lifecycle
related_paths:
  - apps/desktop/src/main/services/dispatchHistoryStore.ts
  - apps/desktop/src/main/ipc.ts
  - apps/desktop/src/renderer/src/components/TaskInspector.tsx
promoted: false
---

# History batching leaves lifecycle and inspector work synchronous and timing-blind

## Observation
Audited current 250ms history batching. queueObservation immediately flushes on every ended observation or lifecycle change; transaction still synchronously locks, reloads, clones, repeatedly JSON-stringifies, and fsyncs the whole bounded history. Only the timer callback records dispatch-history-timing, so lifecycle-triggered, admission, explicit observe and shutdown writes are unmeasured. withConfigLock can synchronously wait 250ms per transaction. DISPATCH_HISTORY_READ calls listRequests separately per manager and listForHostUser once; each resets the cache and reparses the full history. TaskInspector and RecentAgentsPane each independently poll every 3000ms. Read paths do not flush queued metrics; they see up to 250ms old metrics. Headless desktopHost context refresh still observes each changed tracked snapshot synchronously rather than queueing. Traced desktop admission calls observe after accept, so a pre-admission queue being drained before row creation does not lose the final admission snapshot on that path. Focused dispatch-history, task-inspector and workflow-store tests pass (39 tests).

## Impact
The steady-state streaming benchmark improvement does not cover synchronized fleet lifecycle bursts or concurrent inspector polling, and current timing logs cannot rule those out as stalls.

## Recommendation
Measure transaction duration including lock wait for every trigger; provide one coherent history read returning tasks and requests and share renderer polling. Keep critical lifecycle durability while considering async serialized persistence separately; benchmark state transitions and populated request history as well as steady streaming.
