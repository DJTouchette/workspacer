---
title: Dispatch history stage and retry provenance are non-authorizing metadata
date: 2026-09-06
confidence: high
suggested_doc: agent-spawn
related_paths:
  - apps/desktop/src/main/services/dispatchHistoryStore.ts
  - apps/desktop/src/main/services/hubCapabilities.ts
promoted: false
---

# Dispatch history stage and retry provenance are non-authorizing metadata

## Observation
dispatchHistoryStore.validate must only refuse explicit taskId/afterDispatchId links without a live local manager. Optional stage and host-stamped retrySourceSessionId can be present when ownership is absent or has changed; recording must degrade to unrecorded/fresh instead of borrowing the prior manager's task.

## Impact
Treating descriptive metadata as required attribution rejects valid worker launches and turns an unavailable history projection into a launch gate.

## Recommendation
Keep explicit continuation ownership/project checks pre-launch, and resolve retry sources only after confirming the current live owner's scope.
