import { createHash, randomUUID } from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import type { DatabaseSync } from 'node:sqlite';
import {
  openIntentArtifactFile,
  readIntentFile,
  writeIntentFile,
  removeIntentFile,
} from './intentArtifactFiles';
import { intentCriteria } from '../shared/intentEvidence';
import type { IntentWorkspace } from '../shared/intentWorkspace';
import {
  INTENT_ARTIFACT_LIMITS as LIMIT,
  INTENT_ARTIFACT_MIMES,
  type IntentArtifact,
  type IntentArtifactMime,
  type IntentAnnotation,
  type IntentDemonstration,
  type IntentAlternative,
  type IntentAlternativeGroup,
  type IntentAlternativeSelection,
  type IntentArtifactResponse,
} from '../shared/intentArtifacts';

const TABLES = [
  'intent_artifacts',
  'intent_annotations',
  'intent_demonstrations',
  'intent_alternative_groups',
  'intent_alternative_selections',
] as const;
type Table = (typeof TABLES)[number];
type RecordType =
  | IntentArtifact
  | IntentAnnotation
  | IntentDemonstration
  | IntentAlternativeGroup
  | IntentAlternativeSelection;
export const INTENT_ARTIFACT_SCHEMA = TABLES.map(
  (table) => `
CREATE TABLE IF NOT EXISTS ${table} (
 id TEXT PRIMARY KEY, workspace_id TEXT NOT NULL, intent_revision INTEGER NOT NULL,
 snapshot TEXT NOT NULL, request_json TEXT NOT NULL,
 FOREIGN KEY(workspace_id, intent_revision) REFERENCES intent_revisions(workspace_id, revision)
);
CREATE INDEX IF NOT EXISTS ${table}_workspace ON ${table}(workspace_id);
`,
).join('\n');
const hash = (bytes: string | Buffer) => createHash('sha256').update(bytes).digest('hex');
function text(value: unknown, label: string, max = 128): string {
  if (typeof value !== 'string' || !value.trim() || value.length > max || value.includes('\0'))
    throw new Error(`Invalid ${label}`);
  return value.trim();
}
function optional(value: unknown, label: string): string | undefined {
  return value === undefined ? undefined : text(value, label);
}
function record(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value))
    throw new Error('Invalid artifact record');
  return value as Record<string, unknown>;
}
function ids(value: unknown, label: string, max = 128): string[] {
  if (!Array.isArray(value) || value.length > max) throw new Error(`Invalid ${label}`);
  return [...new Set(value.map((v) => text(v, label)))];
}
function upload(value: unknown, mime: IntentArtifactMime): Buffer {
  if (
    typeof value !== 'string' ||
    !value.length ||
    value.length > Math.ceil(LIMIT.bytes / 3) * 4 ||
    !/^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$/.test(value)
  )
    throw new Error('Invalid or oversized artifact upload');
  const bytes = Buffer.from(value, 'base64');
  if (!bytes.length || bytes.length > LIMIT.bytes || bytes.toString('base64') !== value)
    throw new Error('Invalid or oversized artifact bytes');
  const signatures: Record<string, boolean> = {
    'image/png': bytes.subarray(0, 8).equals(Buffer.from('89504e470d0a1a0a', 'hex')),
    'image/jpeg': bytes.subarray(0, 3).equals(Buffer.from('ffd8ff', 'hex')),
    'image/gif': ['GIF87a', 'GIF89a'].includes(bytes.subarray(0, 6).toString('ascii')),
    'image/webp':
      bytes.subarray(0, 4).toString('ascii') === 'RIFF' &&
      bytes.subarray(8, 12).toString('ascii') === 'WEBP',
  };
  if (mime.startsWith('image/') && !signatures[mime])
    throw new Error('Artifact image signature does not match its media type');
  if (
    mime.startsWith('text/') &&
    (bytes.includes(0) || !Buffer.from(bytes.toString('utf8')).equals(bytes))
  )
    throw new Error('Text artifact must contain valid UTF-8 without NUL bytes');
  return bytes;
}

