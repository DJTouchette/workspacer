import { randomUUID } from 'node:crypto';
import path from 'node:path';
import type { DatabaseSync } from 'node:sqlite';
import type {
  IntentProject,
  IntentProjectResponse,
  IntentRepository,
} from '../shared/intentProject';
import type { IntentWorkspace } from '../shared/intentWorkspace';

export const INTENT_PROJECT_SCHEMA = `
  CREATE TABLE IF NOT EXISTS intent_projects (
    id TEXT PRIMARY KEY, name TEXT NOT NULL, revision INTEGER NOT NULL,
    created_at TEXT NOT NULL, updated_at TEXT NOT NULL
  );
  CREATE TABLE IF NOT EXISTS intent_repositories (
    id TEXT PRIMARY KEY, project_id TEXT NOT NULL REFERENCES intent_projects(id),
    root TEXT NOT NULL UNIQUE, revision INTEGER NOT NULL,
    created_at TEXT NOT NULL, updated_at TEXT NOT NULL
  );
  CREATE INDEX IF NOT EXISTS intent_repository_project ON intent_repositories(project_id);
`;
type IdentifiedWorkspace = IntentWorkspace & { projectId?: string; repositoryId?: string };
function required(value: unknown, label: string, max = 128): string {
  if (typeof value !== 'string' || !value.trim() || value.length > max || value.includes('\0'))
    throw new Error(`Invalid ${label}`);
  return value.trim();
}
function rootPath(value: unknown): string {
  const root = required(value, 'repository root', 4096);
  if (!path.isAbsolute(root))
    throw new Error('Repository root must be an absolute path on the connected host');
  const normalized = path.normalize(root);
  // A trailing separator does not give one directory a second identity.
  return normalized.length > path.parse(normalized).root.length
    ? normalized.replace(path.sep === '\\' ? /[\\/]+$/ : /\/+$/, '')
    : normalized;
}

