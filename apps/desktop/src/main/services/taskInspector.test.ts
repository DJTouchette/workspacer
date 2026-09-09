import { afterEach, describe, expect, it, vi } from 'vitest';
import fs from 'fs';
import os from 'os';
import path from 'path';
import * as atomic from '../lib/atomicWriteFile';
import { spawn } from 'child_process';
import { DispatchHistoryStore } from './dispatchHistoryStore';
import { WORKFLOW_STARTERS, type WorkflowPin } from '../shared/fleetWorkflow';
import { validateTaskLinks } from '../shared/dispatchHistory';
const dirs: string[] = [];
afterEach(() => {
  vi.restoreAllMocks();
  dirs.splice(0).forEach((d) => fs.rmSync(d, { recursive: true, force: true }));
});
const owner = { sessionId: 'manager', isWakeTarget: true, status: 'active' };
function fixture(limits?: { tasks: number; attempts: number; bytes: number }) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'task-inspector-'));
  dirs.push(dir);
  const file = path.join(dir, 'history.json');
  const store = new DispatchHistoryStore(() => file, limits);
  const definition = structuredClone(WORKFLOW_STARTERS[0]);
  const pin: WorkflowPin = {
    definition,
    hash: 'frozen-hash',
    templates: {
      'ship-task': {
        id: 'ship-task',
        body: '{{task}}',
        params: [],
        resultSchema: { type: 'object' },
      },
    },
    steps: definition.steps.map((s) => ({ id: s.id, state: 'planned' })),
  };
  const task = store.startWorkflow(owner, dir, 'Task', pin);
  const edit = (
    stepId: string,
    revision = store.task(task.taskId)?.revision ?? 0,
    session = stopped,
  ) =>
    store.editByHostUser(
      { taskId: task.taskId, expectedTaskRevision: revision, action: 'waive', stepId },
      session,
      () => false,
    );
  return { dir, file, store, task, pin, edit };
}
const stopped = (id: string) => ({ sessionId: id, status: 'ended' });
const observe = (store: DispatchHistoryStore, id: string, status = 'ended') =>
  store.observe({ sessionId: id, status, ambientState: 'idle' } as never);
