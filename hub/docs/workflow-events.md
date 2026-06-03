# Workflow telemetry on the bus

workspacer's `workflowWatcher` tails Claude Code's on-disk workflow artifacts and
produces live per-run / per-agent state. That state is now also republished onto
the hub event bus (source `workspacer`), so the rules engine — or any plugin that
subscribes to `workflow.*` — can react to it.

Events are published **only on transitions**, never on every watcher tick, so the
bus stays quiet during a long run.

## Event types

| Type | When | Notable `data` fields |
| --- | --- | --- |
| `workflow.started` | a run is first seen running | `runId`, `name`, `description`, `phases`, `agents`, `cwd`, `sessionId`, `startedAt` |
| `workflow.completed` | a run finishes ok | `runId`, `name`, `durationMs`, `totalTokens`, `totalToolCalls`, `agents`, `cwd` |
| `workflow.failed` | a run fails | same shape as `workflow.completed` |
| `workflow.agent.finished` | one agent in a run reaches `done` or `failed` | `runId`, `agentId`, `label`, `model`, `status`, `durationMs`, `tokens`, `toolCalls`, `phaseTitle`, `cwd` |

All payloads carry `sessionId` and `cwd` so rules can route a notification or
`focus_agent` command back to the originating agent.

## What is *not* here (and why)

- **Agent attention** (needs-approval / question / input / Stop) is already on the
  bus as `agent.state_changed`, published by the Go claudemon bridge. Match those
  events instead.
- **Cost / budget** is covered by the rules engine's `agents.list` poll loop, which
  synthesizes `agents.poll` events carrying `costUSD`. See the `over-budget` rule.

## Example rules

The bundled `rules.json` ships these disabled by default — flip `enabled` to turn
them on:

```json
{
  "id": "workflow-completed",
  "name": "Notify when a workflow run finishes",
  "enabled": true,
  "when": { "event": "workflow.completed" },
  "do": [
    { "type": "notify", "title": "Workflow done: {{data.name}}", "body": "{{data.totalTokens}} tokens in {{data.cwd}}" }
  ]
}
```

```json
{
  "id": "workflow-agent-failed",
  "name": "Alert when an agent inside a workflow fails",
  "enabled": true,
  "when": { "event": "workflow.agent.finished", "match": { "status": "failed" } },
  "do": [
    { "type": "notify", "title": "Agent failed: {{data.label}}", "body": "{{data.phaseTitle}} — {{data.cwd}}" }
  ],
  "cooldownMs": 2000
}
```

## Wiring (for reference)

```
workflowWatcher (fs poll)
  → claudeSessionStore.mergeWatcherData()
    → hubTelemetry.publishWorkflowRuns()   [transition diff]
      → hubClient.publishToHub()           → ws://127.0.0.1:<port>/bus
        → broker.Publish()                 → rules-engine (subscribes "workflow.*")
```

Source: `src/main/services/hubTelemetry.ts`, emitted from
`src/main/services/claudeSessionStore.ts`.
