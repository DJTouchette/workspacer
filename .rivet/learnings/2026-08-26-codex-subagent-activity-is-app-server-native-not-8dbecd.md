---
title: Codex subagent activity is app-server native, not Claude artifact based
date: 2026-08-26
confidence: medium
suggested_doc: workflow-subagent-watcher
related_paths:
  - services/claudemon/src/providers/codex.rs
  - apps/desktop/src/main/services/workflowWatcher.ts
  - apps/desktop/src/main/services/claudeSessionStore.ts
  - apps/desktop/src/renderer/src/types/claudeSession.ts
promoted: false
---

# Codex subagent activity is app-server native, not Claude artifact based

## Observation
Codex 0.150.1 app-server schemas expose subagent/collaboration items (`subAgentActivity`, `collabAgentToolCall`) and thread metadata (`parentThreadId`, `agentNickname`, `agentRole`), but the current claudemon Codex provider has no indexed handling for those types. Claude subagent visibility in desktop is mostly built from Claude transcript sidecar files via `workflowWatcher`, so Codex cannot become visible by reusing the Claude file watcher alone.

## Impact
Adding Codex subagent visibility likely needs a claudemon/provider-level event/state surface; looking only at `workflowWatcher` will miss Codex app-server events.

## Recommendation
Model a provider-neutral subagent/activity update in claudemon or map Codex collaboration items into existing session work metadata, then let the desktop consume that instead of expecting Claude-style `subagents/*.jsonl` artifacts.
