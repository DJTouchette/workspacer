---
title: Manager request receipt originates after delivery acknowledgement
date: 2026-09-15
confidence: high
suggested_doc: chat-tool-rendering
related_paths:
  - apps/desktop/src/renderer/src/panes/ClaudePane.tsx
  - apps/desktop/src/main/ipc.ts
  - apps/desktop/src/main/services/managerRequestService.ts
promoted: false
---

# Manager request receipt originates after delivery acknowledgement

## Observation
Before this change, ClaudePane handleSend posted the routine Request received notification only after managerRequestPrepare returned a durable request ID and claudeMessage acknowledged the same ID as pending or accepted. The removed notifiedRequests set existed only to deduplicate that toast; requestRetry and requestCaptureStatus instead preserve retry identity and delivery failure/uncertainty. Capture is local-authority-only in the IPC handler and ManagerRequestService.prepare. Removing the producer leaves pending conversation markers and failure status intact.

## Recommendation
Keep routine acknowledgements suppressed at this producer, without filtering the shared notification bus or changing capture/delivery semantics.
