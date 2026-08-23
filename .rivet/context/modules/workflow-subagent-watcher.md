---
title: Workflow + subagent artifact watcher and watch panes
tags: [workflow, subagent, filesystem-tail, agent-watch, vm-sandbox]
related_paths:
  - "apps/desktop/src/main/services/workflowWatcher.ts"
  - "apps/desktop/src/renderer/src/panes/AgentWatchPane.tsx"
  - "apps/desktop/src/renderer/src/components/claude/WorkflowRunCard.tsx"
  - "apps/desktop/src/renderer/src/components/claude/WorkflowTimeline.tsx"
owner: Damien Touchette
last_reviewed: 2026-07-11
---

# Workflow + subagent artifact watcher and watch panes

## Overview
`workflowWatcher.ts` is a pure on-disk tailer — no Claude comms — that polls Claude Code's subagent/workflow artifact files beside a session's transcript to surface live progress (running agents, tool activity, tokens, cost) before Claude Code writes any final summary. `claudeSessionStore.ts` attaches/pokes/detaches it per session and merges its snapshots into `session.subagents`/`session.workflows`; the renderer reads that via `useClaudeSession` and drills into individual agents with `AgentWatchPane.tsx`, `WorkflowTimeline.tsx`, and `WorkflowRunCard.tsx`.

## Key modules
- `apps/desktop/src/main/services/workflowWatcher.ts` — the `WorkflowWatcher` class: per-session poll loop (`TICK_MS=2500`, idles after `IDLE_AFTER_MS=60_000` unless a run is live), tails run/agent/journal files, adopts the final-state JSON, exposes `readAgentTranscript`/`readAgentConversation` for drill-in.
- `apps/desktop/src/main/services/modelUsage.ts` — `turnCostUSD(model, usage)`, called per assistant turn in `applyTranscriptEntry` to accumulate live `costUSD`.
- `apps/desktop/src/main/services/claudeSessionStore.ts` — wires the watcher: `workflowWatcher.attach(sessionId, session.transcriptPath, cb)` on first hook event carrying `transcript_path` (~line 425), `workflowWatcher.poke(sessionId)` every hook event (~line 430), `workflowWatcher.detach(sessionId)` on `SessionEnd` (~line 470); `applyWatcherUpdate`/`mergeWatcherData` (~line 750) merge `update.runs` into `session.workflows` and filter workflow-owned agents out of `session.subagents`.
- `apps/desktop/src/main/ipc.ts` — `IPC.WORKFLOW_AGENT_TRANSCRIPT` and `IPC.WORKFLOW_AGENT_CONVERSATION` handlers (~line 567-579) proxy straight to the watcher's read methods.
- `apps/desktop/src/renderer/src/types/claudeSession.ts` — hand-mirrored copies of `WorkflowPhaseInfo`, `WorkflowAgentInfo`, `WorkflowRunInfo` (comment explicitly says "mirrors src/main/services/workflowWatcher.ts", ~line 80-120).
- `apps/desktop/src/renderer/src/panes/AgentWatchPane.tsx` — pane for `watchKind: 'subagent' | 'workflow' | 'agents'`; polls `window.electronAPI.workflowAgentTranscript`/`workflowAgentConversation` every `TRANSCRIPT_POLL_MS=2500` while the agent runs; `'agents'` mode synthesizes a fake `WorkflowRunInfo` (`runId: fleet:<sessionId>`) from `session.subagents` so the same `WorkflowTimeline` renders plain Agent-tool calls.
- `apps/desktop/src/renderer/src/components/claude/WorkflowTimeline.tsx` / `WorkflowRunCard.tsx` — render `WorkflowRunInfo` as swimlane/time-bar cards; consume the same mirrored types.
- `apps/desktop/tests/main/workflowWatcherAgentIds.test.ts` — existing test coverage for agent-id stripping/attribution.
- `apps/desktop/scripts/test-workflow-watcher.js` — standalone manual exerciser script for the watcher.

## Failure modes
- All fs reads are wrapped in try/catch that swallow to empty/null (`tailLines`, `readJsonSafe`, `readdirSync` in `scanRuns`/`scanPlainAgents`) — a missing dir just means "no workflows yet," not an error surfaced anywhere.
- `tick()` catches and `console.error`s any throw from `scanRuns`/`scanPlainAgents` so one bad tick never kills the interval.
- `parseScriptMeta` brace-matches the `export const meta = {...}` literal by hand and evaluates it in a `node:vm` sandboxed context (`vm.runInNewContext`, 50ms timeout, empty global object) — any parse/eval failure falls back to a filename-derived name; this is inherently fragile if Claude Code changes script formatting.
- `tryAdoptFinal` treats the presence of `workflows/wf_<runId>.json` as the single source of truth: once adopted, `run.finalized = true` and the run is never tailed again, even if the file is later rewritten or was written mid-race.
- Duplicate usage entries are deduped per-agent via `lastUsageKey` (assistant `msg.id` or `entry.uuid`) to avoid double-counting tokens/cost when Claude Code re-emits partial transcript chunks.
- `readAgentTranscript`/`readAgentConversation` resolve the run dir only from `watch.runs` (in-memory) — if the session was never attached or the run aged out of `MAX_RUNS=3`, drill-in silently returns `null` and the pane shows "Transcript unavailable."
- `AgentWatchPane` shows an explicit "owning session isn't being watched" message when `session` is falsy or the run/subagent isn't in the live snapshot — the pane is read-only and has no fallback to re-attach.

## Gotchas
- The `WorkflowAgentInfo`/`WorkflowRunInfo`/`WorkflowPhaseInfo` types are hand-duplicated (not shared/generated) between `apps/desktop/src/main/services/workflowWatcher.ts` and `apps/desktop/src/renderer/src/types/claudeSession.ts` — any field added on one side must be manually mirrored or the renderer silently drops it.
- `costUSD` is a live-tail-only figure: the final `wf_<runId>.json` file "has no cost figures" (see comment at `tryAdoptFinal`, ~line 670), so on adoption the code explicitly preserves `run.agents.get(...)?.info.costUSD` from the pre-adoption live tail rather than trusting the final file — losing the live `RunState.agents` map before adoption would zero out cost permanently for that run.
- `label` on `WorkflowAgentInfo` is only ever populated from the final-state file (`p.label`) — it is never set during live tailing, so UI relying on `label` must handle it being undefined for the entire "running" phase of a workflow.
- The on-disk contract (`subagents/agent-<id>.jsonl[.meta.json]`, `subagents/workflows/wf_<runId>/...`, `workflows/scripts/<name>-wf_<id>.js`, `workflows/wf_<runId>.json`) is undocumented/reverse-engineered from Claude Code's own behavior, not a published API — any Claude Code update could silently break tailing with only the "no workflows yet" no-op fallback (no visible error).
- `sessionDir` is derived by stripping the `.jsonl` suffix off `transcriptPath` (`workflowWatcher.attach`) — `attach()` no-ops if that strip doesn't change the string ("unexpected path shape"), so any transcript-path format change upstream silently disables the whole watcher for that session.
- `MAX_RUNS=3` and the `buildUpdate` comment note that dropped runs' `workflowAgentIds` must still be excluded from `subagentActivity`, otherwise agents from aged-out runs would reappear in the plain-subagent list — this coupling between `runs` slicing and `workflowAgentIds` computation is easy to break independently.
- `journal.jsonl` `started`/`result` entries only update agents already registered from a `meta.json` sighting (`refreshJournal` looks up `run.agents.get(id)` and no-ops if absent) — ordering matters: meta.json must be seen before journal entries are meaningful.
