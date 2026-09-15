import type { DatabaseSync } from 'node:sqlite';
import {
  normalizeIntegration,
  resolveIntegration,
  type IntentIntegration,
  type IntentIntegrationResponse,
  type IntentIntegrationReference,
  type IntentIntegrationView,
} from '../shared/intentIntegrations';
import { sourceConnection, sourceText } from './intentSourceAdapters';

export const INTENT_INTEGRATION_SCHEMA = `
CREATE TABLE IF NOT EXISTS intent_integrations (id TEXT PRIMARY KEY, project_root TEXT NOT NULL, snapshot TEXT NOT NULL);
CREATE INDEX IF NOT EXISTS intent_integrations_project ON intent_integrations(project_root);
CREATE INDEX IF NOT EXISTS intent_sources_integration ON intent_sources(json_extract(snapshot, '$.integration.id'));
CREATE TABLE IF NOT EXISTS intent_integration_operations (id TEXT PRIMARY KEY, request TEXT NOT NULL, response TEXT NOT NULL);
`;
export class IntentIntegrationStore {
  constructor(private db: DatabaseSync) {}
  private project(id: string): string {
    const row = this.db.prepare('SELECT snapshot FROM intent_workspaces WHERE id=?').get(id);
    if (!row) throw new Error('Workspace no longer exists');
    return sourceText(JSON.parse(String(row.snapshot)).projectRoot, 'project directory', 4096);
  }
  get(id: string, workspaceId: string): IntentIntegration {
    const row = this.db
      .prepare('SELECT snapshot FROM intent_integrations WHERE id=? AND project_root=?')
      .get(id, this.project(workspaceId));
    if (!row) throw new Error('Integration does not belong to this project');
    return JSON.parse(String(row.snapshot));
  }
  list(workspaceId: string): IntentIntegrationView[] {
    return this.listProject(this.project(workspaceId));
  }
  listProject(projectRoot: string): IntentIntegrationView[] {
    return this.db
      .prepare('SELECT snapshot FROM intent_integrations WHERE project_root=? ORDER BY rowid')
      .all(projectRoot)
      .map((row) => JSON.parse(String(row.snapshot)) as IntentIntegration)
      .filter((c) => !c.deleted)
      .map((c) => ({ ...c, references: this.references(c.id) }));
  }
  private references(id: string): number {
    return Number(
      this.db
        .prepare(
          "SELECT count(*) AS count FROM intent_sources WHERE json_extract(snapshot, '$.integration.id')=?",
        )
        .get(id)?.count ?? 0,
    );
  }
  resolve(workspaceId: string, value: unknown) {
    const ref = value as IntentIntegrationReference | null;
    if (!ref) throw new Error('Integration reference is required');
    const integration = this.get(sourceText(ref.integrationId, 'integration ID'), workspaceId);
    if (integration.deleted || !integration.enabled)
      throw new Error('Integration is disabled or removed');
    if (integration.version !== ref.expectedIntegrationVersion)
      throw new Error('Integration changed. Reload and preview again.');
    return { integration, connection: sourceConnection(resolveIntegration(integration, ref)) };
  }
  request(input: Record<string, unknown>): IntentIntegrationResponse {
    const workspaceId = sourceText(input.id, 'workspace ID');
    const projectRoot = this.project(workspaceId);
    if (input.action === 'integrations')
      return { action: 'integrations', integrations: this.list(workspaceId) };
    if (input.action === 'previewSource')
      return {
        action: 'previewSource',
        connection: this.resolve(workspaceId, input.reference).connection,
      };
    const id = sourceText(input.integrationId, 'integration ID');
    const operationId = sourceText(input.operationId, 'operation ID');
    if (!Number.isSafeInteger(input.expectedVersion) || Number(input.expectedVersion) < 0)
      throw new Error('Invalid integration version');
    const draft =
      input.action === 'saveIntegration' ? normalizeIntegration(input.integration) : null;
    if (!['saveIntegration', 'removeIntegration'].includes(String(input.action)))
      throw new Error('Unknown integration action');
    const fingerprint = JSON.stringify({
      projectRoot,
      action: input.action,
      id,
      expectedVersion: input.expectedVersion,
      draft,
    });
    this.db.exec('BEGIN IMMEDIATE');
    try {
      const operation = this.db
        .prepare('SELECT request,response FROM intent_integration_operations WHERE id=?')
        .get(operationId);
      if (operation) {
        if (operation.request !== fingerprint)
          throw new Error('Operation ID was already used for a different request');
        this.db.exec('COMMIT');
        return JSON.parse(String(operation.response));
      }
      const row = this.db.prepare('SELECT snapshot FROM intent_integrations WHERE id=?').get(id);
      const old = row ? (JSON.parse(String(row.snapshot)) as IntentIntegration) : null;
      if (old && old.projectRoot !== projectRoot)
        throw new Error('Integration does not belong to this project');
      if ((old?.version ?? 0) !== input.expectedVersion || old?.deleted)
        throw new Error('Integration changed. Reload before continuing.');
      if (!draft && !old) throw new Error('Integration no longer exists');
      if (!draft && this.references(id))
        throw new Error(
          'Integration is referenced by sources. Disable it instead; existing sources stay pinned.',
        );
      const integration: IntentIntegration = {
        ...(draft ?? old!),
        id,
        projectRoot,
        version: (old?.version ?? 0) + 1,
        deleted: !draft,
        enabled: draft?.enabled ?? false,
      };
      this.db
        .prepare(
          'INSERT INTO intent_integrations VALUES(?,?,?) ON CONFLICT(id) DO UPDATE SET snapshot=excluded.snapshot',
        )
        .run(id, projectRoot, JSON.stringify(integration));
      const response: IntentIntegrationResponse = {
        action: input.action as 'saveIntegration' | 'removeIntegration',
        integration,
      };
      this.db
        .prepare('INSERT INTO intent_integration_operations VALUES(?,?,?)')
        .run(operationId, fingerprint, JSON.stringify(response));
      this.db.exec('COMMIT');
      return response;
    } catch (error) {
      this.db.exec('ROLLBACK');
      throw error;
    }
  }
}
