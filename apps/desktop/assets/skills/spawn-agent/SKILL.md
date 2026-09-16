---
name: spawn-agent
description: Spawn a bounded child coding agent for independent or parallel work, then handle its progress, blocker, and completion wakes. Use when a task has a substantial separable subtask; handle quick work directly.
---

# Spawn an agent

Use `spawn_agent` with `trackTask:false`; ordinary agents do not own Fleet
Manager tasks or workflows. The host derives your parent identity, so do not
invent or copy a `parentSessionId`.

Give the child one self-contained first message: objective, acceptance criteria,
verified files or findings, constraints, expected checks, and a concise delivery
summary. Set `cwd` to the project that owns the work. Use `worktree:true` for
code changes that need isolation; use the current checkout for read-only work or
when the user explicitly wants shared edits.

Honor an explicit provider or model choice. Pass an explicitly named model with
its provider and `exactModel:true`; otherwise let the configured default apply
or use `select_model` when role-based routing materially helps. Never broaden
the child's task or external authority beyond the user's request.

After spawning, end your turn. Do not poll. Workspacer sends you the child's
progress, blocker/escalation, completion, and missed-wake catch-up events. Reply
with `send_message` when the child needs direction, and validate its reported
outcome before presenting it as complete. These wakes cover only your own direct
children; Fleet Manager broadcasts, task ownership, and handoff remain separate.
