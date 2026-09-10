/**
 * The facade path for manager task references: authentication to a LIVE, LOCAL,
 * owning manager and the exact task/project, plus CAS and conflict reporting.
 * The tool never accepts a session id as an argument; callerSessionId is stamped
 * by the facade, so these tests drive it the same way the bus does.
 */
import { afterAll, describe, expect, it, vi } from 'vitest';
import fs from 'fs';
import os from 'os';
import path from 'path';
import type { WorkflowResponse } from '../shared/fleetWorkflow';

const fixture = vi.hoisted(() => ({
  root: '',
  sessions: {} as Record<
    string,
    { sessionId: string; isWakeTarget?: boolean; status: string; hub?: string } | undefined
  >,
}));
vi.mock('./configService', () => ({
  getConfigDir: () => fixture.root,
  configService: { getConfig: () => ({ agents: {}, projects: {} }) },
}));
vi.mock('./claudeSessionStore', () => ({
  claudeSessionStore: { getSnapshot: (id: string) => fixture.sessions[id] },
}));
vi.mock('./libraryService', () => ({
  libraryService: {
    list: () =>
      ['ship-task', 'scout-task', 'review-task'].map((id) => ({
        id,
        kind: 'dispatch',
        scope: 'global',
        body: '{{task}}',
        resultSchema: { type: 'object' },
      })),
  },
}));

import { fleetWorkflowRequest } from './fleetWorkflowService';

const live = (sessionId: string) => ({ sessionId, isWakeTarget: true, status: 'active' });
fixture.sessions = {
  manager: live('manager'),
  other: live('other'),
  dead: { sessionId: 'dead', isWakeTarget: true, status: 'ended' },
  remote: { ...live('remote'), hub: 'peer' },
};
function start() {
  fixture.root ||= fs.mkdtempSync(path.join(os.tmpdir(), 'manager-refs-service-'));
  const started = fleetWorkflowRequest(
    { op: 'start', cwd: fixture.root, title: 'Task' },
    'manager',
  );
  if (!started.ok || !started.task) throw new Error(JSON.stringify(started));
  return started.task;
}
const get = (taskId: string, caller = 'manager', cwd = fixture.root): WorkflowResponse =>
  fleetWorkflowRequest({ op: 'taskReferences', taskId, cwd }, caller);
const set = (
  taskId: string,
  upsert: unknown,
  expectedTaskRevision: number,
  caller = 'manager',
  cwd = fixture.root,
  remove?: unknown,
): WorkflowResponse =>
  fleetWorkflowRequest(
    { op: 'setTaskReferences', taskId, cwd, expectedTaskRevision, upsert, remove },
    caller,
  );
afterAll(() => {
  if (fixture.root) fs.rmSync(fixture.root, { recursive: true, force: true });
});

describe('who may edit a task reference', () => {
  it('lets the owning live local manager read and write its own task', () => {
    const task = start();
    const read = get(task.taskId);
    expect(read).toMatchObject({ ok: true, references: {}, taskRevision: task.revision ?? 0 });
    const written = set(
      task.taskId,
      [
        {
          kind: 'pullRequest',
          number: '9492',
          url: 'https://dev.azure.test/p/_git/r/pullrequest/9492',
        },
      ],
      read.ok ? (read.taskRevision as number) : -1,
    );
    expect(written).toMatchObject({ ok: true });
    expect(written.ok && written.references).toEqual({
      pullRequest: { number: '9492', url: 'https://dev.azure.test/p/_git/r/pullrequest/9492' },
    });
    // The read result carries the current references and revision, so a manager
    // can reconcile without guessing.
    expect(get(task.taskId)).toMatchObject({
      ok: true,
      taskRevision: (written.ok && written.taskRevision) as number,
    });
  });
  it.each([
    ['a different live manager', 'other'],
    ['an ended manager', 'dead'],
    ['a remote/hub session', 'remote'],
    ['an unknown session', 'ghost'],
    ['an empty caller session', ''],
  ])('refuses %s', (_name, caller) => {
    const task = start();
    const revision = task.revision ?? 0;
    expect(get(task.taskId, caller as string)).toMatchObject({ ok: false, code: 'unavailable' });
    expect(
      set(task.taskId, [{ kind: 'ticket', id: 'X' }], revision, caller as string),
    ).toMatchObject({ ok: false, code: 'unavailable' });
    // Nothing was written by the refused caller.
    expect(get(task.taskId)).toMatchObject({ ok: true, references: {} });
  });
  it('refuses a request with no caller session at all', () => {
    const task = start();
    expect(
      fleetWorkflowRequest({ op: 'taskReferences', taskId: task.taskId, cwd: fixture.root }),
    ).toMatchObject({ ok: false, code: 'unavailable' });
    expect(
      fleetWorkflowRequest({
        op: 'setTaskReferences',
        taskId: task.taskId,
        cwd: fixture.root,
        expectedTaskRevision: task.revision ?? 0,
        upsert: [{ kind: 'ticket', id: 'X' }],
      }),
    ).toMatchObject({ ok: false, code: 'unavailable' });
    expect(get(task.taskId)).toMatchObject({ ok: true, references: {} });
  });
  it('refuses the right manager addressing the wrong project or an unknown task', () => {
    const task = start();
    expect(
      set(task.taskId, [{ kind: 'ticket', id: 'X' }], 0, 'manager', '/elsewhere'),
    ).toMatchObject({ ok: false, code: 'unavailable' });
    expect(get('no-such-task')).toMatchObject({ ok: false, code: 'unavailable' });
  });
});

describe('CAS and conflict reporting', () => {
  it('reports a conflict with the current revision and references instead of overwriting', () => {
    const task = start();
    const first = set(task.taskId, [{ kind: 'ticket', id: 'FIRST' }], task.revision ?? 0);
    expect(first).toMatchObject({ ok: true });
    const stale = set(task.taskId, [{ kind: 'ticket', id: 'SECOND' }], task.revision ?? 0);
    expect(stale).toMatchObject({ ok: false, code: 'conflict' });
    expect(stale.ok === false && stale.currentRevision).toBe(first.ok && first.taskRevision);
    expect(stale.ok === false && stale.references).toEqual({ tickets: [{ id: 'FIRST' }] });
    expect(get(task.taskId)).toMatchObject({
      ok: true,
      references: { tickets: [{ id: 'FIRST' }] },
    });
  });
  it('rejects an unsafe URL through the same host validator the Inspector uses', () => {
    const task = start();
    expect(
      set(
        task.taskId,
        [{ kind: 'reference', label: 'bad', url: 'file:///etc/passwd' }],
        task.revision ?? 0,
      ),
    ).toMatchObject({ ok: false, code: 'unavailable' });
    expect(get(task.taskId)).toMatchObject({ ok: true, references: {} });
  });
  it('removes an exact entry and leaves the rest', () => {
    const task = start();
    let revision = task.revision ?? 0;
    const added = set(
      task.taskId,
      [
        { kind: 'ticket', id: 'KEEP' },
        { kind: 'ticket', id: 'DROP' },
      ],
      revision,
    );
    revision = (added.ok && (added.taskRevision as number)) || 0;
    const removed = set(task.taskId, undefined, revision, 'manager', fixture.root, [
      { kind: 'ticket', id: 'DROP' },
    ]);
    expect(removed.ok && removed.references).toEqual({ tickets: [{ id: 'KEEP' }] });
  });
});
