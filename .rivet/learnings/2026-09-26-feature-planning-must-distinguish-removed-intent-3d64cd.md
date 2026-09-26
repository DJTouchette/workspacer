---
title: Feature planning must distinguish removed Intent workspaces from retained Fleet tasks
date: 2026-09-26
confidence: high
suggested_doc: mission-control-attention
related_paths:
  - apps/desktop/src/renderer/src/hooks/useAttentionFeed.ts
  - apps/desktop/src/renderer/src/components/TaskInspector.tsx
promoted: false
---

# Feature planning must distinguish removed Intent workspaces from retained Fleet tasks

## Observation
Commit 5fa419bb explicitly removes first-class Intent workspaces, boards and tracker integrations while preserving Fleet request resolution and task provenance. Current TaskInspector.tsx still presents DispatchTask workflow steps. useAttentionFeed.ts keeps dismissal, snooze and observed completion transitions in renderer state; its comment explicitly reserves durable cross-device queues as a reason to reconsider backend item storage.

## Impact
A new task-board proposal can accidentally recreate a recently removed product surface. Cross-device attention continuity is a separate extension of the existing Fleet and Inbox.

## Recommendation
For a continuity feature, extend existing attention and task evidence surfaces; validate the desired cross-device behavior before introducing another workspace model.
