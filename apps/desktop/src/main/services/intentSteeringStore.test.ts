import { DatabaseSync } from 'node:sqlite';
import { mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { afterEach, expect, it, vi } from 'vitest';
vi.mock('./configService', () => ({ getConfigDir: () => '/unused-intent-steering-tests' }));
import { INTENT_WORKSPACE_SCHEMA_VERSION, IntentWorkspaceStore } from './intentWorkspaceStore';
import type { IntentDirection, IntentWorkspace } from '../shared/intentWorkspace';

const opened: DatabaseSync[] = [];
function open(file = ':memory:') {
  const db = new DatabaseSync(file);
  opened.push(db);
  return { db, store: new IntentWorkspaceStore(db) };
}
afterEach(() => {
  for (const db of opened.splice(0)) if (db.isOpen) db.close();
});
const target = {
  sessionId: 'worker',
  hub: '',
  label: 'Worker',
  provider: 'codex',
  cwd: '/project',
};
function fixture(file?: string) {
  const { db, store } = open(file);
  const created = store.request({
    action: 'create',
    projectRoot: '/project',
    fields: {
      title: 'Export',
      outcome: 'Filtered rows',
      constraints: 'CSV only',
      successCriteria: 'Tests pass',
      sourceUrl: '',
      status: 'active',
    },
  });
  if (created.action !== 'create') throw new Error('Expected workspace');
  const workspace = created.workspace;
  const attached = store.request({
    action: 'attachSession',
    id: workspace.id,
    expectedRevision: 1,
    session: target,
  });
  if (attached.action !== 'attachSession') throw new Error('Expected execution');
  const prepare = {
    action: 'prepareDirection',
    id: workspace.id,
    directionId: 'direction-one',
    executionId: attached.execution.id,
    expectedRevision: 1,
    text: 'Keep the existing permission checks.',
  };
  const send = {
    action: 'sendDirection',
    id: workspace.id,
    directionId: prepare.directionId,
    attemptId: 'attempt-one',
  };
  return { db, store, workspace, prepare, send };
}
const accepted = () => ({ status: 'accepted' as const, detail: 'Accepted by test transport' });
const directions = (store: IntentWorkspaceStore, workspace: IntentWorkspace) => {
  const result = store.request({ action: 'directions', id: workspace.id });
  if (result.action !== 'directions') throw new Error('Expected directions');
  return result.directions;
};

it('records immutable host-compiled direction and user attribution without sending; survives v2 migration and restart', () => {
  const file = path.join(mkdtempSync(path.join(tmpdir(), 'intent-steering-')), 'work.sqlite');
  const f = fixture(file);
  f.db.exec('DROP TABLE intent_directions; PRAGMA user_version=2');
  f.db.close();
  const migrated = open(file);
  expect(migrated.db.prepare('PRAGMA user_version').get()?.user_version).toBe(
    INTENT_WORKSPACE_SCHEMA_VERSION,
  );
  const result = migrated.store.request({
    ...f.prepare,
    target: { sessionId: 'forged' },
    author: 'agent',
    packet: 'forged',
    attempts: [accepted()],
  });
  expect(result).toMatchObject({
    created: true,
    direction: { target, author: 'user', intentRevision: 1, attempts: [] },
  });
  expect(migrated.store.request(f.prepare)).toMatchObject({ created: false });
  expect(() => migrated.store.request({ ...f.prepare, text: 'Changed under the same ID' })).toThrow(
    'different content',
  );
  const original = directions(migrated.store, f.workspace)[0];
  expect(original.packet).toContain('CSV only');
  expect(original.packet).toContain(f.prepare.text);
  migrated.store.request({
    action: 'update',
    id: f.workspace.id,
    expectedRevision: 1,
    fields: { ...f.workspace, constraints: 'Excel too' },
    reason: 'Changed scope',
  });
  migrated.db.close();
  expect(directions(open(file).store, f.workspace)[0]).toEqual(original);
});

it('records unknown before I/O and fences simultaneous/replayed sends across connections', async () => {
  const file = path.join(mkdtempSync(path.join(tmpdir(), 'intent-steering-race-')), 'work.sqlite');
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
  const pending = f.store.steering.send(f.send, [target], deliver);
  expect(directions(other.store, f.workspace)[0].attempts[0].status).toBe('unknown');
  expect(
    await other.store.steering.send({ ...f.send, attemptId: 'concurrent' }, [target], deliver),
  ).toMatchObject({ dispatched: false });
  finish(accepted());
  await pending;
  expect(deliver).toHaveBeenCalledTimes(1);
  expect(deliver).toHaveBeenCalledWith(target, directions(f.store, f.workspace)[0].packet);
  expect(await other.store.steering.send(f.send, [target], deliver)).toMatchObject({
    dispatched: false,
    direction: { attempts: [{ status: 'accepted' }] },
  });
  expect(deliver).toHaveBeenCalledTimes(1);
});

it('permits an explicit new attempt only after a known failure, preserving every receipt', async () => {
  const f = fixture();
  f.store.request(f.prepare);
  const deliver = vi
    .fn()
    .mockResolvedValueOnce({ status: 'failed', detail: 'Queue full' })
    .mockResolvedValue(accepted());
  await f.store.steering.send(f.send, [target], deliver);
  await f.store.steering.send(f.send, [target], deliver);
  expect(deliver).toHaveBeenCalledTimes(1);
  await f.store.steering.send({ ...f.send, attemptId: 'explicit-retry' }, [target], deliver);
  expect(directions(f.store, f.workspace)[0].attempts.map((a) => a.status)).toEqual([
    'failed',
    'accepted',
  ]);
});

it('keeps network uncertainty unreplayable even after a host restart', async () => {
  const file = path.join(
    mkdtempSync(path.join(tmpdir(), 'intent-steering-unknown-')),
    'work.sqlite',
  );
  const f = fixture(file);
  f.store.request(f.prepare);
  const deliver = vi.fn().mockRejectedValue(new Error('Socket closed after submit'));
  await f.store.steering.send(f.send, [target], deliver);
  f.db.close();
  await open(file).store.steering.send(
    { ...f.send, attemptId: 'new-request-id' },
    [target],
    deliver,
  );
  expect(deliver).toHaveBeenCalledTimes(1);
});

it('does not dispatch when the durable pre-send write fails; post-send receipt failures remain unknown', async () => {
  const f = fixture();
  f.store.request(f.prepare);
  const deliver = vi.fn(async () => accepted());
  f.db.exec(
    "CREATE TRIGGER fail_claim BEFORE UPDATE ON intent_directions BEGIN SELECT RAISE(ABORT, 'disk full'); END",
  );
  await expect(f.store.steering.send(f.send, [target], deliver)).rejects.toThrow('disk full');
  expect(deliver).not.toHaveBeenCalled();
  f.db.exec(
    "DROP TRIGGER fail_claim; CREATE TRIGGER fail_receipt BEFORE UPDATE ON intent_directions WHEN json_extract(new.snapshot, '$.attempts[0].status')='accepted' BEGIN SELECT RAISE(ABORT, 'receipt disk full'); END",
  );
  await expect(f.store.steering.send(f.send, [target], deliver)).rejects.toThrow(
    'receipt could not be saved',
  );
  expect(directions(f.store, f.workspace)[0].attempts[0].status).toBe('unknown');
  await f.store.steering.send({ ...f.send, attemptId: 'retry' }, [target], deliver);
  expect(deliver).toHaveBeenCalledTimes(1);
});

it('pins targets and refuses missing, ended, offline, or same-ID wrong-hub snapshots', async () => {
  const f = fixture();
  f.store.request(f.prepare);
  const deliver = vi.fn(async () => accepted());
  for (const [index, sessions] of [
    [],
    [{ ...target, status: 'ended' }],
    [{ ...target, hubOffline: true }],
    [{ ...target, hub: 'peer' }],
  ].entries()) {
    expect(
      await f.store.steering.send(
        { ...f.send, attemptId: `unavailable-${index}`, target: { ...target, hub: 'peer' } },
        sessions,
        deliver,
      ),
    ).toMatchObject({ dispatched: false, direction: { target } });
  }
  expect(deliver).not.toHaveBeenCalled();
});

it('replaces direction atomically, preserves earlier receipts, and refuses stale branches or old-revision sends', async () => {
  const f = fixture();
  f.store.request(f.prepare);
  let finish!: (value: ReturnType<typeof accepted>) => void;
  const pending = f.store.steering.send(
    f.send,
    [target],
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  f.store.request({
    ...f.prepare,
    directionId: 'replacement',
    supersedesId: f.prepare.directionId,
    text: 'Use the shared permission helper.',
  });
  expect(() =>
    f.store.request({
      ...f.prepare,
      directionId: 'competing',
      supersedesId: f.prepare.directionId,
    }),
  ).toThrow('changed elsewhere');
  finish(accepted());
  await pending;
  const original = directions(f.store, f.workspace).find((d) => d.id === f.prepare.directionId)!;
  expect(original.supersededBy).toBe('replacement');
  expect(original.attempts[0].status).toBe('accepted');
  f.store.request({
    action: 'update',
    id: f.workspace.id,
    expectedRevision: 1,
    fields: { ...f.workspace, constraints: 'Excel too' },
    reason: '',
  });
  const deliver = vi.fn(async () => accepted());
  await expect(
    f.store.steering.send({ ...f.send, directionId: 'replacement' }, [target], deliver),
  ).rejects.toThrow('Intent changed');
  expect(deliver).not.toHaveBeenCalled();
  expect(() =>
    f.store.request({ action: 'finishDirection', id: f.workspace.id, status: 'accepted' }),
  ).toThrow();
});

it('rejects cross-workspace execution and supersession references', () => {
  const f = fixture();
  f.store.request(f.prepare);
  const created = f.store.request({ action: 'create', projectRoot: '/other', fields: f.workspace });
  if (created.action !== 'create') throw new Error('Expected workspace');
  expect(() =>
    f.store.request({ ...f.prepare, id: created.workspace.id, directionId: 'other' }),
  ).toThrow('Link this execution');
  const attached = f.store.request({
    action: 'attachSession',
    id: created.workspace.id,
    expectedRevision: 1,
    session: target,
  });
  if (attached.action !== 'attachSession') throw new Error('Expected execution');
  expect(() =>
    f.store.request({
      ...f.prepare,
      id: created.workspace.id,
      executionId: attached.execution.id,
      directionId: 'other',
      supersedesId: f.prepare.directionId,
    }),
  ).toThrow('does not belong');
});

it('records append-only user reconciliation independently of uncertain delivery and never enables replay', async () => {
  const f = fixture();
  f.store.request(f.prepare);
  const deliver = vi.fn().mockRejectedValue(new Error('Lost response'));
  await f.store.steering.send(f.send, [target], deliver);
  const request = {
    action: 'reconcileDirection',
    id: f.workspace.id,
    directionId: f.prepare.directionId,
    reconciliationId: 'assessment',
    expectedCount: 0,
    assessment: 'observed',
    reason: 'The transcript explains the permission checks',
    author: 'agent',
    status: 'accepted',
  };
  f.store.steering.request(request);
  f.store.steering.request(request);
  expect(directions(f.store, f.workspace)[0]).toMatchObject({
    attempts: [{ status: 'unknown' }],
    reconciliations: [{ author: 'user', assessment: 'observed', reason: request.reason }],
  });
  expect(directions(f.store, f.workspace)[0].reconciliations).toHaveLength(1);
  expect(() => f.store.steering.request({ ...request, reconciliationId: 'stale' })).toThrow(
    'changed elsewhere',
  );
  expect(() => f.store.steering.request({ ...request, reason: 'Changed' })).toThrow(
    'different content',
  );
  await f.store.steering.send({ ...f.send, attemptId: 'after-assessment' }, [target], deliver);
  expect(deliver).toHaveBeenCalledTimes(1);
});
