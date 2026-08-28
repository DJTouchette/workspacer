---
title: Subagent activity flaps have two roots: stale child rows (false-active) and a too-narrow agent task_type set (false-idle)
date: 2026-08-27
suggested_doc: session-lifecycle
related_paths:
  - services/claudemon/src/session/state.rs
  - services/claudemon/src/session/store.rs
  - services/claudemon/src/providers/claude_stream.rs
  - apps/desktop/src/main/services/sessionStore/hookEventRouter.ts
promoted: false
---

# Subagent activity flaps have two roots: stale child rows (false-active) and a too-narrow agent task_type set (false-idle)

## Observation
Diagnosed 2026-08-27 against a LIVE daemon. Busy/idle is NOT derived from output-stream recency anywhere — hook path (state.rs apply), stream driver (claude_stream.rs) and desktop (hookEventRouter) are all explicit lifecycle-event machines. The flap has two distinct roots, one per direction.

FALSE-ACTIVE (parent/child aggregation). Reproduced live: codex session 4df7fc21 sat at mode:input with background_tasks:1 and one subagents[] row at status:running for 10+ minutes across FOUR further user turns — its own conversation contained "It already completed, so there's nothing left to cancel." SessionStore::apply_subagent_update (added bcef811b) derives background_tasks from the running-row count, but NOTHING ever closed a row whose completion frame never arrived: mark_stopped closes them only at session death. A dead child therefore pinned the parent 'working' forever, which also means the parent never reached ambientState 'idle', so its working→idle edge never fired and a dispatched fleet worker never reported finished. Same class on the desktop: SessionState::apply zeroes live_subagents on SessionStart/UserPromptSubmit, but hookEventRouter did NOT mirror that onto its own session.subagents, and applyStopEvent keeps exactly the running rows while dropping completed ones — so a dropped SubagentStop leaves a phantom that survives forever.

FALSE-IDLE (signal vocabulary, not recency). claude_stream.rs background_tasks_changed held the turn busy only for task_type == "local_agent". The Claude Code 2.1.237 bundle's full task_type vocabulary is local_agent, in_process_teammate, remote_agent, local_bash, local_workflow. in_process_teammate is the teammate/team feature (sits beside leadAgentId / dynamicTeamContext / getConcurrentSubagents) and remote_agent is cloud agents including /code-review ultra (task_remote_agent / task_remote_agent_failed telemetry) — both are the parent waiting on another agent, both classified ambient, so the parent's dispatch `result` idled it mid-subagent.

Also corrected: the claim in the old comment that the CLI's own copy calls local_workflow "ambient/housekeeping" is not in the bundle — near local_workflow it carries "Dynamic workflow", workflow_agent, workflow_phase, agentControllers. local_workflow was left ambient on a weaker, stated rationale (pause/abandon is the same latch shape as a shell, and the desktop covers workflows out of band via workflowWatcher → session.workflows).</observation>
<parameter name="impact">Both directions of "workspacer says an agent is working when it isn't / done when it isn't". The false-active also silently disables the fleet worker-finished wake, because that wake keys off the working→idle ambient edge the stale child prevents.

## Recommendation
Turn boundaries are the reconciliation point for parent/child state: a subagent row is scoped to the tool call that spawned it and cannot outlive its turn. Fixed in SessionState::close_stale_subagents (called from set_managed_mode when mode→Input, only after write_pending accepts, so a parked approval reconciles nothing) and closeStaleSubagents in hookEventRouter (SessionStart/UserPromptSubmit). Closing is the self-healing direction — the provider's next update re-opens the row — which is the same asymmetry that keeps local_bash out of the busy set. When adding a busy-holding signal, enumerate the provider's whole vocabulary from its bundle rather than the one spelling you observed; grep the CLI bundle with `strings` and check what a literal sits beside.</recommendation>
<parameter name="confidence">high
