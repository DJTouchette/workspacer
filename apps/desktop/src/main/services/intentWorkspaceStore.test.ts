import { DatabaseSync } from 'node:sqlite';
import { mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { buildIntentContext, type IntentFields } from '../shared/intentWorkspace';
vi.mock('./configService', () => ({ getConfigDir: () => '/unused-by-store-tests' }));
import {
  IntentWorkspaceStore,
  captureIntentSessions,
  INTENT_WORKSPACE_SCHEMA_VERSION,
} from './intentWorkspaceStore';

const fields: IntentFields = {
  title: 'Export results',
  outcome: 'Export the filtered rows',
  constraints: 'Existing permissions',
  successCriteria: 'Filters preserved',
  sourceUrl: 'https://dev.azure.com/team/project/_workitems/edit/4821',
  status: 'draft',
};
const opened: DatabaseSync[] = [];
function open(file = ':memory:') {
  const db = new DatabaseSync(file);
  opened.push(db);
  return { db, store: new IntentWorkspaceStore(db) };
}
function create(store: IntentWorkspaceStore, projectRoot = path.resolve('/project')) {
  const result = store.request({ action: 'create', projectRoot, fields });
  if (result.action !== 'create') throw new Error('Expected create');
  return result.workspace;
}
afterEach(() => {
  vi.useRealTimers();
  for (const db of opened.splice(0)) if (db.isOpen) db.close();
});

describe('durable intent workspaces', () => {
  const session = {
    sessionId: 'session-one',
    hub: '',
    label: 'Export agent',
    provider: 'codex',
    cwd: '/project/worktree',
  };

  it('captures a finish and a late final report without any UI read, retaining them after restart', () => {
    const file = path.join(mkdtempSync(path.join(tmpdir(), 'intent-background-')), 'work.sqlite');
    const first = open(file);
    const workspace = create(first.store);
    first.store.request({
      action: 'attachSession',
      id: workspace.id,
      expectedRevision: 1,
      session,
    });
    first.store.capture(captureIntentSessions([{ ...session, ambientState: 'streaming' }]));
    first.store.capture(captureIntentSessions([{ ...session, status: 'ended' }]));
    first.store.capture(
      captureIntentSessions([
        {
          ...session,
          status: 'ended',
          conversation: [{ role: 'assistant', content: 'Exports complete. 12 tests passed.' }],
        },
      ]),
    );
    first.db.close();
    const result = open(file).store.request({ action: 'executions', id: workspace.id });
    expect(result).toMatchObject({
      executions: [
        { lastObservation: { state: 'stopped', summary: 'Exports complete. 12 tests passed.' } },
      ],
    });
  });

  it('bounds streaming writes, captures transitions immediately, and ignores duplicate/offline/wrong-hub samples', () => {
    vi.useFakeTimers();
    const { store, db } = open();
    const workspace = create(store);
    store.request({ action: 'attachSession', id: workspace.id, expectedRevision: 1, session });
    const sample = (content: string, extra = {}) =>
      captureIntentSessions([
        {
          ...session,
          ambientState: 'streaming',
          conversation: [{ role: 'assistant', content }],
          ...extra,
        },
      ]);
    store.capture(sample('Starting'));
    const changes = () => Number(db.prepare('SELECT total_changes() AS n').get()!.n);
    const before = changes();
    for (let i = 0; i < 100; i++) {
      vi.advanceTimersByTime(10);
      store.capture(sample(`Token ${i}`));
    }
    expect(changes()).toBe(before);
    vi.advanceTimersByTime(5000);
    store.capture(sample('Checkpoint'));
    expect(changes()).toBe(before + 1);
    store.capture(sample('Final', { ambientState: 'idle' }));
    expect(changes()).toBe(before + 2);
    store.capture(sample('Final', { ambientState: 'idle' }));
    store.capture(sample('Offline', { hubOffline: true }));
    store.capture(sample('Other machine', { hub: 'peer' }));
    expect(changes()).toBe(before + 2);
    // Sparse reads may update status but must never erase the saved report.
    expect(
      store.request({ action: 'executions', id: workspace.id }, [
        { ...session, ambientState: 'idle' },
      ]),
    ).toMatchObject({ executions: [{ lastObservation: { summary: 'Final' } }] });
  });

  it('refreshes its linked-session index when another connection attaches an execution', () => {
    const file = path.join(
      mkdtempSync(path.join(tmpdir(), 'intent-background-link-')),
      'work.sqlite',
    );
    const first = open(file),
      second = open(file);
    const workspace = create(first.store);
    expect(first.store.trackedSessions()).toEqual([]);
    second.store.request({
      action: 'attachSession',
      id: workspace.id,
      expectedRevision: 1,
      session,
    });
    first.store.capture(
      captureIntentSessions([
        {
          ...session,
          ambientState: 'idle',
          conversation: [{ role: 'assistant', content: 'Done' }],
        },
      ]),
    );
    expect(second.store.request({ action: 'executions', id: workspace.id })).toMatchObject({
      executions: [{ lastObservation: { summary: 'Done' } }],
    });
  });

  it('shows failed writes alongside retained records and retries the queued final report after the session disappears', () => {
    const { store, db } = open();
    const workspace = create(store);
    store.request({ action: 'attachSession', id: workspace.id, expectedRevision: 1, session });
    db.exec(
      "CREATE TRIGGER fail_capture BEFORE UPDATE ON intent_executions BEGIN SELECT RAISE(ABORT, 'disk write failed'); END;",
    );
    expect(() =>
      store.capture(
        captureIntentSessions([
          {
            ...session,
            status: 'ended',
            conversation: [{ role: 'assistant', content: 'Final report' }],
          },
        ]),
      ),
    ).toThrow('disk write failed');
    expect(store.request({ action: 'executions', id: workspace.id })).toMatchObject({
      captureWarning: expect.stringContaining('disk write failed'),
      executions: [{ lastObservation: null }],
    });
    db.exec('DROP TRIGGER fail_capture');
    expect(store.request({ action: 'executions', id: workspace.id })).toMatchObject({
      executions: [{ lastObservation: { state: 'stopped', summary: 'Final report' } }],
    });
    expect(store.captureWarning).toBeUndefined();
  });

  it('pins launch context and makes claims idempotent across connections and later revisions', () => {
    const file = path.join(mkdtempSync(path.join(tmpdir(), 'intent-launch-')), 'work.sqlite');
    const first = open(file),
      second = open(file);
    const workspace = create(first.store);
    const input = {
      action: 'prepareExecution',
      id: workspace.id,
      expectedRevision: 1,
      executionId: 'run-one',
      task: 'Implement exports',
    };
    const prepared = first.store.request(input);
    expect(prepared).toMatchObject({
      created: true,
      execution: {
        state: 'launching',
        intentRevision: 1,
        session: null,
        contextPacket: buildIntentContext(workspace, 'run-one', 'Implement exports'),
      },
    });
    second.store.request({
      action: 'update',
      id: workspace.id,
      expectedRevision: 1,
      fields: { ...fields, constraints: 'Excel now required' },
      reason: 'Product decision',
    });
    expect(second.store.request(input)).toMatchObject({
      created: false,
      execution: {
        intentRevision: 1,
        contextPacket: buildIntentContext(workspace, 'run-one', 'Implement exports'),
      },
    });
    expect(() => second.store.request({ ...input, executionId: 'run-two' })).toThrow(
      'Intent changed',
    );
    const other = create(first.store);
    expect(() => second.store.request({ ...input, id: other.id })).toThrow('another workspace');
  });

  it('keeps tracking-only associations distinct and uses hub-qualified identity', () => {
    const { store } = open();
    const workspace = create(store);
    const input = { action: 'attachSession', id: workspace.id, expectedRevision: 1, session };
    const first = store.request(input);
    expect(first).toMatchObject({
      execution: { kind: 'attached', state: 'linked', contextPacket: null, session },
    });
    expect(store.request(input)).toEqual(first);
    store.request({ ...input, session: { ...session, hub: 'remote-host' } });
    const result = store.request({ action: 'executions', id: workspace.id });
    expect(result.action === 'executions' && result.executions).toHaveLength(2);
  });

  it('reconciles an uncertain launch without overwriting an already linked identity', () => {
    const { store } = open();
    const workspace = create(store);
    store.request({
      action: 'prepareExecution',
      id: workspace.id,
      expectedRevision: 1,
      executionId: 'run-one',
      task: 'Implement',
    });
    expect(
      store.request({ action: 'markExecutionUnknown', id: workspace.id, executionId: 'run-one' }),
    ).toMatchObject({ execution: { state: 'unknown' } });
    store.request({ action: 'linkExecution', id: workspace.id, executionId: 'run-one', session });
    expect(
      store.request({ action: 'markExecutionUnknown', id: workspace.id, executionId: 'run-one' }),
    ).toMatchObject({ execution: { state: 'linked', session } });
    expect(() =>
      store.request({
        action: 'linkExecution',
        id: workspace.id,
        executionId: 'run-one',
        session: { ...session, sessionId: 'wrong' },
      }),
    ).toThrow('already linked');
    const other = create(store);
    expect(() =>
      store.request({ action: 'linkExecution', id: other.id, executionId: 'run-one', session }),
    ).toThrow('does not belong');
  });

  it('retains host-observed results and references after restart without treating offline telemetry as fresh', () => {
    const file = path.join(mkdtempSync(path.join(tmpdir(), 'intent-observation-')), 'work.sqlite');
    const first = open(file);
    const workspace = create(first.store);
    const remote = { ...session, hub: 'remote-host' };
    first.store.request(
      { action: 'attachSession', id: workspace.id, expectedRevision: 1, session: remote },
      [
        {
          ...remote,
          ambientState: 'idle',
          conversation: [{ role: 'assistant', content: 'Exports implemented; 12 tests passed.' }],
        },
      ],
    );
    const pr = {
      action: 'addWorkLink',
      id: workspace.id,
      kind: 'pull-request',
      target: 'https://example.com/pr/42',
    };
    expect(first.store.request(pr)).toEqual(first.store.request(pr));
    first.store.request({
      action: 'addWorkLink',
      id: workspace.id,
      kind: 'branch',
      target: 'portal / feature/export',
    });
    first.db.close();
    const second = open(file);
    const read = {
      action: 'executions',
      id: workspace.id,
      sessions: [{ ...remote, ambientState: 'thinking' }],
    };
    expect(
      second.store.request(read, [
        {
          ...remote,
          hubOffline: true,
          ambientState: 'thinking',
          conversation: [{ role: 'assistant', content: 'Stale text' }],
        },
      ]),
    ).toMatchObject({
      executions: [
        { lastObservation: { state: 'idle', summary: 'Exports implemented; 12 tests passed.' } },
      ],
      links: [{ kind: 'branch' }, { kind: 'pull-request' }],
    });
    expect(() => second.store.request({ ...pr, target: 'javascript:alert(1)' })).toThrow('HTTP');
  });

  it('migrates an existing v1 workspace database without losing its history', () => {
    const { store, db } = open();
    const workspace = create(store);
    db.exec(
      'DROP TABLE intent_directions; DROP TABLE intent_executions; DROP TABLE intent_work_links; PRAGMA user_version=1;',
    );
    const migrated = new IntentWorkspaceStore(db);
    expect(migrated.request({ action: 'history', id: workspace.id })).toMatchObject({
      revisions: [{ revision: 1, snapshot: workspace }],
    });
    expect(
      migrated.request({ action: 'attachSession', id: workspace.id, expectedRevision: 1, session }),
    ).toMatchObject({ execution: { intentRevision: 1 } });
  });

  it('recovers project-owned state and full revisions after closing and reopening SQLite', () => {
    const file = path.join(mkdtempSync(path.join(tmpdir(), 'intent-workspaces-')), 'work.sqlite');
    const first = open(file);
    const workspace = create(first.store);
    first.store.request({
      action: 'update',
      id: workspace.id,
      expectedRevision: 1,
      fields: { ...fields, status: 'active', constraints: 'CSV only for this release' },
      reason: 'Agreed release scope',
    });
    first.db.close();
    const second = open(file);
    expect(second.store.request({ action: 'list' })).toMatchObject({
      workspaces: [
        {
          id: workspace.id,
          projectRoot: workspace.projectRoot,
          revision: 2,
          constraints: 'CSV only for this release',
        },
      ],
    });
    expect(second.store.request({ action: 'history', id: workspace.id })).toMatchObject({
      revisions: [
        {
          revision: 2,
          reason: 'Agreed release scope',
          snapshot: { constraints: 'CSV only for this release', status: 'active' },
        },
        { revision: 1, snapshot: { constraints: 'Existing permissions', status: 'draft' } },
      ],
    });
  });

  it('refuses stale edits across connections without adding a revision or changing another project', () => {
    const file = path.join(mkdtempSync(path.join(tmpdir(), 'intent-concurrent-')), 'work.sqlite');
    const first = open(file),
      second = open(file);
    const a = create(first.store),
      b = create(first.store, path.resolve('/other-project'));
    second.store.request({
      action: 'update',
      id: a.id,
      expectedRevision: 1,
      fields: { ...fields, title: 'Current title' },
      reason: 'Review',
    });
    expect(() =>
      first.store.request({
        action: 'update',
        id: a.id,
        expectedRevision: 1,
        fields: { ...fields, title: 'Stale title' },
        reason: '',
      }),
    ).toThrow('changed elsewhere');
    const history = first.store.request({ action: 'history', id: a.id });
    expect(history.action === 'history' && history.revisions).toHaveLength(2);
    expect(first.store.request({ action: 'history', id: b.id })).toMatchObject({
      revisions: [{ revision: 1, snapshot: { title: fields.title } }],
    });
  });

  it('rolls back current state when recording its revision fails', () => {
    const { db, store } = open();
    const workspace = create(store);
    db.exec(
      "CREATE TRIGGER reject_revision BEFORE INSERT ON intent_revisions BEGIN SELECT RAISE(ABORT, 'disk failure fixture'); END",
    );
    expect(() =>
      store.request({
        action: 'update',
        id: workspace.id,
        expectedRevision: 1,
        fields: { ...fields, title: 'Must roll back' },
        reason: '',
      }),
    ).toThrow('disk failure fixture');
    expect(store.request({ action: 'list' })).toMatchObject({
      workspaces: [{ title: fields.title, revision: 1 }],
    });
  });

  it.each([
    { action: 'create', projectRoot: 'relative', fields },
    { action: 'create', projectRoot: path.resolve('/project'), fields: { ...fields, title: ' ' } },
    {
      action: 'create',
      projectRoot: path.resolve('/project'),
      fields: { ...fields, sourceUrl: 'javascript:alert(1)' },
    },
    {
      action: 'create',
      projectRoot: path.resolve('/project'),
      fields: { ...fields, sourceUrl: 'https://user:secret@example.com' },
    },
    {
      action: 'create',
      projectRoot: path.resolve('/project'),
      fields: { ...fields, status: 'deleted' },
    },
    { action: 'delete', id: 'anything' },
    null,
  ])('rejects malformed requests without writing state: %j', (input) => {
    const { store } = open();
    expect(() => store.request(input)).toThrow();
    expect(store.request({ action: 'list' })).toEqual({
      action: 'list',
      workspaces: [],
      projects: [],
      executionIndex: {},
    });
  });

  it('refuses a newer schema without rewriting its version', () => {
    const db = new DatabaseSync(':memory:');
    opened.push(db);
    db.exec(`PRAGMA user_version=${INTENT_WORKSPACE_SCHEMA_VERSION + 1}`);
    expect(() => new IntentWorkspaceStore(db)).toThrow('newer Workspacer');
    expect(db.prepare('PRAGMA user_version').get()?.user_version).toBe(
      INTENT_WORKSPACE_SCHEMA_VERSION + 1,
    );
  });
});
