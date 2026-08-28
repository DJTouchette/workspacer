---
title: Plan/subagent carriers differ per provider: copilot uses SQLite, opencode uses a server-wide SSE stream
date: 2026-08-28
confidence: high
suggested_doc: claudemon-providers
related_paths:
  - services/claudemon/src/providers/copilot.rs
  - services/claudemon/src/providers/opencode.rs
  - apps/desktop/src/renderer/src/lib/providerCaps.ts
promoted: false
---

# Plan/subagent carriers differ per provider: copilot uses SQLite, opencode uses a server-wide SSE stream

## Observation
Verified 2026-08-28 against the installed CLIs.

COPILOT (v1.0.81): the plan is NOT on the wire. `session.todos_changed` has an empty `data` on every occurrence; the todo list lives in a `todos` table in `~/.copilot/session-state/<session-id>/session.db`, which the model mutates through the ordinary `sql` tool (confirmed live — the conversation shows `sql` INSERT/UPDATE statements against `todos`, and the table exists on disk with the CLI's own CHECK constraint `status IN ('pending','in_progress','done','blocked')`). Subagents ARE on the wire: `subagent.started` / `subagent.configured` / `subagent.completed` carry a TOP-LEVEL `agentId` (not inside `data`), plus a `system.notification` with `kind.type == "agent_idle"` as a second close signal. A subagent's OWN frames (its `user.message` dispatch prompt, its tool calls, its whole report) also ride the parent's stdout tagged with that same top-level `agentId` — folding them in renders the child's report as the parent's answer.

OPENCODE (v1.18.25): `GET /event` is a SERVER-wide stream, not a session one, and a subagent is a whole child SESSION on the same server. Events must be filtered by session id or a child's traffic becomes the parent's conversation. Field placement differs per event: `properties.sessionID` (session.*), `properties.info.sessionID` (message.updated), `properties.part.sessionID` (message.part.updated). opencode's own headless client does exactly this filter. Plan carrier = the `todowrite` tool (`{todos:[{content,status}]}`); subagent dispatch = the `task` tool (`{subagent_type, description}`). Tool part states are pending/running/completed/error. NOTE: `properties.id` on a non-session event is NOT a session id (on `permission.updated` it is the permission id).

PI (v0.84.3): built-in tool set is exactly bash, edit, find, grep, ls, powershell, read, write (`dist/core/tools/`). No todo tool, no task tool — no plan and no subagents are possible out of the box.</observation>
<parameter name="impact">Anyone wiring plan/subagent parity for a new provider will look for a wire event and conclude the provider has no plan. Copilot's is in SQLite; opencode's is a tool call on a shared stream. The opencode stream-scoping issue is also a live conversation-correctness bug class, not just a missing feature.

## Recommendation
Check the harness's own persisted state and tool registry before concluding "no signal". For opencode, always filter `/event` by session id at the driver level. Capability flags live in apps/desktop/src/renderer/src/lib/providerCaps.ts `delegation`.
