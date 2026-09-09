import { afterAll, expect, it, vi } from 'vitest';
import fs from 'fs';
import os from 'os';
import path from 'path';
import * as atomic from '../lib/atomicWriteFile';
import type { WorkflowResponse } from '../shared/fleetWorkflow';

const fixture = vi.hoisted(() => ({ root: '' }));
vi.mock('./configService', () => ({
  getConfigDir: () => fixture.root,
  configService: { getConfig: () => ({ agents: {}, projects: {} }) },
}));
vi.mock('./claudeSessionStore', () => ({
  claudeSessionStore: {
    getSnapshot: (sessionId: string) => ({ sessionId, isWakeTarget: true, status: 'active' }),
  },
}));
vi.mock('./libraryService', () => ({
  libraryService: {
    list: () =>
      ['ship-task', 'scout-task', 'review-task'].map((id) => ({
        id,
        kind: 'dispatch',
        scope: 'global',
        body: '{{task}}',
        resultSchema: {
          type: 'object',
          required: ['outcome'],
          properties: { outcome: { type: 'string' } },
        },
      })),
  },
}));

import { fleetWorkflowRequest } from './fleetWorkflowService';
import { dispatchHistoryStore, DispatchHistoryStore } from './dispatchHistoryStore';
import { workflowInstructions } from './fleetWorkflowRuntime';

function taskResponse(response: WorkflowResponse) {
  if (!response.ok || !response.task) throw new Error(JSON.stringify(response));
  return { ...response, task: response.task };
}
function start() {
  fixture.root ||= fs.mkdtempSync(path.join(os.tmpdir(), 'workflow-service-'));
  return taskResponse(
    fleetWorkflowRequest({ op: 'start', cwd: fixture.root, title: 'Task' }, 'manager'),
  );
}
afterAll(() => {
  if (fixture.root) fs.rmSync(fixture.root, { recursive: true, force: true });
});

it.each([false, true])(
  'returns committed decision, revision and next instructions for run=%s',
  (run) => {
    const started = start();
    const taskId = started.task.taskId;
    expect(started.task).toEqual(dispatchHistoryStore.task(taskId));
    expect(started.instructions).toContain('Call decide_workflow_step');
    const decided = taskResponse(
      fleetWorkflowRequest(
        {
          op: 'decide',
          cwd: fixture.root,
          taskId,
          stepId: 'scout',
          run,
          reason: 'Explicit risk decision',
        },
        'manager',
      ),
    );
    const persisted = new DispatchHistoryStore(() =>
      path.join(fixture.root, 'dispatch-history.json'),
    ).task(taskId)!;
    expect(decided.task).toEqual(persisted);
    expect(decided.task.revision).toBe(started.task.revision! + 1);
    expect(decided.task.workflow!.steps[0]).toMatchObject({
      state: run ? 'planned' : 'skipped',
      decision: run,
      reason: 'Explicit risk decision',
    });
    expect(decided.instructions).toBe(workflowInstructions(persisted));
    expect(decided.instructions).toContain(`workflowStepId=${run ? 'scout' : 'implement'}`);
    expect(decided.instructions).not.toContain('Call decide_workflow_step');
    expect(fleetWorkflowRequest({ op: 'next', cwd: fixture.root, taskId }, 'manager')).toEqual(
      decided,
    );
    // The original response is a snapshot, not a live reference into the store.
    expect(started.task.workflow!.steps[0].decision).toBeUndefined();
  },
);

it('does not report a committed decision or next instructions after persistence failure', () => {
  const started = start();
  const filename = path.join(fixture.root, 'dispatch-history.json');
  const before = fs.readFileSync(filename, 'utf8');
  const write = vi.spyOn(atomic, 'atomicWriteFileSync').mockImplementationOnce(() => {
    throw new Error('Fixture persistence failure');
  });
  try {
    expect(
      fleetWorkflowRequest(
        {
          op: 'decide',
          cwd: fixture.root,
          taskId: started.task.taskId,
          stepId: 'scout',
          run: false,
          reason: 'Explicit risk decision',
        },
        'manager',
      ),
    ).toEqual({ ok: false, code: 'unavailable', error: 'Fixture persistence failure' });
    expect(fs.readFileSync(filename, 'utf8')).toBe(before);
    expect(dispatchHistoryStore.task(started.task.taskId)).toEqual(started.task);
  } finally {
    write.mockRestore();
  }
});
