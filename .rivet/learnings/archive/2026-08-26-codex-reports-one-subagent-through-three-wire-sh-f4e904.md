---
title: Codex reports one subagent through THREE wire shapes; the child thread id is the only join key
date: 2026-08-26
suggested_doc: claudemon-providers
related_paths:
  - services/claudemon/src/providers/codex.rs
  - services/claudemon/src/providers/mod.rs
promoted: true
promoted_to: claudemon-providers
---

# Codex reports one subagent through THREE wire shapes; the child thread id is the only join key

## Observation
As shipped in bcef811b, claudemon's Codex provider builds `AgentUpdate::Subagent` from three unrelated app-server shapes, and every one of them must resolve to the SAME id or the parent grows duplicate rows (`SessionStore::apply_subagent_update` upserts purely by `id`):

1. `thread/started` → `subagent_from_thread`, which reads `thread.id` but ONLY after `parentThreadId` is present. That guard is what stops every top-level thread from registering itself as its own subagent — remove it and each session gains a phantom child.
2. `item/started|completed` type `subAgentActivity` → `agentThreadId` (NOT the item `id`). `kind` is the status: `started`/`interacted` mean Running, anything else means Complete.
3. `item/started|completed` type `collabAgentToolCall` → the KEYS of the `agentsStates` object, falling back to `receiverThreadIds[]` only when that map is absent or empty. `senderThreadId` is the parent and is deliberately never used as a row id.

The dispatch inside `translate_item` is also asymmetric on purpose: `subAgentActivity` `return`s after emitting the Subagent update (activity rows are not tool cards), while `collabAgentToolCall` deliberately FALLS THROUGH so the generic path also emits a `ToolUse`/`ToolResult` pair — a `spawnAgent` renders as the familiar "Agent" tool card AND a subagent row from one wire item. `collab_spawn_agent_yields_tool_card_and_subagent_row` asserts exactly two updates.</observation>
<parameter name="impact">Adding a `return` after the collab branch (the obvious symmetry "fix") silently deletes the Agent tool card. Keying a new handler off `item.id` or `senderThreadId` instead of the child thread id produces a second row for a child that already exists, and the `background_tasks` count derived from that list then over-reports live work.

## Recommendation
Any new Codex subagent wire handler must produce the CHILD thread id as `SubagentUpdate.id` and must decide explicitly whether it also wants a tool card (fall through) or not (return). The `tool_use_id` link back to the card is only set when `tool == "spawnAgent"` — that is what lets the UI tie a row to its originating card.</recommendation>
<parameter name="confidence">high
