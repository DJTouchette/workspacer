---
title: Workflow + subagent artifact watcher and watch panes
tags: [workflow, subagent, filesystem-tail, agent-watch, vm-metadata]
related_paths:
  - "apps/desktop/src/main/services/workflowWatcher.ts"
  - "apps/desktop/src/renderer/src/panes/AgentWatchPane.tsx"
  - "apps/desktop/src/renderer/src/components/claude/WorkflowRunCard.tsx"
  - "apps/desktop/src/renderer/src/components/claude/WorkflowTimeline.tsx"
owner: Damien Touchette
last_reviewed: 2026-09-26
---

# Workflow artifacts and subagent watch panes

## Reader ownership

`apps/desktop/src/main/services/workflowWatcher.ts` reads Claude artifact files
beside a session transcript. It does not communicate with the model. The session
store attaches it when a transcript path becomes known, pokes it on hook activity,
and detaches on teardown. The headless shared-service path can also prime a newly
attached reader with `refresh` before answering its first query.

Attach derives the session directory by removing the .jsonl suffix; an unexpected
path shape is ignored. Polling is every 2.5 seconds, idling after a minute without
a poke when no workflow is live. Missing/unreadable directories generally look
like no artifacts; a reader returning no data does not prove the provider emitted
none. These layouts are provider-version-sensitive, not a stable public API.

## Artifact shapes and adoption

Plain subagent transcripts live beneath the session’s subagents directory.
Workflow runs have their own directories, agent metadata/transcripts and journals;
script metadata and final run JSON live beneath the workflows tree. Keep the
path construction functions as the source of truth when a provider changes layout.

Agent metadata creates known rows before journal started/result entries can
update them. `parseScriptMeta` extracts the exported metadata literal and evaluates
it in a separate node:vm context with a 50 ms timeout, then filters recognized
fields. This is evaluation with limits, not a claimed OS security sandbox or a
full JavaScript parser. Failures fall back to filename-derived metadata.

A successfully parsed final JSON is adopted once and marks the run finalized;
it is not tailed again after that. An unreadable/unparseable final file does not
finalize the run. A valid but premature/partial object can still finalize under
the current permissive reader, so mere final-file presence is not a robust
transactional completion protocol.

Final adoption preserves the cost accumulated from live agent transcripts because
the final artifact does not supply that figure. Label fields arrive from final
progress records and may be absent while running. The live usage fold uses the
last message ID/UUID as its duplicate key; do not treat a single remembered key
as arbitrary replay-order deduplication.

## Projection boundaries

`buildUpdate` emits only the latest three run snapshots. It does not discard the
entire in-memory run map at that limit. `workflowAgentIds` is computed from the
same emitted slice: keeping omitted runs’ agents suppressed would hide them from
both the run cards and the plain-subagent projection.

Workflow phase/agent/run types are mirrored in the renderer’s session types.
Keep additive fields and missing-value behavior aligned. The renderer uses
WorkflowRunCard/WorkflowTimeline and AgentWatchPane; its fleet watch synthesizes
a run shape from plain subagent rows rather than creating a real workflow run.

## Transcript drill-in

`runId === null` selects a plain subagent; a workflow ID selects its known run
directory. Missing watch/run/file returns null. The watcher strips the agent-
filename prefix consistently, so callers must not double-prefix IDs.

File stat/read is asynchronous and parsed views are cached by mtime and size,
with an eight-file insertion-order cap. **Parsing still runs synchronously after
the asynchronous read.** The source comment saying parsing is off the event loop
is stronger than the implementation. A cache hit avoids the repeated parse, but
an asynchronous method alone does not make a large first parse non-blocking.

AgentWatchPane polls a running subagent every 2.5 seconds only while the pane is
active; missing owning-session/run state gets an explanatory empty state. A
successful transcript read is still a projection of recorded artifacts, not proof
that the agent process is currently alive.

Provider-native subagents use the daemon’s provider-neutral state/replay routes.
Codex child IDs join its native thread/rollout data; the Claude artifact watcher
cannot manufacture those records. Keep plain-subagent versus workflow routing
aligned across IPC, shared headless services and web/bus methods, including
`sessions.subagentConversation`. Do not give a new native provider a watch action
until that read path exists or the UI explicitly reports its limitation.

## Verification

From `apps/desktop`:

```bash
npx vitest run tests/main/workflowWatcherAgentIds.test.ts
```

These tests pin attribution/prefix behavior; they are not complete provider-version
coverage. For changes to native child replay, include daemon provider tests and
backend conversation tests. For cache/performance changes, inspect which work
actually leaves the event loop instead of relying on an async return type.
