/**
 * Regression: workflowAgentIds must mirror the MAX_RUNS-sliced `runs` array.
 * It was built from ALL watch.runs, so agents of runs dropped by the slice were
 * still listed as workflow-owned — keeping them out of the plain subagent list
 * even though their run card is no longer shown (they vanish entirely).
 */
import { describe, it, expect, afterEach } from 'vitest';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import {
  workflowWatcher,
  type WorkflowWatcherUpdate,
} from '../../src/main/services/workflowWatcher';

const made: string[] = [];

afterEach(() => {
  workflowWatcher.detachAll();
  for (const d of made.splice(0)) fs.rmSync(d, { recursive: true, force: true });
});

function makeSession(runCount: number): { sessionDir: string; transcriptPath: string } {
  const sessionDir = fs.mkdtempSync(path.join(os.tmpdir(), 'wfw-'));
  made.push(sessionDir);
  const runsRoot = path.join(sessionDir, 'subagents', 'workflows');
  const finalRoot = path.join(sessionDir, 'workflows');
  fs.mkdirSync(runsRoot, { recursive: true });
  fs.mkdirSync(finalRoot, { recursive: true });

  for (let i = 1; i <= runCount; i++) {
    const runId = `wf_${i}`;
    fs.mkdirSync(path.join(runsRoot, runId), { recursive: true });
    // Final state file → adopted on the first tick, registering one agent.
    fs.writeFileSync(
      path.join(finalRoot, `${runId}.json`),
      JSON.stringify({
        status: 'completed',
        workflowName: runId,
        startTime: 1000 + i, // ascending → wf_1 is the oldest
        durationMs: 10,
        workflowProgress: [{ type: 'workflow_agent', agentId: `agent-a${i}`, state: 'done' }],
      }),
    );
  }
  return { sessionDir, transcriptPath: `${sessionDir}.jsonl` };
}

describe('WorkflowWatcher.buildUpdate — workflowAgentIds vs MAX_RUNS slice', () => {
  it('only lists agents of the runs that survive the MAX_RUNS slice', () => {
    const { transcriptPath } = makeSession(4); // MAX_RUNS is 3, so wf_1 is dropped
    let update: WorkflowWatcherUpdate | undefined;
    workflowWatcher.attach('s1', transcriptPath, (u) => {
      update = u;
    });

    expect(update).toBeDefined();
    // Only 3 runs shown — the oldest (wf_1) dropped.
    expect(update!.runs.map((r) => r.runId).sort()).toEqual(['wf_2', 'wf_3', 'wf_4']);
    // workflowAgentIds must match the shown runs' agents — NOT include a1.
    expect(update!.workflowAgentIds.sort()).toEqual(['a2', 'a3', 'a4']);
    expect(update!.workflowAgentIds).not.toContain('a1');
  });
});
