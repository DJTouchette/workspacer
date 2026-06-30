/**
 * Bridges workspacer's live workflow telemetry onto the hub event bus, so the
 * rules engine (and any plugin) can react to it. The `workflowWatcher` produces
 * rich per-run / per-agent state, but it only ever reached the renderer — this
 * module republishes the *transitions* onto the bus:
 *
 *   workflow.started          a run is first seen running
 *   workflow.completed        a run finishes ok
 *   workflow.failed           a run fails
 *   workflow.agent.finished   one agent in a run reaches done|failed
 *
 * We deliberately publish only on state transitions, never on every 1s watcher
 * tick, so the bus stays quiet. Agent attention (needs-approval / question /
 * input / Stop) is already published by the Go claudemon bridge as
 * `agent.state_changed`, and cost/budget is covered by the rules engine's
 * `agents.list` poll loop — so neither is duplicated here.
 */
import { publishToHub } from './hubClient';
import { isRemoteShareEnabled } from './hubDaemon';
import type { WorkflowRunInfo } from './workflowWatcher';
import type { ClaudeSessionSnapshot } from './claudeSessionStore';

/**
 * Publish a full session snapshot onto the bus as `agent.snapshot`, so the web
 * build's renderer (which has no Electron IPC) gets the same rich per-session
 * state — transcript, tool calls, fleet/workflow detail — that the desktop gets
 * over `claude-session:update`. Gated on remote sharing: when it's off there is
 * no web consumer, so we skip the extra serialization entirely and the
 * desktop-only path is unchanged.
 */
export function publishSnapshot(snapshot: ClaudeSessionSnapshot): void {
  if (!isRemoteShareEnabled()) return;
  publishToHub({ type: 'agent.snapshot', data: snapshot });
}

interface SessionMeta {
  sessionId: string;
  cwd: string;
}

// Transition memory so the same fact is published at most once.
const runStatus = new Map<string, WorkflowRunInfo['status']>(); // runId -> last status published
const agentStatus = new Map<string, 'done' | 'failed'>();        // `${runId}:${agentId}` -> last terminal status
const sessionRuns = new Map<string, Set<string>>();              // sessionId -> runIds seen (for cleanup)

function trackRun(sessionId: string, runId: string): void {
  let set = sessionRuns.get(sessionId);
  if (!set) { set = new Set(); sessionRuns.set(sessionId, set); }
  set.add(runId);
}

/**
 * Diff the latest workflow snapshot against what we've already published and
 * emit a bus event for each new transition. Safe to call on every merge — it is
 * idempotent and only publishes when something actually changed.
 */
export function publishWorkflowRuns(meta: SessionMeta, runs: WorkflowRunInfo[]): void {
  for (const run of runs) {
    const prev = runStatus.get(run.runId);
    if (prev !== run.status) {
      runStatus.set(run.runId, run.status);
      trackRun(meta.sessionId, run.runId);
      if (run.status === 'running' && prev === undefined) {
        publishToHub({
          type: 'workflow.started',
          data: {
            sessionId: meta.sessionId,
            cwd: meta.cwd,
            runId: run.runId,
            name: run.name,
            description: run.description,
            phases: run.phases.length,
            agents: run.agents.length,
            startedAt: run.startedAt,
          },
        });
      } else if (run.status === 'completed' || run.status === 'failed') {
        publishToHub({
          type: `workflow.${run.status}`,
          data: {
            sessionId: meta.sessionId,
            cwd: meta.cwd,
            runId: run.runId,
            name: run.name,
            status: run.status,
            durationMs: run.durationMs,
            totalTokens: run.totalTokens,
            totalToolCalls: run.totalToolCalls,
            agents: run.agents.length,
          },
        });
      }
    }

    // Per-agent terminal transitions (done / failed), published once each.
    for (const a of run.agents) {
      if (a.status !== 'done' && a.status !== 'failed') continue;
      const key = `${run.runId}:${a.id}`;
      if (agentStatus.get(key) === a.status) continue;
      agentStatus.set(key, a.status);
      publishToHub({
        type: 'workflow.agent.finished',
        data: {
          sessionId: meta.sessionId,
          cwd: meta.cwd,
          runId: run.runId,
          agentId: a.id,
          label: a.label,
          model: a.model,
          status: a.status,
          durationMs: a.durationMs,
          tokens: a.tokens,
          toolCalls: a.toolCalls,
          phaseTitle: a.phaseTitle,
        },
      });
    }
  }
}

/** Drop a session's transition memory when it ends, to bound memory. */
export function forgetSession(sessionId: string): void {
  const runIds = sessionRuns.get(sessionId);
  if (runIds) {
    for (const runId of runIds) {
      runStatus.delete(runId);
      for (const key of Array.from(agentStatus.keys())) {
        if (key.startsWith(`${runId}:`)) agentStatus.delete(key);
      }
    }
    sessionRuns.delete(sessionId);
  }
}
