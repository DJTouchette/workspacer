---
title: Bridged backend can need IPC overrides for partly bus-backed methods
date: 2026-08-26
confidence: high
suggested_doc: renderer-backend-seam
related_paths:
  - apps/desktop/src/renderer/src/backend/bridgedBackend.ts
  - apps/desktop/src/renderer/src/backend/webBackend.ts
  - apps/desktop/src/main/ipc.ts
promoted: false
---

# Bridged backend can need IPC overrides for partly bus-backed methods

## Observation
workflowAgentTranscript and workflowAgentConversation are now bus-backed in createWebBackend for provider-native subagent rows, but Claude workflow-run drill-in still depends on local workflowWatcher artifact files. In bridged desktop mode, leaving these methods purely on the web backend would make runId-backed workflow rows return null even though preload IPC can read them.

## Impact
Prevents default desktop bridged mode from regressing features that still rely on main-process local files while adding web/remote bus parity.

## Recommendation
For ElectronAPI methods with mixed transport needs, keep the web implementation for browser/remote clients, and add an explicit createBridgedBackend override that routes the local-only branch to preload IPC while preserving the bus path where it applies.
