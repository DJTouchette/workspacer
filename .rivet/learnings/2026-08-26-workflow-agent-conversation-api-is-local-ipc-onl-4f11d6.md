---
title: Workflow agent conversation API is local IPC-only
date: 2026-08-26
confidence: high
suggested_doc: renderer-backend-seam
related_paths:
  - apps/desktop/src/main/ipc.ts
  - apps/desktop/src/main/preload.ts
  - apps/desktop/src/renderer/src/backend/webBackend.ts
  - apps/desktop/src/renderer/src/components/claude/InspectorCard.tsx
promoted: false
---

# Workflow agent conversation API is local IPC-only

## Observation
The renderer's existing workflowAgentTranscript/workflowAgentConversation methods are backed by Electron IPC and workflowWatcher file reads; createWebBackend currently returns null for both, so browser/remote clients cannot use that watch-pane drill-in without adding a hub capability as well.

## Impact
Codex subagent drill-in can be made provider-correct for local desktop via claudemon, but enabling it over web/remote requires a separate bus method or hub capability instead of only editing preload/ipc.

## Recommendation
When widening subagent monitor support, either keep the UI affordance gated to backends that can serve it or add a bus capability that fetches provider-owned subagent conversation data by session id and child thread id.
