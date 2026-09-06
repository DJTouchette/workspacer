---
title: Codex child thread model is authoritative over spawn request
date: 2026-09-06
confidence: high
suggested_doc: claudemon-providers
related_paths:
  - services/claudemon/src/providers/codex.rs
  - services/claudemon/src/session/store.rs
  - apps/desktop/src/renderer/src/components/claude/SubagentRow.tsx
promoted: false
---

# Codex child thread model is authoritative over spawn request

## Observation
Codex 0.153.4's generated app-server schema marks collabAgentToolCall.model as a requested model, while thread/started carries Thread.model as the child's configured/latest persisted model. The current adapter copied the requested collab field to SubagentUpdate and dropped Thread.model, so a late collab update could overwrite an effective child model.

## Impact
Inspector model labels must only use Thread.model and partial subagent updates must preserve the last known value.

## Recommendation
Populate SubagentUpdate.model only from the child Thread.model in thread/started; do not use collabAgentToolCall.model for the rendered child model. Keep the store's existing non-null merge behavior and cover parser, store, and Inspector rendering.
