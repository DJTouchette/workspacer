import { afterAll, expect, it, vi } from 'vitest';
import fs from 'fs';
import os from 'os';
import path from 'path';
import * as atomic from '../lib/atomicWriteFile';
import type { WorkflowResponse } from '../shared/fleetWorkflow';

const fixture = vi.hoisted(() => ({ root: '' }));
vi.mock('./configService', () => ({
  getConfigDir: () => fixture.root,
  configService: {
    getConfig: () => ({ agents: { defaultWorkflowId: 'scout-implement-review' }, projects: {} }),
  },
}));
vi.mock('./claudeSessionStore', () => ({
  claudeSessionStore: {
    getSnapshot: (sessionId: string) => ({
      sessionId,
      cwd: fixture.root,
      isWakeTarget: true,
      status: 'active',
    }),
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
    expect(started.instructions).toContain('Conditional decision required');
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
    expect(decided.instructions).toContain(`stepId=${run ? 'scout' : 'implement'}`);
    expect(decided.instructions).not.toContain('Conditional decision required');
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

it('prepares one exact step with a conditional decision and returns canonical dispatch metadata', () => {
  const started = start();
  const request = {
    op: 'prepareDispatch' as const,
    taskId: started.task.taskId,
    cwd: fixture.root,
    stepId: 'scout',
    expectedTaskRevision: started.task.revision ?? 0,
    run: true,
    reason: 'Architecture is unresolved',
    templateParams: { task: 'Verify the interface' },
  };
  const prepared = fleetWorkflowRequest(request, 'manager');
  expect(prepared).toMatchObject({
    ok: true,
    dispatch: {
      taskId: request.taskId,
      cwd: fixture.root,
      stepId: 'scout',
      role: 'scout',
      stage: 'scout',
      template: 'scout-task',
      toolScope: 'view',
    },
  });
  if (!prepared.ok || !prepared.dispatch) throw new Error('No dispatch plan');
  expect(prepared.dispatch.expectedTaskRevision).toBeGreaterThan(request.expectedTaskRevision);
  expect(fleetWorkflowRequest(request, 'manager')).toMatchObject({ ok: false, code: 'conflict' });
  expect(dispatchHistoryStore.task(request.taskId)!.attempts).toEqual([]);
  expect(
    fleetWorkflowRequest(
      { ...request, run: undefined, expectedTaskRevision: prepared.dispatch.expectedTaskRevision },
      'other',
    ),
  ).toMatchObject({ ok: false });
});

it('records a conditional skip without preparing the next worker, and refuses required-step skips', () => {
  const started = start();
  const skipped = fleetWorkflowRequest(
    {
      op: 'prepareDispatch',
      taskId: started.task.taskId,
      cwd: fixture.root,
      stepId: 'scout',
      expectedTaskRevision: started.task.revision ?? 0,
      run: false,
      reason: 'Bounded change',
    },
    'manager',
  );
  expect(skipped).toMatchObject({ ok: true, skipped: true });
  expect(skipped).not.toHaveProperty('dispatch');
  const task = dispatchHistoryStore.task(started.task.taskId)!;
  expect(task.workflow!.steps[0].state).toBe('skipped');
  expect(
    fleetWorkflowRequest(
      {
        op: 'prepareDispatch',
        taskId: task.taskId,
        cwd: fixture.root,
        stepId: 'implement',
        expectedTaskRevision: task.revision ?? 0,
        run: false,
        reason: 'Skip implementation',
      },
      'manager',
    ),
  ).toMatchObject({ ok: false, error: 'Required step cannot be skipped' });
  expect(dispatchHistoryStore.task(task.taskId)!.attempts).toEqual([]);
});

it('refuses stale, missing-input and undecided preparations without recording decisions', () => {
  const started = start();
  const base = {
    op: 'prepareDispatch' as const,
    taskId: started.task.taskId,
    cwd: fixture.root,
    stepId: 'scout',
    expectedTaskRevision: started.task.revision ?? 0,
  };
  expect(fleetWorkflowRequest({ ...base, expectedTaskRevision: 999 }, 'manager')).toMatchObject({
    ok: false,
    code: 'conflict',
  });
  expect(fleetWorkflowRequest({ ...base, run: true, reason: 'Risk' }, 'manager')).toMatchObject({
    ok: false,
  });
  expect(dispatchHistoryStore.task(base.taskId)!.workflow!.steps[0].decision).toBeUndefined();
  expect(
    fleetWorkflowRequest({ ...base, templateParams: { task: 'Inspect' } }, 'manager'),
  ).toMatchObject({ ok: false, code: 'ineligible' });
  expect(dispatchHistoryStore.task(base.taskId)!.revision).toBe(started.task.revision);
});

it.each(['cancelled', 'dependency'] as const)(
  'refuses %s tasks before recording a conditional decision',
  (kind) => {
    const started = start();
    dispatchHistoryStore.requestTransaction((_requests, tasks) => {
      const task = tasks.find((t) => t.taskId === started.task.taskId)!;
      if (kind === 'cancelled') task.cancelled = true;
      else task.dependsOn = ['not-accepted'];
    });
    const task = dispatchHistoryStore.task(started.task.taskId)!;
    expect(
      fleetWorkflowRequest(
        {
          op: 'prepareDispatch',
          taskId: task.taskId,
          cwd: fixture.root,
          stepId: 'scout',
          expectedTaskRevision: task.revision,
          run: true,
          reason: 'Inspect risk',
          templateParams: { task: 'Inspect' },
        },
        'manager',
      ),
    ).toMatchObject({ ok: false });
    expect(dispatchHistoryStore.task(task.taskId)!.workflow!.steps[0].decision).toBeUndefined();
    expect(dispatchHistoryStore.task(task.taskId)!.revision).toBe(task.revision);
  },
);

it('returns bounded next actions without hiding a committed resolution when projection fails', async () => {
  start(); // initialize this fixture's isolated history/config root
  const { managerRequests } = await import('./managerRequestService');
  const inbox = managerRequests();
  const captured = inbox.prepare('manager', 'Five independent tasks');
  if (!captured.available) throw new Error('capture unavailable');
  const delivery = inbox.beginDelivery('manager', captured.requestId)!;
  inbox.finishDelivery(captured.requestId, delivery.deliveryId, 'accepted');
  const request = {
    op: 'resolveRequest' as const,
    requestId: captured.requestId,
    expectedRevision: inbox.request('manager', captured.requestId).revision,
    intents: Array.from({ length: 5 }, (_, i) => ({
      key: `action-${i}`,
      kind: 'create' as const,
      cwd: fixture.root,
      title: `Task ${i}`,
      reason: 'Independent work',
      provenance: 'explicit' as const,
    })),
  };
  const projection = vi.spyOn(dispatchHistoryStore, 'task').mockImplementationOnce(() => {
    throw new Error('projection unavailable after commit');
  });
  let response: any;
  try {
    response = fleetWorkflowRequest(request, 'manager');
  } finally {
    projection.mockRestore();
  }
  expect(response).toMatchObject({ ok: true, nextActionsRemaining: 1 });
  expect(response.tasks).toHaveLength(5);
  expect(response.nextActions).toHaveLength(4);
  expect(response.nextActions[0]).toMatchObject({ unavailable: true });
  expect(response.nextActions[1]).toMatchObject({
    cwd: fixture.root,
    revision: expect.any(Number),
    instructions: expect.any(String),
  });
  const replay = fleetWorkflowRequest(request, 'manager') as any;
  expect(replay.ok).toBe(true);
  expect(replay.tasks.map((t: any) => t.taskId)).toEqual(response.tasks.map((t: any) => t.taskId));
});