function implement(f: ReturnType<typeof fixture>) {
  f.store.workflowDecision(f.task.taskId, 'scout', false, 'No material risk');
  return f.store.accept({
    owner,
    taskId: f.task.taskId,
    projectCwd: f.dir,
    executionCwd: f.dir,
    sessionId: 'worker',
    workflowStepId: 'implement',
  })!;
}
describe('host-user task edits', () => {
  it('allows required FUTURE review while implementation runs, retains pin, and leaves earlier work unfinished', () => {
    const f = fixture();
    implement(f);
    expect(() => f.store.workflowDecision(f.task.taskId, 'review', false, 'skip')).toThrow();
    const revision = f.store.task(f.task.taskId)!.revision!;
    expect(f.edit('review')).toMatchObject({ ok: true, task: { revision: revision + 1 } });
    const task = f.store.task(f.task.taskId)!;
    expect(task.workflow!.steps.map((s) => s.state)).toEqual(['skipped', 'dispatched', 'waived']);
    expect(task.workflow!.definition).toEqual(f.pin.definition);
    expect(task.workflow!.templates).toEqual(f.pin.templates);
    expect(task.workflow!.hash).toBe(f.pin.hash);
    expect(task.audit).toEqual([
      expect.objectContaining({
        actor: 'host-user',
        stepId: 'review',
        action: 'waive',
        createdAt: expect.any(String),
        reason: expect.any(String),
      }),
    ]);
    expect(task.workflow!.steps[2].outcome).toBeUndefined();
    expect(task.attempts[0].resultContract).toBe('absent');
    expect(f.edit('implement')).toMatchObject({ ok: false });
  });
  it('keeps manager refusal for the next required step', () => {
    const f = fixture();
    implement(f);
    f.store.validated('worker', 'valid', undefined, { commit: 'abc' });
    expect(() => f.store.workflowDecision(f.task.taskId, 'review', false, 'Not needed')).toThrow(
      'Required step',
    );
    expect(f.edit('review').ok).toBe(true);
  });
  it.each(['invalid', 'escalated'] as const)(
    'preserves %s failure evidence and refuses live, wrong, unknown and stale sessions',
    (contract) => {
      const f = fixture();
      implement(f);
      f.store.validated('worker', contract, 'evidence', { failure: 'original' });
      expect(f.edit('implement').ok).toBe(false);
      observe(f.store, 'worker');
      expect(f.edit('implement', undefined, () => undefined).ok).toBe(false);
      expect(
        f.edit('implement', undefined, () => ({ sessionId: 'wrong', status: 'ended' })).ok,
      ).toBe(false);
      expect(f.edit('implement', undefined, (id) => ({ sessionId: id, status: 'active' })).ok).toBe(
        false,
      );
      const stale = new DispatchHistoryStore(() => f.file);
      expect(
        stale.editByHostUser(
          {
            taskId: f.task.taskId,
            expectedTaskRevision: f.store.task(f.task.taskId)!.revision!,
            action: 'waive',
            stepId: 'implement',
          },
          () => undefined,
          () => false,
        ).ok,
      ).toBe(false);
      expect(f.edit('implement').ok).toBe(true);
      observe(f.store, 'worker');
      f.store.validated('worker', contract, 'evidence', { failure: 'late' });
      const task = f.store.task(f.task.taskId)!;
      expect(task.workflow!.steps[1]).toMatchObject({
        state: 'waived',
        outcome: { failure: 'original' },
        reason: `Worker result contract: ${contract}`,
      });
      expect(task.attempts[0]).toMatchObject({
        resultContract: contract,
        reviewEvidenceId: 'evidence',
      });
    },
  );
  it('serializes two readers, lifecycle updates, adoption, links and reload without lost writes', () => {
    const f = fixture();
    implement(f);
    const second = new DispatchHistoryStore(() => f.file);
    const revision = second.task(f.task.taskId)!.revision!;
    const first = f.store.editByHostUser(
      {
        taskId: f.task.taskId,
        expectedTaskRevision: revision,
        action: 'links',
        links: { pullRequest: { number: '12' }, tickets: [{ id: 'WKS-1' }] },
      },
      stopped,
      () => false,
    );
    expect(first.ok).toBe(true);
    expect(
      second.editByHostUser(
        {
          taskId: f.task.taskId,
          expectedTaskRevision: revision,
          action: 'waive',
          stepId: 'review',
        },
        stopped,
        () => false,
      ),
    ).toMatchObject({ ok: false, code: 'conflict' });
    observe(second, 'worker');
    second.adoptWorkflowTasks('manager', 'successor');
    expect(f.edit('review').ok).toBe(true);
    const reload = new DispatchHistoryStore(() => f.file).task(f.task.taskId)!;
    expect(reload.ownerSessionId).toBe('successor');
    expect(reload.links).toEqual({ pullRequest: { number: '12' }, tickets: [{ id: 'WKS-1' }] });
    expect(reload.attempts[0]).toMatchObject({ lifecycle: 'ended', stale: true });
    expect(reload.workflow!.steps[2].state).toBe('waived');
    expect(reload.audit).toHaveLength(2);
    expect(reload.revision).toBeGreaterThan(revision + 2);
  });
  it('rolls back ownership/revisions and disk after persistence failure', () => {
    const f = fixture();
    const before = fs.readFileSync(f.file, 'utf8');
    vi.spyOn(atomic, 'atomicWriteFileSync').mockImplementation(() => {
      throw new Error('disk failure');
    });
    expect(() => f.store.adoptWorkflowTasks('manager', 'replacement')).toThrow();
    expect(f.store.task(f.task.taskId)!.ownerSessionId).toBe('manager');
    expect(fs.readFileSync(f.file, 'utf8')).toBe(before);
  });
  it('honors another process holding the file lock, then succeeds after release', async () => {
    const f = fixture();
    const child = spawn(
      process.execPath,
      [
        '-e',
        `const fs=require('fs'); fs.writeFileSync(process.argv[1]+'.lock','held',{flag:'wx'}); console.log('ready'); process.stdin.once('data',()=>{fs.unlinkSync(process.argv[1]+'.lock');process.exit(0)});`,
        f.file,
      ],
      { stdio: ['pipe', 'pipe', 'pipe'] },
    );
    await new Promise<void>((resolve, reject) => {
      child.stdout.once('data', () => resolve());
      child.once('error', reject);
    });
    try {
      expect(f.edit('review')).toMatchObject({
        ok: false,
        error: expect.stringContaining('locked'),
      });
    } finally {
      child.stdin.write('release');
      await new Promise((r) => child.once('exit', r));
    }
    expect(f.edit('review').ok).toBe(true);
  });
  it('treats waived steps as terminal for capacity but protects earlier unfinished steps', () => {
    const f = fixture({ tasks: 1, attempts: 20, bytes: 100000 });
    f.edit('review');
    expect(() => f.store.startWorkflow(owner, f.dir, 'Second', f.pin)).toThrow('capacity');
    f.edit('scout');
    f.edit('implement');
    expect(() => f.store.startWorkflow(owner, f.dir, 'Second', f.pin)).not.toThrow();
    expect(f.store.task(f.task.taskId)).toBeUndefined();
  });
  it('rejects stale revisions, completed actions, unknown actions, busy dispatches and oversized reasons without changing bytes', () => {
    const f = fixture();
    const before = fs.readFileSync(f.file, 'utf8');
    const request = {
      taskId: f.task.taskId,
      expectedTaskRevision: f.task.revision!,
      action: 'waive' as const,
      stepId: 'review',
    };
    expect(f.store.editByHostUser(request, stopped, () => true).ok).toBe(false);
    expect(
      f.store.editByHostUser({ ...request, reason: 'x'.repeat(2001) }, stopped, () => false).ok,
    ).toBe(false);
    expect(
      f.store.editByHostUser({ ...request, action: 'complete' } as never, stopped, () => false).ok,
    ).toBe(false);
    expect(fs.readFileSync(f.file, 'utf8')).toBe(before);
    implement(f);
    f.store.validated('worker', 'valid');
    observe(f.store, 'worker');
    expect(f.edit('implement').ok).toBe(false);
  });
  it('bounds link audit while retaining every waiver', () => {
    const f = fixture();
    f.edit('review');
    for (let n = 1; n <= 45; n++)
      expect(
        f.store.editByHostUser(
          {
            taskId: f.task.taskId,
            expectedTaskRevision: f.store.task(f.task.taskId)!.revision!,
            action: 'links',
            links: { pullRequest: { number: String(n) } },
          },
          stopped,
          () => false,
        ).ok,
      ).toBe(true);
    expect(f.store.task(f.task.taskId)!.audit).toHaveLength(41);
    expect(f.store.task(f.task.taskId)!.audit![0].action).toBe('waive');
  });
});
it('validates user references and opens only exact recorded task targets', () => {
  const f = fixture();
  for (const links of [
    { pullRequest: { url: 'file:///etc/passwd' } },
    { pullRequest: { number: '0' } },
    { pullRequest: { url: 'https://user:pass@host' } },
    { tickets: [{ id: 'a' }, { id: 'A' }] },
    { tickets: Array(21).fill({ id: 'x' }) },
    { tickets: [{ id: 'x'.repeat(201) }] },
    { references: [{ label: 'a', url: 'javascript:alert(1)' }] },
    { pullRequest: { url: 'https://host' }, tickets: [{ id: 'x', url: 'https://host/' }] },
  ])
    expect(() => validateTaskLinks(links)).toThrow();
  expect(
    validateTaskLinks({
      pullRequest: { url: 'https://example.test/pr/1' },
      tickets: [{ id: 'T-1' }],
      references: [{ label: 'Design', url: 'http://example.test/design' }],
    }),
  ).toMatchObject({ tickets: [{ id: 'T-1' }] });
  f.store.workflowDecision(f.task.taskId, 'scout', false, 'No risk');
  const worktree = path.join(f.dir, 'tree');
  fs.mkdirSync(worktree);
  const dispatch = f.store.accept({
    owner,
    taskId: f.task.taskId,
    projectCwd: f.dir,
    executionCwd: worktree,
    sessionId: 'worker',
    workflowStepId: 'implement',
    worktree: { allocated: true, requested: true, fallback: false, branch: 'recorded' },
  })!;
  expect(
    f.store.openTarget({
      taskId: f.task.taskId,
      kind: 'worktree',
      dispatchId: dispatch.dispatchId,
    }),
  ).toEqual({ kind: 'worktree', target: worktree });
  expect(() =>
    f.store.openTarget({ taskId: f.task.taskId, kind: 'worktree', dispatchId: 'foreign' }),
  ).toThrow();
  fs.rmdirSync(worktree);
  fs.symlinkSync(os.tmpdir(), worktree);
  expect(() =>
    f.store.openTarget({
      taskId: f.task.taskId,
      kind: 'worktree',
      dispatchId: dispatch.dispatchId,
    }),
  ).toThrow();
  expect(() =>
    f.store.openTarget({ taskId: f.task.taskId, kind: 'url', reference: 'pullRequest' }),
  ).toThrow();
});