export class IntentProjectStore {
  constructor(private db: DatabaseSync) {}
  /** Savepoints compose with the central migration/create transaction. */
  private transaction<T>(run: () => T): T {
    this.db.exec('SAVEPOINT intent_project_change');
    try {
      const value = run();
      this.db.exec('RELEASE intent_project_change');
      return value;
    } catch (error) {
      this.db.exec('ROLLBACK TO intent_project_change; RELEASE intent_project_change');
      throw error;
    }
  }
  private project(id: string): IntentProject {
    const row = this.db.prepare('SELECT * FROM intent_projects WHERE id=?').get(id);
    if (!row) throw new Error('Intent project no longer exists');
    return {
      id,
      name: String(row.name),
      revision: Number(row.revision),
      createdAt: String(row.created_at),
      updatedAt: String(row.updated_at),
      repositories: this.db
        .prepare('SELECT * FROM intent_repositories WHERE project_id=? ORDER BY created_at,id')
        .all(id)
        .map((row) => ({
          id: String(row.id),
          projectId: id,
          root: String(row.root),
          revision: Number(row.revision),
          createdAt: String(row.created_at),
          updatedAt: String(row.updated_at),
        })),
    };
  }
  private ensure(root: string): { projectId: string; repositoryId: string } {
    const found = this.db
      .prepare('SELECT id,project_id FROM intent_repositories WHERE root=?')
      .get(root);
    if (found) return { projectId: String(found.project_id), repositoryId: String(found.id) };
    const now = new Date().toISOString(),
      projectId = randomUUID(),
      repositoryId = randomUUID();
    this.db
      .prepare('INSERT INTO intent_projects VALUES (?, ?, 1, ?, ?)')
      .run(projectId, path.basename(root) || root, now, now);
    this.db
      .prepare('INSERT INTO intent_repositories VALUES (?, ?, ?, 1, ?, ?)')
      .run(repositoryId, projectId, root, now, now);
    return { projectId, repositoryId };
  }
  ensureRepository(projectRoot: string): { projectId: string; repositoryId: string } {
    return this.transaction(() => this.ensure(rootPath(projectRoot)));
  }
  /** Annotate only current snapshots. Historical intent and dispatch packets stay immutable. */
  backfill(): void {
    this.transaction(() => {
      for (const row of this.db.prepare('SELECT id,snapshot FROM intent_workspaces').all()) {
        const workspace = JSON.parse(String(row.snapshot)) as IdentifiedWorkspace;
        if (workspace.projectId && workspace.repositoryId) continue;
        const identity = this.ensure(rootPath(workspace.projectRoot));
        this.db
          .prepare('UPDATE intent_workspaces SET snapshot=? WHERE id=?')
          .run(JSON.stringify({ ...workspace, ...identity }), row.id);
      }
    });
  }
  request(input: Record<string, unknown>): IntentProjectResponse {
    return this.transaction(() => {
      if (input.action === 'projects')
        return {
          action: 'projects',
          projects: this.db
            .prepare('SELECT id FROM intent_projects ORDER BY name,id')
            .all()
            .map((row) => this.project(String(row.id))),
        };
      const projectId = required(input.projectId, 'project ID');
      const project = this.project(projectId);
      if (!Number.isSafeInteger(input.expectedRevision) || Number(input.expectedRevision) < 1)
        throw new Error('Invalid expected project revision');
      if (input.expectedRevision !== project.revision)
        throw new Error(
          'Project changed elsewhere. Refresh before saving; your draft is preserved.',
        );
      const now = new Date().toISOString();
      const advance = (name = project.name) =>
        this.db
          .prepare('UPDATE intent_projects SET name=?,revision=revision+1,updated_at=? WHERE id=?')
          .run(name, now, projectId);
      if (input.action === 'renameIntentProject') {
        const name = required(input.name, 'project name', 240);
        if (name !== project.name) advance(name);
        return { action: 'renameIntentProject', project: this.project(projectId) };
      }
      if (input.action !== 'addIntentRepository' && input.action !== 'relocateIntentRepository')
        throw new Error('Unknown intent project action');
      const root = rootPath(input.root);
      const existing = this.db
        .prepare('SELECT id,project_id FROM intent_repositories WHERE root=?')
        .get(root);
      if (input.action === 'addIntentRepository') {
        if (existing) {
          if (existing.project_id !== projectId)
            throw new Error('This root already belongs to another intent project');
          return { action: 'addIntentRepository', project };
        }
        this.db
          .prepare('INSERT INTO intent_repositories VALUES (?, ?, ?, 1, ?, ?)')
          .run(randomUUID(), projectId, root, now, now);
        advance();
        return { action: 'addIntentRepository', project: this.project(projectId) };
      }
      const repositoryId = required(input.repositoryId, 'repository ID');
      const repository: IntentRepository | undefined = project.repositories.find(
        (repo) => repo.id === repositoryId,
      );
      if (!repository) throw new Error('Repository does not belong to this intent project');
      const reason = required(input.reason, 'relocation reason', 4000);
      if (existing && existing.id !== repositoryId)
        throw new Error('This root already belongs to another repository');
      if (repository.root === root)
        return { action: 'relocateIntentRepository', project, workspaces: [] };
      const workspaces: IntentWorkspace[] = [];
      for (const row of this.db
        .prepare(
          "SELECT snapshot FROM intent_workspaces WHERE json_extract(snapshot, '$.repositoryId')=?",
        )
        .all(repositoryId)) {
        const previous = JSON.parse(String(row.snapshot)) as IdentifiedWorkspace;
        if (previous.projectId !== projectId)
          throw new Error('Workspace project identity is inconsistent; relocation was refused');
        const workspace: IdentifiedWorkspace = {
          ...previous,
          projectRoot: root,
          revision: previous.revision + 1,
          updatedAt: now,
        };
        this.db
          .prepare(
            'UPDATE intent_workspaces SET project_root=?,revision=?,updated_at=?,snapshot=? WHERE id=?',
          )
          .run(root, workspace.revision, now, JSON.stringify(workspace), workspace.id);
        this.db
          .prepare('INSERT INTO intent_revisions VALUES (?, ?, ?, ?, ?)')
          .run(
            workspace.id,
            workspace.revision,
            now,
            `Repository root relocated from ${repository.root} to ${root}: ${reason}`,
            JSON.stringify(workspace),
          );
        workspaces.push(workspace);
      }
      this.db
        .prepare(
          'UPDATE intent_repositories SET root=?,revision=revision+1,updated_at=? WHERE id=?',
        )
        .run(root, now, repositoryId);
      advance();
      return { action: 'relocateIntentRepository', project: this.project(projectId), workspaces };
    });
  }
}
