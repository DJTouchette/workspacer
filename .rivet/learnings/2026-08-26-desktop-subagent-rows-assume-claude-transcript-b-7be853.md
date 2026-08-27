---
title: Desktop subagent rows assume Claude transcript-backed watch panes
date: 2026-08-26
confidence: high
suggested_doc: workflow-subagent-watcher
related_paths:
  - apps/desktop/src/renderer/src/components/claude/InspectorCard.tsx
  - apps/desktop/src/main/ipc.ts
  - apps/desktop/src/main/services/workflowWatcher.ts
promoted: false
---

# Desktop subagent rows assume Claude transcript-backed watch panes

## Observation
The renderer's Agents tab renders every `session.subagents[]` row with a click target that calls `requestAgentWatch(kind: 'subagent')`; the main-process handler for that watch pane reads from `workflowWatcher`/Claude sidecar transcript files. Codex app-server subagent rows would be visible if added to `subagents[]`, but their watch pane would be empty unless a Codex thread-read path is added or the click is gated.

## Impact
A minimal Codex subagent v1 should either hide/disable row watch actions for Codex rows or add a separate app-server/thread transcript reader; otherwise the UI advertises drill-in that cannot work.

## Recommendation
For the first provider-neutral subagent snapshot path, populate the Agents tab but gate monitor/watch affordances to providers with transcript-backed watcher data. Add Codex thread-read drill-in as a follow-up.
