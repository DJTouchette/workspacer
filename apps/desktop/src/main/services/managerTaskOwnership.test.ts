import { afterEach, describe, expect, it } from 'vitest';
import fs from 'fs';
import os from 'os';
import path from 'path';
import { DispatchHistoryStore } from './dispatchHistoryStore';
import { FleetWorkflowStore } from './fleetWorkflowStore';
import { WORKFLOW_STARTERS, type WorkflowTemplate } from '../shared/fleetWorkflow';

const dirs: string[] = [];
afterEach(() => {
  for (const dir of dirs.splice(0)) fs.rmSync(dir, { recursive: true, force: true });
});

function fixture() {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'manager-ownership-'));
  dirs.push(dir);
  const filename = path.join(dir, 'history.json');
  const history = new DispatchHistoryStore(() => filename);
  const templates: WorkflowTemplate[] = ['ship-task', 'scout-task', 'review-task'].map((id) => ({
    id,
    body: '{{task}}',
    resultSchema: {
      type: 'object',
      required: ['outcome'],
      properties: { outcome: { type: 'string' } },
    },
    params: [{ name: 'task', required: true }],
  }));
  const definitions = new FleetWorkflowStore(
    () => path.join(dir, 'definitions.json'),
    () => templates,
    () => [],
  );
  const owner = {
    sessionId: 'source',
    label: 'Fleet Manager',
    isWakeTarget: true,
    status: 'active',
  };
  const admission = { owner, projectCwd: dir, executionCwd: dir };
  const standalone = history.accept({
    ...admission,
    sessionId: 'ordinary',
    title: 'Original instructions',
    stage: 'implement',
  })!;
  history.accept({ ...admission, sessionId: 'retry', retrySourceSessionId: 'ordinary' });
  const workflow = history.startWorkflow(
    owner,
    dir,
    'Pinned workflow',
    definitions.pin(WORKFLOW_STARTERS[0]),
  );
  history.workflowDecision(workflow.taskId, 'scout', false, 'No unresolved material risk');
  history.accept({
    ...admission,
    sessionId: 'workflow-worker',
    taskId: workflow.taskId,
    workflowStepId: 'implement',
    stage: 'implement',
  });
  history.validated('workflow-worker', 'valid', 'review-evidence', { outcome: 'Implemented' });
  const unrelated = history.accept({
    ...admission,
    owner: { ...owner, sessionId: 'unrelated' },
    sessionId: 'other-worker',
  })!;
  history.flush();
  return { filename, history, admission, standalone, workflow, unrelated };
}

describe('manager task ownership transfer', () => {
  it('refuses adoption under the same file lock while allocation is reserved, then preserves the accepted attempt and revisions', () => {
    const { history, filename, admission, workflow, standalone } = fixture();
    const reserved = history.reserveWorkflowDispatch(
      workflow.taskId,
      history.task(workflow.taskId)!.revision!,
      'review',
    );
    const before = fs.readFileSync(filename, 'utf8');
    // A second store models the manager adoption racing the allocation host.
    const adopter = new DispatchHistoryStore(() => filename);
    expect(() => adopter.adoptWorkflowTasks('source', 'successor')).toThrow('reservation');
    expect(fs.readFileSync(filename, 'utf8')).toBe(before);
    expect(adopter.task(standalone.taskId)?.ownerSessionId).toBe('source');
    const accepted = history.accept({
      ...admission,
      sessionId: 'late-review',
      taskId: workflow.taskId,
      workflowStepId: 'review',
      stage: 'review',
    })!;
    // Acceptance alone is insufficient: the async host must release its token.
    expect(() => adopter.adoptWorkflowTasks('source', 'successor')).toThrow('reservation');
    history.releaseWorkflowDispatch(workflow.taskId, reserved);
    const settled = history.task(workflow.taskId)!;
    adopter.adoptWorkflowTasks('source', 'successor');
    const transferred = history.task(workflow.taskId)!;
    expect(transferred.ownerSessionId).toBe('successor');
    expect(transferred.revision).toBe(settled.revision! + 1);
    expect(transferred.attempts).toEqual(settled.attempts);
    expect(transferred.workflow).toEqual(settled.workflow);
    expect(transferred.attempts.at(-1)?.dispatchId).toBe(accepted.dispatchId);
    expect(transferred.workflow?.steps.find((s) => s.id === 'review')).toMatchObject({
      state: 'dispatched',
      sessionId: 'late-review',
    });
  });
  it('transfers ordinary and workflow records without rebuilding attempts, pinned policy or history', () => {
    const { history, standalone, workflow, unrelated } = fixture();
    const before = [standalone, workflow].map(({ taskId }) => history.task(taskId)!);
    const other = history.task(unrelated.taskId);
    history.adoptWorkflowTasks('source', 'successor');
    for (const task of before) {
      // Task Inspector owns revision/audit changes. Pin the records succession
      // must preserve without forbidding its additive mutation bookkeeping.
      expect(history.task(task.taskId)).toMatchObject({
        taskId: task.taskId,
        projectCwd: task.projectCwd,
        title: task.title,
        createdAt: task.createdAt,
        attempts: task.attempts,
        ownerSessionId: 'successor',
        ownerLabel: 'successor',
      });
      expect(history.task(task.taskId)?.workflow).toEqual(task.workflow);
    }
    expect(history.task(unrelated.taskId)).toEqual(other);
  });

  it('persists both ownership changes and makes a duplicate adoption a no-op', () => {
    const { history, filename, standalone, workflow } = fixture();
    history.adoptWorkflowTasks('source', 'successor');
    const bytes = fs.readFileSync(filename, 'utf8');
    history.adoptWorkflowTasks('source', 'successor');
    expect(fs.readFileSync(filename, 'utf8')).toBe(bytes);
    const reopened = new DispatchHistoryStore(() => filename);
    for (const taskId of [standalone.taskId, workflow.taskId]) {
      expect(reopened.task(taskId)?.ownerSessionId).toBe('successor');
    }
    expect(reopened.task(workflow.taskId)?.workflow).toEqual(
      history.task(workflow.taskId)?.workflow,
    );
  });

  it('admits continuation under the successor only, retaining ordinary task and predecessor IDs', () => {
    const { history, admission, standalone } = fixture();
    history.adoptWorkflowTasks('source', 'successor');
    const next = {
      ...admission,
      taskId: standalone.taskId,
      afterDispatchId: standalone.dispatchId,
      stage: 'review' as const,
    };
    expect(() => history.validate(next)).toThrow();
    const accepted = history.accept({
      ...next,
      owner: { ...admission.owner, sessionId: 'successor' },
      sessionId: 'reviewer',
    })!;
    expect(accepted.taskId).toBe(standalone.taskId);
    expect(history.task(standalone.taskId)?.attempts.at(-1)?.afterDispatchId).toBe(
      standalone.dispatchId,
    );
  });
});
