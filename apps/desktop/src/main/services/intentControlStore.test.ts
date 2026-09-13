import { DatabaseSync } from 'node:sqlite';
import { mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { afterEach, expect, it, vi } from 'vitest';
vi.mock('./configService', () => ({ getConfigDir: () => '/unused-intent-control-tests' }));
import { IntentWorkspaceStore } from './intentWorkspaceStore';
import { INTENT_CONTROL_SCHEMA, IntentControlStore } from './intentControlStore';

const opened: DatabaseSync[] = [];
const target = {
  sessionId: 'worker',
  hub: '',
  label: 'Worker',
  provider: 'codex',
  cwd: '/project',
};
const accepted = () => ({ status: 'accepted' as const, detail: 'Service accepted' });
function open(file = ':memory:') {
  const db = new DatabaseSync(file);
  opened.push(db);
  const workspaceStore = new IntentWorkspaceStore(db);
  db.exec(INTENT_CONTROL_SCHEMA);
  return { db, workspaceStore, store: new IntentControlStore(db) };
}
function fixture(file?: string) {
  const f = open(file);
  const created = f.workspaceStore.request({
    action: 'create',
    projectRoot: '/project',
    fields: {
      title: 'Feature',
      outcome: 'A useful outcome',
      constraints: 'Keep permissions',
      successCriteria: 'Checks pass',
      sourceUrl: '',
      status: 'active',
    },
  });
  if (created.action !== 'create') throw new Error('Expected workspace');
  const workspace = created.workspace;
  const attached = f.workspaceStore.request({
    action: 'attachSession',
    id: workspace.id,
    expectedRevision: 1,
    session: target,
  });
  if (attached.action !== 'attachSession') throw new Error('Expected execution');
  const prepare = {
    action: 'prepareControl',
    id: workspace.id,
    controlId: 'control',
    executionId: attached.execution.id,
    expectedRevision: 1,
    kind: 'interrupt',
    text: 'Review before more changes',
  };
  const send = {
    action: 'sendControl',
    id: workspace.id,
    controlId: 'control',
    attemptId: 'attempt',
  };
  const read = () => {
    const result = f.store.request({ action: 'controls', id: workspace.id });
    if (result.action !== 'controls') throw new Error('Expected controls');
    return result.controls[0];
  };
  return { ...f, workspace, prepare, send, read };
}
afterEach(() => {
  for (const db of opened.splice(0)) if (db.isOpen) db.close();
});

it('pins host-owned identity, intent, attribution and reviewed continuation packet; survives reopen', () => {
  const file = path.join(mkdtempSync(path.join(tmpdir(), 'intent-controls-')), 'work.sqlite');
  const f = fixture(file);
  f.store.request({
    ...f.prepare,
    kind: 'continue',
    target: { sessionId: 'forged' },
    packet: 'forged',
    author: 'agent',
    attempts: [accepted()],
  });
  const control = f.read();
  expect(control).toMatchObject({
    target,
    author: 'user',
    intentRevision: 1,
    attempts: [],
    reconciliations: [],
  });
  expect(control.packet).toContain('Keep permissions');
  expect(control.packet).toContain(`Continue the work as follows:\n${f.prepare.text}`);
  expect(f.store.request({ ...f.prepare, kind: 'continue' })).toMatchObject({ created: false });
  expect(() => f.store.request(f.prepare)).toThrow('different content');
  f.db.close();
  expect(open(file).store.request({ action: 'controls', id: f.workspace.id })).toMatchObject({
    controls: [control],
  });
});
it('persists unknown before I/O, fences concurrent claims, and preserves an assessment written while delivery is in flight', async () => {
  const file = path.join(mkdtempSync(path.join(tmpdir(), 'intent-control-race-')), 'work.sqlite');
  const f = fixture(file),
    other = open(file);
  f.store.request(f.prepare);
  let finish!: (value: ReturnType<typeof accepted>) => void;
  const deliver = vi.fn(
    () =>
      new Promise<ReturnType<typeof accepted>>((resolve) => {
        finish = resolve;
      }),
  );
  const pending = f.store.send(f.send, [target], deliver);
  expect(f.read().attempts[0].status).toBe('unknown');
  expect(
    await other.store.send({ ...f.send, attemptId: 'second' }, [target], deliver),
  ).toMatchObject({ dispatched: false });
  other.store.request({
    action: 'reconcileControl',
    id: f.workspace.id,
    controlId: 'control',
    reconciliationId: 'assessment',
    expectedCount: 0,
    assessment: 'unresolved',
    reason: 'Still waiting for the session',
  });
  finish(accepted());
  await pending;
  expect(f.read()).toMatchObject({
    attempts: [{ status: 'accepted' }],
    reconciliations: [{ assessment: 'unresolved' }],
  });
  expect(deliver).toHaveBeenCalledExactlyOnceWith(target, 'interrupt', f.read().packet);
});
it('never uses a user assessment as a receipt or permission to replay after restart', async () => {
  const file = path.join(
    mkdtempSync(path.join(tmpdir(), 'intent-control-unknown-')),
    'work.sqlite',
  );
  const f = fixture(file);
  f.store.request(f.prepare);
  const deliver = vi.fn().mockRejectedValue(new Error('Socket closed'));
  await f.store.send(f.send, [target], deliver);
  const assessment = {
    action: 'reconcileControl',
    id: f.workspace.id,
    controlId: 'control',
    reconciliationId: 'assessment',
    expectedCount: 0,
    assessment: 'not-observed',
    reason: 'Agent is still working',
    status: 'accepted',
    author: 'agent',
  };
  f.store.request(assessment);
  f.store.request(assessment);
  expect(f.read()).toMatchObject({
    attempts: [{ status: 'unknown' }],
    reconciliations: [{ author: 'user', assessment: 'not-observed' }],
  });
  expect(() => f.store.request({ ...assessment, reconciliationId: 'stale' })).toThrow(
    'changed elsewhere',
  );
  expect(() => f.store.request({ ...assessment, reason: '' })).toThrow('reason');
  f.db.close();
  expect(
    await open(file).store.send({ ...f.send, attemptId: 'after-restart' }, [target], deliver),
  ).toMatchObject({ dispatched: false });
  expect(deliver).toHaveBeenCalledTimes(1);
});
it('sends nothing on failed claim writes and leaves receipt write failure durably uncertain', async () => {
  const f = fixture();
  f.store.request(f.prepare);
  const deliver = vi.fn(async () => accepted());
  f.db.exec(
    "CREATE TRIGGER fail_claim BEFORE UPDATE ON intent_controls BEGIN SELECT RAISE(ABORT, 'disk full'); END",
  );
  await expect(f.store.send(f.send, [target], deliver)).rejects.toThrow('disk full');
  expect(deliver).not.toHaveBeenCalled();
  f.db.exec(
    "DROP TRIGGER fail_claim; CREATE TRIGGER fail_receipt BEFORE UPDATE ON intent_controls WHEN json_extract(new.snapshot, '$.attempts[0].status')='accepted' BEGIN SELECT RAISE(ABORT, 'disk full'); END",
  );
  await expect(f.store.send(f.send, [target], deliver)).rejects.toThrow(
    'receipt could not be saved',
  );
  expect(f.read().attempts[0].status).toBe('unknown');
  await f.store.send({ ...f.send, attemptId: 'retry' }, [target], deliver);
  expect(deliver).toHaveBeenCalledTimes(1);
});
it('refuses missing, stopped, offline and wrong-hub targets without I/O; permits explicit retry after a proven refusal', async () => {
  const f = fixture();
  f.store.request(f.prepare);
  const deliver = vi.fn(async () => accepted());
  for (const [index, sessions] of [
    [],
    [{ ...target, status: 'stopped' }],
    [{ ...target, hubOffline: true }],
    [{ ...target, hub: 'elsewhere' }],
  ].entries()) {
    expect(
      await f.store.send({ ...f.send, attemptId: `failed-${index}` }, sessions, deliver),
    ).toMatchObject({ dispatched: false });
  }
  expect(deliver).not.toHaveBeenCalled();
  await f.store.send(f.send, [target], deliver);
  expect(f.read().attempts.map((a) => a.status)).toEqual([
    'failed',
    'failed',
    'failed',
    'failed',
    'accepted',
  ]);
});
it('rejects stale saved controls, invalid kind, unlinked and cross-workspace execution references', async () => {
  const f = fixture();
  f.store.request(f.prepare);
  expect(() => f.store.request({ ...f.prepare, controlId: 'bad', kind: 'kill' })).toThrow('kind');
  expect(() => f.store.request({ ...f.prepare, controlId: 'bad', executionId: 'missing' })).toThrow(
    'Link this execution',
  );
  const second = f.workspaceStore.request({
    action: 'create',
    projectRoot: '/other',
    fields: f.workspace,
  });
  if (second.action !== 'create') throw new Error('Expected workspace');
  expect(() =>
    f.store.request({ ...f.prepare, id: second.workspace.id, controlId: 'other' }),
  ).toThrow('Link this execution');
  expect(() =>
    f.store.request({ action: 'reconcileControl', id: second.workspace.id, controlId: 'control' }),
  ).toThrow('does not belong');
  f.workspaceStore.request({
    action: 'update',
    id: f.workspace.id,
    expectedRevision: 1,
    fields: { ...f.workspace, outcome: 'Different outcome' },
    reason: '',
  });
  const deliver = vi.fn(async () => accepted());
  await expect(f.store.send(f.send, [target], deliver)).rejects.toThrow('Intent changed');
  expect(deliver).not.toHaveBeenCalled();
});
