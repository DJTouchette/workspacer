import { DatabaseSync } from 'node:sqlite';
import { mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { expect, it, vi } from 'vitest';
vi.mock('./configService', () => ({ getConfigDir: () => '/unused-intent-project-migration' }));
import { INTENT_WORKSPACE_SCHEMA_VERSION, IntentWorkspaceStore } from './intentWorkspaceStore';

it('migrates a real v4 database, preserving immutable roots/packets and creating new work in an existing multi-repository project', () => {
  const file = path.join(
    mkdtempSync(path.join(tmpdir(), 'intent-project-migration-')),
    'work.sqlite',
  );
  const db = new DatabaseSync(file);
  try {
    const work = {
      id: 'legacy',
      projectRoot: '/legacy-root',
      revision: 1,
      title: 'Legacy',
      outcome: 'Outcome',
      constraints: '',
      successCriteria: 'Check',
      sourceUrl: '',
      status: 'active',
      createdAt: '2026-09-12T12:00:00Z',
      updatedAt: '2026-09-12T12:00:00Z',
    };
    const snapshot = JSON.stringify(work);
    // v4 schema before intent project identity, matching the existing tables.
    db.exec(`CREATE TABLE intent_workspaces (id TEXT PRIMARY KEY, project_root TEXT NOT NULL, revision INTEGER NOT NULL, updated_at TEXT NOT NULL, snapshot TEXT NOT NULL);
      CREATE TABLE intent_revisions (workspace_id TEXT NOT NULL REFERENCES intent_workspaces(id), revision INTEGER NOT NULL, at TEXT NOT NULL, reason TEXT NOT NULL, snapshot TEXT NOT NULL, PRIMARY KEY(workspace_id,revision));
      CREATE TABLE intent_executions (id TEXT PRIMARY KEY, workspace_id TEXT NOT NULL, intent_revision INTEGER NOT NULL, snapshot TEXT NOT NULL, FOREIGN KEY(workspace_id,intent_revision) REFERENCES intent_revisions(workspace_id,revision));
      CREATE TABLE intent_work_links (id TEXT PRIMARY KEY, workspace_id TEXT NOT NULL REFERENCES intent_workspaces(id), kind TEXT NOT NULL, target TEXT NOT NULL, created_at TEXT NOT NULL, UNIQUE(workspace_id,kind,target));
      CREATE TABLE intent_directions (id TEXT PRIMARY KEY, workspace_id TEXT NOT NULL, execution_id TEXT NOT NULL REFERENCES intent_executions(id), intent_revision INTEGER NOT NULL, snapshot TEXT NOT NULL, FOREIGN KEY(workspace_id,intent_revision) REFERENCES intent_revisions(workspace_id,revision));
      CREATE TABLE intent_evidence (id TEXT PRIMARY KEY, workspace_id TEXT NOT NULL, intent_revision INTEGER NOT NULL, snapshot TEXT NOT NULL, request_json TEXT NOT NULL, FOREIGN KEY(workspace_id,intent_revision) REFERENCES intent_revisions(workspace_id,revision));
      CREATE TABLE intent_reviews (id TEXT PRIMARY KEY, workspace_id TEXT NOT NULL, intent_revision INTEGER NOT NULL, snapshot TEXT NOT NULL, request_json TEXT NOT NULL, FOREIGN KEY(workspace_id,intent_revision) REFERENCES intent_revisions(workspace_id,revision));
      CREATE TABLE intent_controls (id TEXT PRIMARY KEY, workspace_id TEXT NOT NULL REFERENCES intent_workspaces(id), execution_id TEXT NOT NULL REFERENCES intent_executions(id), intent_revision INTEGER NOT NULL, snapshot TEXT NOT NULL, FOREIGN KEY(workspace_id,intent_revision) REFERENCES intent_revisions(workspace_id,revision));
      PRAGMA user_version=4;`);
    db.prepare('INSERT INTO intent_workspaces VALUES (?,?,?,?,?)').run(
      work.id,
      work.projectRoot,
      1,
      work.updatedAt,
      snapshot,
    );
    db.prepare('INSERT INTO intent_revisions VALUES (?,?,?,?,?)').run(
      work.id,
      1,
      work.updatedAt,
      'Created',
      snapshot,
    );
    const packet = 'Immutable packet referencing /legacy-root';
    const execution = JSON.stringify({
      id: 'legacy-run',
      workspaceId: work.id,
      intentRevision: 1,
      kind: 'launch',
      state: 'linked',
      task: 'Implement',
      contextPacket: packet,
      session: {
        sessionId: 'worker',
        hub: '',
        label: 'Worker',
        provider: 'codex',
        cwd: '/legacy-root/worktree',
      },
      lastObservation: null,
      createdAt: work.createdAt,
      updatedAt: work.updatedAt,
    });
    db.prepare('INSERT INTO intent_executions VALUES (?,?,?,?)').run(
      'legacy-run',
      work.id,
      1,
      execution,
    );
    const store = new IntentWorkspaceStore(db);
    expect(db.prepare('PRAGMA user_version').get()?.user_version).toBe(
      INTENT_WORKSPACE_SCHEMA_VERSION,
    );
    const listed = store.request({ action: 'list' });
    if (listed.action !== 'list') throw new Error('Expected list');
    const current = listed.workspaces[0];
    expect(current).toMatchObject({
      ...work,
      projectId: expect.any(String),
      repositoryId: expect.any(String),
    });
    expect(db.prepare('SELECT snapshot FROM intent_revisions').get()?.snapshot).toBe(snapshot);
    expect(db.prepare('SELECT snapshot FROM intent_executions').get()?.snapshot).toBe(execution);
    const add = store.request({
      action: 'addIntentRepository',
      projectId: current.projectId,
      expectedRevision: 1,
      root: '/api-root',
    });
    expect(add).toMatchObject({ project: { id: current.projectId, revision: 2 } });
    const created = store.request({ action: 'create', projectRoot: '/api-root', fields: work });
    expect(created).toMatchObject({
      workspace: {
        projectId: current.projectId,
        repositoryId: expect.any(String),
        projectRoot: '/api-root',
      },
    });
    if (created.action !== 'create') throw new Error('Expected create');
    expect(created.workspace.repositoryId).not.toBe(current.repositoryId);
    db.close();
    const reopened = new DatabaseSync(file);
    try {
      expect(new IntentWorkspaceStore(reopened).request({ action: 'list' })).toMatchObject({
        workspaces: expect.arrayContaining([
          expect.objectContaining({
            id: current.id,
            projectId: current.projectId,
            repositoryId: current.repositoryId,
          }),
        ]),
      });
    } finally {
      reopened.close();
    }
  } finally {
    if (db.isOpen) db.close();
  }
});