/** Only supplied upload bytes or URL references are accepted; never renderer filesystem paths. */
export class IntentArtifactStore {
  constructor(
    private readonly db: DatabaseSync,
    private readonly options: { artifactDirectory?: string } = {},
  ) {}
  private transaction<T>(run: () => T): T {
    this.db.exec('BEGIN IMMEDIATE');
    try {
      const result = run();
      this.db.exec('COMMIT');
      return result;
    } catch (error) {
      this.db.exec('ROLLBACK');
      throw error;
    }
  }
  private workspace(id: string, revision?: unknown): IntentWorkspace {
    const row = this.db.prepare('SELECT snapshot FROM intent_workspaces WHERE id=?').get(id);
    if (!row) throw new Error('Workspace no longer exists');
    const workspace = JSON.parse(String(row.snapshot)) as IntentWorkspace;
    if (revision !== undefined && revision !== workspace.revision)
      throw new Error('Intent changed elsewhere. Reload before saving artifacts or decisions.');
    return workspace;
  }
  private get<T extends RecordType>(table: Table, id: string, workspaceId: string): T {
    const row = this.db
      .prepare(`SELECT snapshot FROM ${table} WHERE id=? AND workspace_id=?`)
      .get(id, workspaceId);
    if (!row) throw new Error('Artifact or decision does not belong to this workspace');
    return JSON.parse(String(row.snapshot));
  }
  private all<T extends RecordType>(table: Table, id: string): T[] {
    return this.db
      .prepare(`SELECT snapshot FROM ${table} WHERE workspace_id=? ORDER BY rowid DESC`)
      .all(id)
      .map((r) => JSON.parse(String(r.snapshot)));
  }
  private existing<T extends RecordType>(table: Table, id: string, key: string): T | undefined {
    const row = this.db.prepare(`SELECT snapshot, request_json FROM ${table} WHERE id=?`).get(id);
    if (!row) return undefined;
    if (row.request_json !== key)
      throw new Error('Record ID was already used for different content');
    return JSON.parse(String(row.snapshot));
  }
  private insert<T extends RecordType>(table: Table, value: T, key: string): T {
    return this.transaction(() => {
      const existing = this.existing<T>(table, value.id, key);
      if (existing) return existing;
      this.workspace(value.workspaceId, value.intentRevision);
      if (table === 'intent_alternative_selections') {
        const selection = value as IntentAlternativeSelection;
        const latest = this.all<IntentAlternativeSelection>(table, value.workspaceId).find(
          (row) => row.groupId === selection.groupId,
        );
        if (latest?.id !== selection.previousSelectionId)
          throw new Error('Alternative selection changed elsewhere. Reload before choosing again.');
      }
      const count = Number(
        this.db
          .prepare(`SELECT count(*) AS n FROM ${table} WHERE workspace_id=?`)
          .get(value.workspaceId)?.n,
      );
      if (count >= (table === 'intent_annotations' ? LIMIT.annotations : LIMIT.records))
        throw new Error('Workspace artifact record limit reached');
      this.db
        .prepare(`INSERT INTO ${table} VALUES (?, ?, ?, ?, ?)`)
        .run(value.id, value.workspaceId, value.intentRevision, JSON.stringify(value), key);
      return value;
    });
  }
  private currentArtifact(id: string, workspace: IntentWorkspace): IntentArtifact {
    const artifact = this.get<IntentArtifact>('intent_artifacts', id, workspace.id);
    if (artifact.intentRevision !== workspace.revision)
      throw new Error('Use an artifact version from the current intent revision');
    return artifact;
  }
  private execution(id: string, workspaceId: string): void {
    if (
      !this.db
        .prepare('SELECT id FROM intent_executions WHERE id=? AND workspace_id=?')
        .get(id, workspaceId)
    )
      throw new Error('Execution does not belong to this workspace');
  }
  private directory(): string {
    const file = this.db
      .prepare('PRAGMA database_list')
      .all()
      .find((r) => r.name === 'main')?.file;
    if (!file && !this.options.artifactDirectory)
      throw new Error('Artifacts require persistent host storage');
    const directory =
      this.options.artifactDirectory ??
      path.join(fs.realpathSync(path.dirname(String(file))), 'intent-artifacts');
    return directory;
  }
  private read(artifact: IntentArtifact): string {
    if (!artifact.contentId) return '';
    if (!Number.isSafeInteger(artifact.bytes) || artifact.bytes < 1 || artifact.bytes > LIMIT.bytes)
      throw new Error('Invalid saved artifact size');
    if (process.platform === 'win32') {
      const bytes = readIntentFile(this.directory(), `${artifact.contentId}.bin`, LIMIT.bytes);
      if (bytes.length !== artifact.bytes || hash(bytes) !== artifact.sha256)
        throw new Error('Artifact integrity check failed');
      return bytes.toString('base64');
    }
    const file = openIntentArtifactFile(this.directory(), `${artifact.contentId}.bin`);
    const fd = file.fd;
    try {
      const stat = fs.fstatSync(fd);
      if (!stat.isFile() || stat.size !== artifact.bytes)
        throw new Error('Artifact bytes changed or are unavailable');
      const buffer = Buffer.alloc(artifact.bytes + 1);
      let length = 0;
      while (length < buffer.length) {
        const n = fs.readSync(fd, buffer, length, buffer.length - length, null);
        if (!n) break;
        length += n;
      }
      const bytes = buffer.subarray(0, length);
      if (length !== artifact.bytes || hash(bytes) !== artifact.sha256)
        throw new Error('Artifact integrity check failed');
      return bytes.toString('base64');
    } finally {
      file.close();
    }
  }
  request(input: Record<string, unknown>): IntentArtifactResponse {
    const id = text(input.id, 'workspace ID');
    let written: Pick<ReturnType<typeof openIntentArtifactFile>, 'remove' | 'close'> | undefined;
    try {
      return (() => {
        const base = this.workspace(id);
        if (input.action === 'artifacts')
          return {
            action: 'artifacts',
            artifacts: this.all('intent_artifacts', id),
            annotations: this.all('intent_annotations', id),
            demonstrations: this.all('intent_demonstrations', id),
            groups: this.all('intent_alternative_groups', id),
            selections: this.all('intent_alternative_selections', id),
          };
        if (input.action === 'readArtifact') {
          const artifact = this.get<IntentArtifact>(
            'intent_artifacts',
            text(input.artifactId, 'artifact ID'),
            id,
          );
          return { action: 'readArtifact', artifact, dataBase64: this.read(artifact) };
        }
        if (!Number.isSafeInteger(input.expectedRevision) || Number(input.expectedRevision) < 1)
          throw new Error('Invalid saved intent revision');
        const current = () => this.workspace(id, input.expectedRevision);
        const stamp = () => ({
          workspaceId: id,
          intentRevision: base.revision,
          author: 'user' as const,
          createdAt: new Date().toISOString(),
        });
        if (input.action === 'addArtifact') {
          const artifactId = text(input.artifactId, 'artifact ID');
          const title = text(input.title, 'artifact title', 240);
          const versionOf = optional(input.versionOf, 'prior artifact ID');
          const executionId = optional(input.executionId, 'execution ID');
          const criterionId = optional(input.criterionId, 'criterion ID');
          let url: string | undefined;
          let bytes: Buffer | undefined;
          let mimeType: IntentArtifactMime | undefined;
          if (input.url !== undefined) {
            url = text(input.url, 'artifact URL', 4096);
            let parsed: URL;
            try {
              parsed = new URL(url);
            } catch {
              throw new Error('Artifact URL must be HTTP or HTTPS');
            }
            if (
              !['http:', 'https:'].includes(parsed.protocol) ||
              parsed.username ||
              parsed.password
            )
              throw new Error('Artifact URL must be HTTP or HTTPS without credentials');
            if (input.dataBase64 !== undefined || input.mimeType !== undefined)
              throw new Error('Choose uploaded bytes or a URL reference');
          } else {
            if (!INTENT_ARTIFACT_MIMES.includes(input.mimeType as IntentArtifactMime))
              throw new Error('Unsupported artifact media type');
            mimeType = input.mimeType as IntentArtifactMime;
            bytes = upload(input.dataBase64, mimeType);
          }
          const sha256 = hash(bytes ?? url!);
          const key = JSON.stringify({
            action: input.action,
            id,
            expectedRevision: input.expectedRevision,
            artifactId,
            title,
            versionOf,
            executionId,
            criterionId,
            mimeType,
            url,
            sha256,
          });
          const prior = this.existing<IntentArtifact>('intent_artifacts', artifactId, key);
          if (prior) return { action: 'addArtifact', artifact: prior };
          const workspace = current();
          if (versionOf) this.get('intent_artifacts', versionOf, id);
          if (executionId) this.execution(executionId, id);
          const criterion = criterionId
            ? intentCriteria(workspace.revision, workspace.successCriteria).find(
                (c) => c.id === criterionId,
              )
            : undefined;
          if (criterionId && !criterion)
            throw new Error('Select a criterion from the saved revision');
          const contentId = bytes ? randomUUID() : undefined;
          const artifact: IntentArtifact = {
            ...stamp(),
            id: artifactId,
            title,
            kind: url
              ? 'url'
              : mimeType!.startsWith('image/')
                ? 'image'
                : mimeType === 'text/html'
                  ? 'html'
                  : 'text',
            bytes: bytes?.length ?? 0,
            sha256,
            ...(contentId ? { contentId, mimeType } : { url }),
            ...(versionOf ? { versionOf } : {}),
            ...(executionId ? { executionId } : {}),
            ...(criterion ? { criterion } : {}),
          };
          if (bytes && contentId) {
            if (process.platform === 'win32') {
              writeIntentFile(this.directory(), `${contentId}.bin`, bytes);
              written = {
                remove: () => removeIntentFile(this.directory(), `${contentId}.bin`, sha256),
                close: () => {},
              };
            } else {
              const file = openIntentArtifactFile(this.directory(), `${contentId}.bin`, true);
              written = file;
              fs.writeFileSync(file.fd, bytes);
              fs.fsyncSync(file.fd);
            }
          }
          const saved = this.insert('intent_artifacts', artifact, key);
          if (written && saved.contentId !== artifact.contentId) {
            try {
              written.remove();
            } catch {
              /* Unreferenced loser bytes are safe to retain if cleanup fails. */
            }
          }
          return { action: 'addArtifact', artifact: saved };
        }
        if (input.action === 'annotateArtifact') {
          const annotationId = text(input.annotationId, 'annotation ID');
          const artifactId = text(input.artifactId, 'artifact ID');
          const artifactSha256 = text(input.artifactSha256, 'artifact digest');
          const note = text(input.text, 'annotation', 8000);
          let point: IntentAnnotation['point'];
          if (input.point !== undefined) {
            const p = record(input.point);
            if (
              typeof p.x !== 'number' ||
              typeof p.y !== 'number' ||
              !Number.isFinite(p.x) ||
              !Number.isFinite(p.y) ||
              p.x < 0 ||
              p.y < 0 ||
              p.x > 1 ||
              p.y > 1
            )
              throw new Error('Annotation coordinates must be between zero and one');
            point = { x: p.x, y: p.y };
          }
          const key = JSON.stringify({
            action: input.action,
            id,
            expectedRevision: input.expectedRevision,
            annotationId,
            artifactId,
            artifactSha256,
            text: note,
            point,
          });
          const prior = this.existing<IntentAnnotation>('intent_annotations', annotationId, key);
          if (prior) return { action: 'annotateArtifact', annotation: prior };
          const artifact = this.currentArtifact(artifactId, current());
          if (artifact.sha256 !== artifactSha256)
            throw new Error('Artifact version changed; reload before annotating');
          if (point && artifact.kind !== 'image')
            throw new Error('Point annotations require a screenshot or image');
          if (artifact.contentId) this.read(artifact);
          const annotation: IntentAnnotation = {
            ...stamp(),
            id: annotationId,
            artifactId,
            artifactSha256,
            text: note,
            ...(point ? { point } : {}),
          };
          return {
            action: 'annotateArtifact',
            annotation: this.insert('intent_annotations', annotation, key),
          };
        }
        if (input.action === 'createDemonstration') {
          const demonstrationId = text(input.demonstrationId, 'demonstration ID');
          const title = text(input.title, 'demonstration title', 240);
          if (
            !Array.isArray(input.steps) ||
            !input.steps.length ||
            input.steps.length > LIMIT.demonstrationSteps
          )
            throw new Error('Demonstration requires 1–24 ordered screenshots');
          const steps = input.steps.map((v) => {
            const step = record(v);
            return {
              artifactId: text(step.artifactId, 'step artifact ID'),
              caption: text(step.caption, 'step caption', 2000),
            };
          });
          const key = JSON.stringify({
            action: input.action,
            id,
            expectedRevision: input.expectedRevision,
            demonstrationId,
            title,
            steps,
          });
          const prior = this.existing<IntentDemonstration>(
            'intent_demonstrations',
            demonstrationId,
            key,
          );
          if (prior) return { action: 'createDemonstration', demonstration: prior };
          const workspace = current();
          const pinned = steps.map((step) => {
            const artifact = this.currentArtifact(step.artifactId, workspace);
            if (artifact.kind !== 'image')
              throw new Error('Demonstration steps must be screenshots or images');
            this.read(artifact);
            return { ...step, sha256: artifact.sha256 };
          });
          const demonstration: IntentDemonstration = {
            ...stamp(),
            id: demonstrationId,
            title,
            steps: pinned,
          };
          return {
            action: 'createDemonstration',
            demonstration: this.insert('intent_demonstrations', demonstration, key),
          };
        }
        if (input.action === 'createAlternativeGroup') {
          const groupId = text(input.groupId, 'alternative group ID');
          const title = text(input.title, 'group title', 240);
          const purpose = text(input.purpose, 'exploration purpose', 4000);
          if (
            !Number.isSafeInteger(input.budgetMinutes) ||
            Number(input.budgetMinutes) < 1 ||
            Number(input.budgetMinutes) > 1440
          )
            throw new Error('Exploration budget must be 1–1440 minutes');
          if (
            !Array.isArray(input.alternatives) ||
            input.alternatives.length < 2 ||
            input.alternatives.length > LIMIT.alternatives
          )
            throw new Error('Record between two and six alternatives');
          const alternatives: IntentAlternative[] = input.alternatives.map((v) => {
            const a = record(v);
            return {
              id: text(a.id, 'alternative ID'),
              title: text(a.title, 'alternative title', 240),
              hypothesis: text(a.hypothesis, 'hypothesis', 4000),
              artifactIds: ids(a.artifactIds, 'alternative artifact IDs'),
              executionIds: ids(a.executionIds, 'alternative execution IDs'),
            };
          });
          if (new Set(alternatives.map((a) => a.id)).size !== alternatives.length)
            throw new Error('Alternative IDs must be distinct');
          const key = JSON.stringify({
            action: input.action,
            id,
            expectedRevision: input.expectedRevision,
            groupId,
            title,
            purpose,
            budgetMinutes: input.budgetMinutes,
            alternatives,
          });
          const prior = this.existing<IntentAlternativeGroup>(
            'intent_alternative_groups',
            groupId,
            key,
          );
          if (prior) return { action: 'createAlternativeGroup', group: prior };
          const workspace = current();
          for (const alternative of alternatives) {
            for (const aid of alternative.artifactIds) this.currentArtifact(aid, workspace);
            for (const eid of alternative.executionIds) this.execution(eid, id);
          }
          const group: IntentAlternativeGroup = {
            ...stamp(),
            id: groupId,
            title,
            purpose,
            budgetMinutes: Number(input.budgetMinutes),
            alternatives,
          };
          return {
            action: 'createAlternativeGroup',
            group: this.insert('intent_alternative_groups', group, key),
          };
        }
        if (input.action === 'selectAlternative') {
          const selectionId = text(input.selectionId, 'selection ID');
          const groupId = text(input.groupId, 'group ID');
          const alternativeId = text(input.alternativeId, 'alternative ID');
          const reason = text(input.reason, 'selection reason', 8000);
          const previousSelectionId = optional(input.expectedSelectionId, 'previous selection ID');
          const key = JSON.stringify({
            action: input.action,
            id,
            expectedRevision: input.expectedRevision,
            selectionId,
            groupId,
            alternativeId,
            reason,
            previousSelectionId,
          });
          const prior = this.existing<IntentAlternativeSelection>(
            'intent_alternative_selections',
            selectionId,
            key,
          );
          if (prior) return { action: 'selectAlternative', selection: prior };
          const workspace = current();
          const group = this.get<IntentAlternativeGroup>('intent_alternative_groups', groupId, id);
          if (group.intentRevision !== workspace.revision)
            throw new Error('Alternative group belongs to an earlier intent revision');
          if (!group.alternatives.some((a) => a.id === alternativeId))
            throw new Error('Alternative does not belong to this group');
          const latest = this.all<IntentAlternativeSelection>(
            'intent_alternative_selections',
            id,
          ).find((s) => s.groupId === groupId);
          if (latest?.id !== previousSelectionId)
            throw new Error(
              'Alternative selection changed elsewhere. Reload before choosing again.',
            );
          const selection: IntentAlternativeSelection = {
            ...stamp(),
            id: selectionId,
            groupId,
            alternativeId,
            reason,
            ...(previousSelectionId ? { previousSelectionId } : {}),
          };
          return {
            action: 'selectAlternative',
            selection: this.insert('intent_alternative_selections', selection, key),
          };
        }
        throw new Error('Unknown artifact action');
      })();
    } catch (error) {
      if (written) {
        try {
          written.remove();
        } catch {
          /* Unreferenced bytes can be cleaned up later. */
        }
      }
      throw error;
    } finally {
      written?.close();
    }
  }
}
