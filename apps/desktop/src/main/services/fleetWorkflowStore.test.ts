import { describe, it, expect, vi, afterEach } from 'vitest';
import fs from 'fs';
import os from 'os';
import path from 'path';
import { FleetWorkflowStore, WorkflowConflict } from './fleetWorkflowStore';
import {
  WORKFLOW_STARTERS,
  validateWorkflow,
  type WorkflowTemplate,
} from '../shared/fleetWorkflow';
vi.mock('electron', () => ({ app: { getPath: () => os.tmpdir() } }));
import { DispatchHistoryStore } from './dispatchHistoryStore';
const dirs: string[] = [];
afterEach(() => {
  for (const d of dirs) fs.rmSync(d, { recursive: true, force: true });
  dirs.length = 0;
});
function setup() {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'workflow-test-'));
  dirs.push(dir);
  const filename = path.join(dir, 'workflow-definitions.json');
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
  const store = new FleetWorkflowStore(
    () => filename,
    () => templates,
    () => ['scout-implement-review'],
  );
  return { dir, filename, templates, store };
}
const custom = () => ({
  ...structuredClone(WORKFLOW_STARTERS[0]),
  id: 'custom',
  name: 'My policy',
});
describe('Fleet definition store and pinned history', () => {
  it('seeds immutable starters additively, rejects stale CAS without touching bytes, preserves edits on reopen', () => {
    const { store, filename, templates } = setup();
    expect(store.list()).toHaveLength(WORKFLOW_STARTERS.length);
    store.mutate('create', undefined, undefined, custom());
    const d = store.mutate('update', 'custom', 1, { ...custom(), name: 'Edited' })!;
    expect(d.revision).toBe(2);
    const bytes = fs.readFileSync(filename, 'utf8');
    expect(() => store.mutate('update', 'custom', 1, custom())).toThrow(WorkflowConflict);
    expect(fs.readFileSync(filename, 'utf8')).toBe(bytes);
    expect(
      new FleetWorkflowStore(
        () => filename,
        () => templates,
        () => [],
      )
        .list()
        .find((d) => d.id === 'custom')?.name,
    ).toBe('Edited');
    expect(() => store.mutate('update', WORKFLOW_STARTERS[0].id, 1, custom())).toThrow('immutable');
    expect(() => store.mutate('delete', WORKFLOW_STARTERS[0].id, 1)).toThrow();
  });
  it('accepts research-only/order customization; rejects privilege keys, arbitrary stages/conditions and unbounded repair links', () => {
    const { store } = setup();
    const d = custom();
    d.steps = [{ ...d.steps[0], when: 'always' }];
    for (const id of [true, false, 1, null]) {
      expect(() => validateWorkflow({ ...d, id })).toThrow();
      expect(() => validateWorkflow({ ...d, steps: [{ ...d.steps[0], id }] })).toThrow();
    }
    expect(store.validate(d).steps).toHaveLength(1);
    for (const key of [
      'toolScope',
      'permissions',
      'provider',
      'model',
      'capability',
      'worktree',
      'script',
    ])
      expect(() => store.validate({ ...d, steps: [{ ...d.steps[0], [key]: 'operator' }] })).toThrow(
        'Unknown workflow field',
      );
    expect(() => validateWorkflow({ ...d, steps: [{ ...d.steps[0], when: 'eval' }] })).toThrow();
    expect(() => validateWorkflow({ ...d, steps: [{ ...d.steps[0], stage: 'custom' }] })).toThrow();
    const repair = {
      ...WORKFLOW_STARTERS[1].steps[0],
      id: 'repair',
      kind: 'repair' as const,
      repairOf: 'review',
    };
    expect(() =>
      validateWorkflow({
        ...custom(),
        steps: [...custom().steps, repair, { ...repair, id: 'repair-twice' }],
      }),
    ).toThrow('at most once');
  });
  it('pins actual template bytes/contracts and step decisions across edits, disable, delete and restart; no idle completion', () => {
    const { store, dir, templates } = setup();
    const d = store.mutate('create', undefined, undefined, custom())!;
    const history = new DispatchHistoryStore(() => path.join(dir, 'history.json'));
    const owner = { sessionId: 'manager', isWakeTarget: true, status: 'active' };
    const task = history.startWorkflow(owner, dir, 'task', store.pin(d));
    history.workflowDecision(task.taskId, 'scout', false, 'No unresolved material risk');
    const accepted = history.accept({
      owner,
      projectCwd: dir,
      executionCwd: dir,
      taskId: task.taskId,
      workflowStepId: 'implement',
      stage: 'implement',
      sessionId: 'worker',
    })!;
    history.observe({ sessionId: 'worker', status: 'active', ambientState: 'idle' } as never);
    expect(history.task(task.taskId)?.workflow?.steps[1].state).toBe('dispatched');
    templates[0].body = 'CHANGED';
    store.mutate('disable', 'custom', 1);
    store.mutate('delete', 'custom', 2);
    history.validated('worker', 'valid', undefined, { outcome: 'failed tests' });
    const reload = new DispatchHistoryStore(() => path.join(dir, 'history.json'));
    const pin = reload.task(task.taskId)!.workflow!;
    expect(pin.templates['ship-task'].body).toBe('{{task}}');
    expect(pin.definition.revision).toBe(1);
    expect(pin.steps[0]).toMatchObject({ state: 'skipped', reason: 'No unresolved material risk' });
    expect(pin.steps[1]).toMatchObject({
      state: 'completed',
      dispatchId: accepted.dispatchId,
      outcome: { outcome: 'failed tests' },
    });
    expect(pin.steps[2].state).toBe('planned');
    expect(() => reload.workflowDecision(task.taskId, 'review', false, 'skip')).toThrow('Required');
    expect(() => reload.validate({ owner, projectCwd: dir, taskId: task.taskId })).toThrow(
      'workflowStepId',
    );
  });
  it('rejects corrupt versions and oversized definitions without reseeding or overwriting', () => {
    const { store, filename } = setup();
    fs.writeFileSync(filename, '{"version":99}');
    expect(() => store.list()).toThrow();
    expect(fs.readFileSync(filename, 'utf8')).toBe('{"version":99}');
    expect(() => store.validate({ ...custom(), description: 'x'.repeat(1001) })).toThrow();
  });
});

