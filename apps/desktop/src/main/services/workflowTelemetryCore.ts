import type { WorkflowRunInfo } from './workflowWatcher';
export function createWorkflowTelemetry(publishToHub:(event:{type:string;data:unknown})=>void) {
interface SessionMeta {
  sessionId: string;
  cwd: string;
}

// Transition memory so the same fact is published at most once.
const runStatus = new Map<string, WorkflowRunInfo['status']>(); // runId -> last status published
const agentStatus = new Map<string, 'done' | 'failed'>(); // `${runId}:${agentId}` -> last terminal status
const sessionRuns = new Map<string, Set<string>>(); // sessionId -> runIds seen (for cleanup)

function trackRun(sessionId: string, runId: string): void {
  let set = sessionRuns.get(sessionId);
  if (!set) {
    set = new Set();
    sessionRuns.set(sessionId, set);
  }
  set.add(runId);
}

/**
 * Diff the latest workflow snapshot against what we've already published and
 * emit a bus event for each new transition. Safe to call on every merge — it is
 * idempotent and only publishes when something actually changed.
 */
function publishWorkflowRuns(meta: SessionMeta, runs: WorkflowRunInfo[]): void {
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
function forgetSession(sessionId: string): void {
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

return {publishWorkflowRuns,forgetSession};
}
