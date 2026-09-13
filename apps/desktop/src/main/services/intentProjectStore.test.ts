import { DatabaseSync } from 'node:sqlite';
import { mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { afterEach, expect, it, vi } from 'vitest';
vi.mock('./configService', () => ({ getConfigDir: () => '/unused-intent-project-tests' }));
import { IntentWorkspaceStore } from './intentWorkspaceStore';
import { INTENT_PROJECT_SCHEMA, IntentProjectStore } from './intentProjectStore';

const opened: DatabaseSync[] = [];
function open(file = ':memory:') {
  const db = new DatabaseSync(file);
  opened.push(db);
  const work = new IntentWorkspaceStore(db);
  db.exec(INTENT_PROJECT_SCHEMA);
  const projects = new IntentProjectStore(db);
  const create = (root: string) => {
    const response = work.request({
      action: 'create',
      projectRoot: root,
      fields: {
        title: 'Feature',
        outcome: 'Outcome',
        constraints: '',
        successCriteria: 'Checks pass',
        sourceUrl: '',
        status: 'active',
      },
    });
    if (response.action !== 'create') throw new Error('Expected workspace');
    return response.workspace;
  };
  const list = () => {
    const result = projects.request({ action: 'projects' });
    if (result.action !== 'projects') throw new Error('Expected projects');
    return result.projects;
  };
  return { db, work, projects, create, list };
}
afterEach(() => {
  for (const db of opened.splice(0)) if (db.isOpen) db.close();
});
it('backfills stable project/repository IDs for shared roots without rewriting prior revisions or packets, and survives reopen', () => {
  const file = path.join(mkdtempSync(path.join(tmpdir(), 'intent-projects-')), 'work.sqlite');
  const f = open(file);
  const first = f.create('/project'),
    second = f.create('/project'),
    other = f.create('/other');
  f.work.request({
    action: 'prepareExecution',
    id: first.id,
    executionId: 'run',
    expectedRevision: 1,
    task: 'Implement',
  });
  // Simulate current snapshots written before identity existed, even after the
  // central migration starts assigning IDs on creation.
  f.db.exec(
    "UPDATE intent_workspaces SET snapshot=json_remove(snapshot, '$.projectId', '$.repositoryId')",
  );
  const history = f.db.prepare('SELECT * FROM intent_revisions ORDER BY workspace_id').all();
  const executions = f.db.prepare('SELECT * FROM intent_executions').all();
  f.projects.backfill();
  f.projects.backfill();
  const identity = f.projects.ensureRepository('/project/');
  const current = f.work.request({ action: 'list' });
  expect(current).toMatchObject({
    workspaces: expect.arrayContaining([
      { ...first, ...identity },
      { ...second, ...identity },
    ]),
  });
  expect(f.list()).toHaveLength(2);
  expect(f.projects.ensureRepository('/other').projectId).not.toBe(identity.projectId);
  expect(f.db.prepare('SELECT * FROM intent_revisions ORDER BY workspace_id').all()).toEqual(
    history,
  );
  expect(f.db.prepare('SELECT * FROM intent_executions').all()).toEqual(executions);
  f.db.close();
  const reopened = open(file);
  expect(reopened.projects.ensureRepository('/project')).toEqual(identity);
  expect(reopened.list()).toHaveLength(2);
  expect(reopened.work.request({ action: 'history', id: other.id })).toMatchObject({
    revisions: [{ revision: 1 }],
  });
});
it('adds a second repository to one stable project, reuses mappings and CAS-fences cross-connection changes', () => {
  const file = path.join(mkdtempSync(path.join(tmpdir(), 'intent-project-cas-')), 'work.sqlite');
  const f = open(file),
    other = open(file);
  const identity = f.projects.ensureRepository('/project');
  const add = f.projects.request({
    action: 'addIntentRepository',
    projectId: identity.projectId,
    expectedRevision: 1,
    root: '/project-api',
  });
  expect(add).toMatchObject({
    project: {
      revision: 2,
      repositories: expect.arrayContaining([
        expect.objectContaining({ root: '/project' }),
        expect.objectContaining({ root: '/project-api' }),
      ]),
    },
  });
  expect(f.projects.ensureRepository('/project-api').projectId).toBe(identity.projectId);
  expect(() =>
    other.projects.request({
      action: 'renameIntentProject',
      projectId: identity.projectId,
      expectedRevision: 1,
      name: 'Stale',
    }),
  ).toThrow('changed elsewhere');
  expect(
    f.projects.request({
      action: 'renameIntentProject',
      projectId: identity.projectId,
      expectedRevision: 2,
      name: 'Whole product',
    }),
  ).toMatchObject({ project: { id: identity.projectId, name: 'Whole product', revision: 3 } });
  const foreign = f.projects.ensureRepository('/elsewhere');
  expect(() =>
    f.projects.request({
      action: 'addIntentRepository',
      projectId: foreign.projectId,
      expectedRevision: 1,
      root: '/project-api',
    }),
  ).toThrow('another intent project');
});
it('relocates all current work in one repository atomically, advances intent revisions, and preserves sibling roots, executions and history', () => {
  const f = open();
  const first = f.create('/project'),
    second = f.create('/project');
  f.projects.backfill();
  const identity = f.projects.ensureRepository('/project');
  f.projects.request({
    action: 'addIntentRepository',
    projectId: identity.projectId,
    expectedRevision: 1,
    root: '/api',
  });
  const sibling = f.create('/api');
  f.projects.backfill();
  const launch = f.work.request({
    action: 'prepareExecution',
    id: first.id,
    executionId: 'launch',
    expectedRevision: 1,
    task: 'Implement',
  });
  f.work.request({
    action: 'linkExecution',
    id: first.id,
    executionId: 'launch',
    session: {
      sessionId: 'worker',
      hub: '',
      label: 'Worker',
      provider: 'codex',
      cwd: '/project/worktree',
    },
  });
  const before = f.db.prepare('SELECT * FROM intent_executions').all();
  const relocation = f.projects.request({
    action: 'relocateIntentRepository',
    projectId: identity.projectId,
    repositoryId: identity.repositoryId,
    expectedRevision: 2,
    root: '/moved',
    reason: 'Moved checkout',
  });
  expect(relocation).toMatchObject({
    project: { id: identity.projectId, revision: 3 },
    workspaces: expect.arrayContaining([
      expect.objectContaining({ id: first.id, projectRoot: '/moved', revision: 2 }),
      expect.objectContaining({ id: second.id, projectRoot: '/moved', revision: 2 }),
    ]),
  });
  expect(f.projects.ensureRepository('/moved')).toEqual(identity);
  expect(f.work.request({ action: 'history', id: first.id })).toMatchObject({
    revisions: [
      {
        revision: 2,
        reason: expect.stringContaining('Moved checkout'),
        snapshot: { projectRoot: '/moved' },
      },
      { revision: 1, snapshot: { projectRoot: '/project' } },
    ],
  });
  expect(f.work.request({ action: 'history', id: sibling.id })).toMatchObject({
    revisions: [{ revision: 1, snapshot: { projectRoot: '/api' } }],
  });
  expect(f.db.prepare('SELECT * FROM intent_executions').all()).toEqual(before);
  expect(launch).toMatchObject({
    execution: { contextPacket: expect.stringContaining('Project directory: /project') },
  });
  expect(() =>
    f.work.request({
      action: 'update',
      id: first.id,
      expectedRevision: 1,
      fields: first,
      reason: '',
    }),
  ).toThrow('changed elsewhere');
});
it('rolls back every root and revision write on a failed relocation, including the mapping', () => {
  const f = open();
  const first = f.create('/project');
  f.create('/project');
  f.projects.backfill();
  const identity = f.projects.ensureRepository('/project');
  const workBefore = f.db.prepare('SELECT * FROM intent_workspaces ORDER BY id').all();
  const revisionsBefore = f.db
    .prepare('SELECT * FROM intent_revisions ORDER BY workspace_id')
    .all();
  f.db.exec(
    "CREATE TRIGGER fail_revision BEFORE INSERT ON intent_revisions BEGIN SELECT RAISE(ABORT, 'disk full'); END",
  );
  expect(() =>
    f.projects.request({
      action: 'relocateIntentRepository',
      projectId: identity.projectId,
      repositoryId: identity.repositoryId,
      expectedRevision: 1,
      root: '/moved',
      reason: 'Move',
    }),
  ).toThrow('disk full');
  expect(f.db.prepare('SELECT * FROM intent_workspaces ORDER BY id').all()).toEqual(workBefore);
  expect(f.db.prepare('SELECT * FROM intent_revisions ORDER BY workspace_id').all()).toEqual(
    revisionsBefore,
  );
  expect(f.list()[0]).toMatchObject({
    revision: 1,
    repositories: [{ root: '/project', revision: 1 }],
  });
  expect(f.work.request({ action: 'history', id: first.id })).toMatchObject({
    revisions: [{ revision: 1 }],
  });
});
it('composes identity creation with a surrounding transaction and refuses invalid or foreign relocation targets', () => {
  const f = open();
  f.db.exec('BEGIN IMMEDIATE');
  f.projects.ensureRepository('/rolled-back');
  f.db.exec('ROLLBACK');
  expect(f.list()).toEqual([]);
  expect(() => f.projects.ensureRepository('relative/root')).toThrow('absolute path');
  const own = f.projects.ensureRepository('/project'),
    foreign = f.projects.ensureRepository('/other');
  const base = {
    action: 'relocateIntentRepository',
    projectId: own.projectId,
    repositoryId: own.repositoryId,
    expectedRevision: 1,
    root: '/moved',
    reason: 'Relocated',
  };
  expect(() => f.projects.request({ ...base, repositoryId: foreign.repositoryId })).toThrow(
    'does not belong',
  );
  expect(() => f.projects.request({ ...base, root: '/other' })).toThrow('another repository');
  expect(() => f.projects.request({ ...base, reason: '' })).toThrow('reason');
  expect(() => f.projects.request({ ...base, expectedRevision: 0 })).toThrow('revision');
  expect(f.list().find((p) => p.id === own.projectId)).toMatchObject({
    revision: 1,
    repositories: [{ root: '/project' }],
  });
});
