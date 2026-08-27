---
title: Codex child subagent drill-in can replay rollout by thread id
date: 2026-08-26
confidence: high
suggested_doc: claudemon-providers
related_paths:
  - services/claudemon/src/daemon/api.rs
  - services/claudemon/src/providers/codex_rollout.rs
  - apps/desktop/src/main/services/providerSubagentConversation.ts
  - apps/desktop/src/renderer/src/panes/AgentWatchPane.tsx
promoted: false
---

# Codex child subagent drill-in can replay rollout by thread id

## Observation
Codex app-server subagent rows expose child thread ids, and codex_rollout::rollout_for_thread can locate the corresponding durable rollout file directly. A read-only drill-in does not need to send a live app-server RPC as long as the endpoint scopes access through the parent session's known subagents.

## Impact
This enables local desktop Codex subagent watch panes with the existing ConversationItem parser/applier, while avoiding a broader app-server control-channel surface. The remaining web/remote gap is transport exposure, not parsing.

## Recommendation
For Codex subagent inspection, read the child rollout through claudemon and verify the child id appears on the parent SessionState.subagents before serving it. Add a hub capability separately if web/remote clients need the same data.