it('refuses locked/stale writes and selected disable through update; capacity cannot evict active pins', () => {
  const { store, filename, dir, templates } = setup();
  store.list();
  store.mutate('create', undefined, undefined, custom());
  const selected = new FleetWorkflowStore(
    () => filename,
    () => templates,
    () => ['custom'],
  );
  expect(() => selected.mutate('update', 'custom', 1, { ...custom(), enabled: false })).toThrow(
    'Select another',
  );
  const before = fs.readFileSync(filename, 'utf8');
  fs.writeFileSync(filename + '.lock', 'other writer');
  expect(() => store.mutate('update', 'custom', 1, custom())).toThrow('locked');
  expect(fs.readFileSync(filename, 'utf8')).toBe(before);
  fs.unlinkSync(filename + '.lock');
  const history = new DispatchHistoryStore(() => path.join(dir, 'bounded.json'), {
    tasks: 1,
    attempts: 8,
    bytes: 100000,
  });
  const owner = { sessionId: 'owner', isWakeTarget: true, status: 'active' };
  const first = history.startWorkflow(owner, dir, 'active', store.pin(custom()));
  expect(() => history.startWorkflow(owner, dir, 'refused', store.pin(custom()))).toThrow(
    'capacity',
  );
  expect(history.list().map((t) => t.taskId)).toEqual([first.taskId]);
});

it('does not make unrelated optional metadata a standalone launch gate', () => {
  const { dir } = setup();
  const store = new DispatchHistoryStore(() => path.join(dir, 'legacy.json'));
  expect(
    store.accept({
      projectCwd: dir,
      executionCwd: dir,
      sessionId: 'ordinary',
      stage: 'unknown-label',
    } as never),
  ).toBeUndefined();
  const result = store.accept({
    projectCwd: dir,
    executionCwd: dir,
    sessionId: 'attributed',
    stage: 'unknown-label',
    owner: { sessionId: 'manager', isWakeTarget: true, status: 'active' },
  } as never)!;
  expect(store.task(result.taskId)?.attempts[0].stage).toBeUndefined();
});

it('pins unusual library ids as data across JSON persistence', () => {
  const { store, templates } = setup();
  templates.push({
    ...templates[0],
    id: '__proto__',
    body: 'Pinned special-name template {{task}}',
  });
  const definition = custom();
  const noTask = { ...templates[0], id: 'no-task', body: 'Static research', params: [] };
  templates.push(noTask);
  expect(() =>
    store.validate({
      ...definition,
      steps: [{ ...definition.steps[0], template: 'no-task', instructions: 'Apply this' }],
    }),
  ).toThrow('task input');
  definition.steps = [{ ...definition.steps[0], template: '__proto__' }];
  const persisted = JSON.parse(JSON.stringify(store.pin(definition)));
  expect(Object.hasOwn(persisted.templates, '__proto__')).toBe(true);
  expect(persisted.templates.__proto__.body).toBe('Pinned special-name template {{task}}');
});
