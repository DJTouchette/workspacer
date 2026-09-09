import { describe, expect, it, vi } from 'vitest';
import type { DispatchTask } from '../shared/dispatchHistory';
import { WORKFLOW_STARTERS } from '../shared/fleetWorkflow';
const fixture = vi.hoisted(() => ({ task: undefined as DispatchTask | undefined }));
vi.mock('./configService', () => ({
  getConfigDir: () => process.env.TMPDIR!,
  configService: { getConfig: () => ({}) },
}));
vi.mock('./claudeSessionStore', () => ({
  claudeSessionStore: { getSnapshot: () => ({ isWakeTarget: true, status: 'active' }) },
}));
vi.mock('./dispatchHistoryStore', () => ({ dispatchHistoryStore: { task: () => fixture.task } }));
import {
  missingWorkflowEvidence,
  workflowInstructions,
  workflowSpawn,
} from './fleetWorkflowRuntime';
function task(): DispatchTask {
  const definition = structuredClone(WORKFLOW_STARTERS[0]);
  return {
    taskId: 'task',
    ownerSessionId: 'manager',
    ownerLabel: 'Manager',
    projectCwd: '/project',
    title: 'Task',
    createdAt: '2026-01-01',
    attempts: [],
    workflow: {
      definition,
      hash: 'pin',
      templates: Object.fromEntries(
        definition.steps.map((s) => [
          s.template,
          {
            id: s.template,
            body: '{{task}}',
            params: [{ name: 'task', required: true }],
            resultSchema: {},
          },
        ]),
      ),
      steps: [
        { id: 'scout', state: 'waived' },
        { id: 'implement', state: 'dispatched' },
        { id: 'review', state: 'waived' },
      ],
    },
  };
}
describe('workflow waiver ordering and evidence', () => {
  it('a future review waiver does not advance past unfinished implementation', async () => {
    const t = task();
    fixture.task = t;
    expect(workflowInstructions(t)).toContain('Step implement is dispatched');
    const spawn = vi.fn();
    await expect(
      workflowSpawn(spawn)({
        taskId: 'task',
        dispatchOwnerSessionId: 'manager',
        cwd: '/project',
        workflowStepId: 'review',
      }),
    ).rejects.toThrow('not eligible');
    expect(spawn).not.toHaveBeenCalled();
    t.workflow!.steps[1].state = 'completed';
    expect(workflowInstructions(t)).toContain('explicit skips');
    expect(workflowInstructions(t)).toContain('never passing results');
  });
  it('rejects missing independent implementation and repair artifacts instead of fabricating results', async () => {
    const t = task();
    fixture.task = t;
    t.workflow!.steps[1].state = 'waived';
    t.workflow!.steps[2].state = 'planned';
    expect(missingWorkflowEvidence(t, 2)).toContain('implement');
    expect(workflowInstructions(t)).toContain('Do not dispatch this step or invent artifacts');
    await expect(
      workflowSpawn(vi.fn())({
        taskId: 'task',
        dispatchOwnerSessionId: 'manager',
        cwd: '/project',
        workflowStepId: 'review',
      }),
    ).rejects.toThrow('Required evidence');
    t.workflow!.definition.steps[2].independentOf = undefined;
    t.workflow!.definition.steps[2].repairOf = 'implement';
    expect(missingWorkflowEvidence(t, 2)).toContain('implement');
  });
  it('requires exact task attempt linkage and a reported result for independent work', () => {
    const t = task();
    t.workflow!.steps[1] = {
      id: 'implement',
      state: 'completed',
      dispatchId: 'd',
      sessionId: 's',
      outcome: { commit: 'actual' },
    };
    expect(missingWorkflowEvidence(t, 2)).toBeTruthy();
    t.attempts.push({
      dispatchId: 'd',
      sessionId: 's',
      workflowStepId: 'implement',
      resultContract: 'valid',
    } as never);
    expect(missingWorkflowEvidence(t, 2)).toBeUndefined();
    t.attempts[0].resultContract = 'invalid';
    expect(missingWorkflowEvidence(t, 2)).toBeTruthy();
  });
});
