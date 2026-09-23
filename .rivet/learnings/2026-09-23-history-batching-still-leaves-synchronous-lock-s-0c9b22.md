---
title: History batching still leaves synchronous lock stalls outside timer timing logs
date: 2026-09-23
confidence: high
suggested_doc: agent-spawn
related_paths:
  - apps/desktop/src/main/services/dispatchHistoryStore.ts
  - apps/desktop/src/main/lib/configLock.ts
  - apps/desktop/src/main/lib/fileLock.ts
promoted: false
---

# History batching still leaves synchronous lock stalls outside timer timing logs

## Observation
The desktop dispatch history batching patch queues metrics but lifecycle transitions still call flush synchronously (dispatchHistoryStore.ts:90-96), and transaction enters withConfigLock (119-123). configLock maxWaitMs is 250 and fileLock retries using Atomics.wait on the calling thread (fileLock.ts:27-30,96-97). The newly added dispatch-history-timing log wraps only the scheduled observations callback (dispatchHistoryStore.ts:101-114), so immediate lifecycle transactions are not measured. This is a remaining limitation, not a newly introduced locking regression.

## Impact
Concurrent desktop/brain history writes can still pause Electron main-process IPC and launches for up to the lock wait budget, while the new history timing log stays quiet for the immediate path.

## Recommendation
Instrument the common transaction boundary with total duration and lock acquisition duration for both immediate and batched operations; evaluate a serialized background writer while retaining the cross-process locking contract and lifecycle durability.
