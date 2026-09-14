import type { IntentRun } from '../shared/intentAutomation';
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
import {
  intentCriteria,
  type IntentCriterion,
  type IntentEvidence,
  type IntentEvidenceResponse,
  type IntentReview,
} from '../shared/intentEvidence';
import type {
  IntentExecution,
  IntentLiveSession,
  IntentWorkspace,
} from '../shared/intentWorkspace';
import {
  captureIntentGit,
  INTENT_CAPTURE_LIMITS,
  type IntentGitCapture,
} from './intentEvidenceCapture';

/** Executed in the owning workspace store's versioned migration transaction. */
export const INTENT_EVIDENCE_SCHEMA = `
CREATE TABLE IF NOT EXISTS intent_evidence (
  id TEXT PRIMARY KEY, workspace_id TEXT NOT NULL, intent_revision INTEGER NOT NULL,
  snapshot TEXT NOT NULL, request_json TEXT NOT NULL,
  FOREIGN KEY(workspace_id, intent_revision) REFERENCES intent_revisions(workspace_id, revision)
);
CREATE INDEX IF NOT EXISTS intent_evidence_workspace ON intent_evidence(workspace_id);
CREATE TABLE IF NOT EXISTS intent_reviews (
  id TEXT PRIMARY KEY, workspace_id TEXT NOT NULL, intent_revision INTEGER NOT NULL,
  snapshot TEXT NOT NULL, request_json TEXT NOT NULL,
  FOREIGN KEY(workspace_id, intent_revision) REFERENCES intent_revisions(workspace_id, revision)
);
CREATE INDEX IF NOT EXISTS intent_review_workspace ON intent_reviews(workspace_id);
`;
function text(value: unknown, label: string, max = 128, optional = false): string {
  if (
    typeof value !== 'string' ||
    value.length > max ||
    value.includes('\0') ||
    (!optional && !value.trim())
  )
    throw new Error(`Invalid ${label}`);
  return value.trim();
}
const digest = (value: string | Buffer) => createHash('sha256').update(value).digest('hex');

export class IntentEvidenceStore {
  constructor(
    private readonly db: DatabaseSync,
    private readonly options: {
      artifactDirectory?: string;
      capture?: (cwd: string) => Promise<IntentGitCapture>;
    } = {},
    private readonly onReview?: (review: IntentReview) => void,
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
  private workspace(id: string): IntentWorkspace {
    const row = this.db.prepare('SELECT snapshot FROM intent_workspaces WHERE id=?').get(id);
    if (!row) throw new Error('Workspace no longer exists');
    return JSON.parse(String(row.snapshot));
  }
  private current(id: string, revision: unknown): IntentWorkspace {
    const workspace = this.workspace(id);
    if (revision !== workspace.revision)
      throw new Error('Intent changed elsewhere. Reload before recording evidence or review.');
    return workspace;
  }
  private criterion(workspace: IntentWorkspace, value: unknown): IntentCriterion {
    const id = text(value, 'criterion ID');
    const criterion = intentCriteria(workspace.revision, workspace.successCriteria).find(
      (c) => c.id === id,
    );
    if (!criterion) throw new Error('Select a success criterion from this saved intent revision');
    return criterion;
  }
  private evidence(id: string, workspaceId: string): IntentEvidence {
    const row = this.db
      .prepare('SELECT snapshot FROM intent_evidence WHERE id=? AND workspace_id=?')
      .get(id, workspaceId);
    if (!row) throw new Error('Evidence does not belong to this workspace');
    return JSON.parse(String(row.snapshot));
  }
  private execution(id: string, workspaceId: string): IntentExecution {
    const row = this.db
      .prepare('SELECT snapshot FROM intent_executions WHERE id=? AND workspace_id=?')
      .get(id, workspaceId);
    if (!row) throw new Error('Execution does not belong to this workspace');
    return JSON.parse(String(row.snapshot));
  }
  private existing<T>(
    table: 'intent_evidence' | 'intent_reviews',
    id: string,
    request: string,
  ): T | undefined {
    const row = this.db.prepare(`SELECT snapshot, request_json FROM ${table} WHERE id=?`).get(id);
    if (!row) return undefined;
    if (row.request_json !== request)
      throw new Error('Record ID was already used for different content');
    return JSON.parse(String(row.snapshot));
  }
  private insert(
    table: 'intent_evidence' | 'intent_reviews',
    record: IntentEvidence | IntentReview,
    request: string,
  ): void {
    const count = Number(
      this.db
        .prepare(`SELECT count(*) AS n FROM ${table} WHERE workspace_id=?`)
        .get(record.workspaceId)?.n,
    );
    if (count >= (table === 'intent_evidence' ? 512 : 256))
      throw new Error('Workspace evidence or review record limit reached');
    this.db
      .prepare(`INSERT INTO ${table} VALUES (?, ?, ?, ?, ?)`)
      .run(record.id, record.workspaceId, record.intentRevision, JSON.stringify(record), request);
  }
  private artifactDirectory(): string {
    if (this.options.artifactDirectory) return this.options.artifactDirectory;
    const file = this.db
      .prepare('PRAGMA database_list')
      .all()
      .find((row) => row.name === 'main')?.file;
    if (!file) throw new Error('Git evidence requires persistent host storage');
    return path.join(fs.realpathSync(path.dirname(String(file))), 'intent-evidence');
  }
  private readArtifact(evidence: IntentEvidence): string {
    const metadata = evidence.git;
    if (!metadata) return '';
    if (
      !Number.isSafeInteger(metadata.bytes) ||
      metadata.bytes < 0 ||
      metadata.bytes > INTENT_CAPTURE_LIMITS.bytes
    )
      throw new Error('Invalid saved artifact size');
    if (process.platform === 'win32') {
      const bytes = readIntentFile(
        this.artifactDirectory(),
        `${metadata.artifactId}.diff`,
        INTENT_CAPTURE_LIMITS.bytes,
      );
      if (bytes.length !== metadata.bytes || digest(bytes) !== metadata.sha256)
        throw new Error('Saved evidence artifact integrity check failed');
      return bytes.toString('utf8');
    }
    const file = openIntentArtifactFile(this.artifactDirectory(), `${metadata.artifactId}.diff`);
    const fd = file.fd;
    try {
      const stat = fs.fstatSync(fd);
      if (!stat.isFile() || stat.size !== metadata.bytes)
        throw new Error('Saved evidence artifact size changed');
      const bytes = Buffer.alloc(metadata.bytes + 1);
      let length = 0;
      while (length < bytes.length) {
        const read = fs.readSync(fd, bytes, length, bytes.length - length, null);
        if (!read) break;
        length += read;
      }
      const value = bytes.subarray(0, length);
      if (length !== metadata.bytes || digest(value) !== metadata.sha256)
        throw new Error('Saved evidence artifact integrity check failed');
      return value.toString('utf8');
    } finally {
      file.close();
    }
  }

  request(input: Record<string, unknown>): IntentEvidenceResponse {
    const id = text(input.id, 'workspace ID');
    if (input.action === 'readEvidence') {
      this.workspace(id);
      const evidence = this.evidence(text(input.evidenceId, 'evidence ID'), id);
      return { action: 'readEvidence', evidence, artifact: this.readArtifact(evidence) };
    }
    return this.transaction(() => {
      const workspace = this.workspace(id);
      if (input.action === 'evidence')
        return {
          action: 'evidence',
          criteria: intentCriteria(workspace.revision, workspace.successCriteria),
          evidence: this.db
            .prepare(
              'SELECT snapshot FROM intent_evidence WHERE workspace_id=? ORDER BY rowid DESC',
            )
            .all(id)
            .map((r) => JSON.parse(String(r.snapshot))),
          reviews: this.db
            .prepare('SELECT snapshot FROM intent_reviews WHERE workspace_id=? ORDER BY rowid DESC')
            .all(id)
            .map((r) => JSON.parse(String(r.snapshot))),
        };
      if (input.action === 'recordReview') return this.recordReview(input, id);
      if (input.action !== 'addEvidence') throw new Error('Unknown evidence action');
      const evidenceId = text(input.evidenceId, 'evidence ID');
      const criterionId = text(input.criterionId, 'criterion ID');
      const note = text(input.note, 'evidence note', 8000);
      const reference = text(input.reference, 'evidence reference', 4096, true);
      if (reference) {
        let url: URL;
        try {
          url = new URL(reference);
        } catch {
          throw new Error('Evidence reference must be an HTTP or HTTPS URL');
        }
        if (!['http:', 'https:'].includes(url.protocol) || url.username || url.password)
          throw new Error('Evidence reference must be an HTTP or HTTPS URL without credentials');
      }
      if (!['reported', 'unresolved', 'user-verified'].includes(String(input.assessment)))
        throw new Error('Invalid evidence assessment');
      const assessment = input.assessment as IntentEvidence['assessment'];
      const executionId =
        input.executionId === undefined ? undefined : text(input.executionId, 'execution ID');
      const linkedEvidenceId =
        input.linkedEvidenceId === undefined
          ? undefined
          : text(input.linkedEvidenceId, 'linked evidence ID');
      const key = JSON.stringify({
        action: 'addEvidence',
        id,
        expectedRevision: input.expectedRevision,
        evidenceId,
        criterionId,
        note,
        reference,
        assessment,
        executionId,
        linkedEvidenceId,
      });
      const prior = this.existing<IntentEvidence>('intent_evidence', evidenceId, key);
      if (prior) return { action: 'addEvidence', evidence: prior };
      const current = this.current(id, input.expectedRevision);
      const criterion = this.criterion(current, criterionId);
      if (executionId) this.execution(executionId, id);
      if (linkedEvidenceId) {
        const linked = this.evidence(linkedEvidenceId, id);
        if (linked.intentRevision !== current.revision || linked.criterion.id !== criterion.id)
          throw new Error('Linked evidence must address the same criterion and intent revision');
      }
      const evidence: IntentEvidence = {
        id: evidenceId,
        workspaceId: id,
        intentRevision: current.revision,
        criterion,
        kind: 'manual',
        author: 'user',
        assessment,
        note,
        reference,
        createdAt: new Date().toISOString(),
        ...(executionId ? { executionId } : {}),
        ...(linkedEvidenceId ? { linkedEvidenceId } : {}),
      };
      this.insert('intent_evidence', evidence, key);
      return { action: 'addEvidence', evidence };
    });
  }
  /** Called inside the owner observation transaction. Reports are explicitly
   * unverified and leave capacity for the user's verification records. */
  reportRun(workspace: IntentWorkspace, run: IntentRun) {
    const count = Number(
      this.db
        .prepare('SELECT count(*) AS n FROM intent_evidence WHERE workspace_id=?')
        .get(workspace.id)?.n,
    );
    for (const criterion of intentCriteria(workspace.revision, workspace.successCriteria).slice(
      0,
      Math.max(0, Math.min(64, 256 - count)),
    )) {
      const id = `run-${run.id}-${criterion.id}`;
      if (this.db.prepare('SELECT id FROM intent_evidence WHERE id=?').get(id)) continue;
      this.insert(
        'intent_evidence',
        {
          id,
          workspaceId: workspace.id,
          intentRevision: workspace.revision,
          criterion,
          kind: 'agent-report',
          author: 'agent',
          assessment: 'reported',
          note: `Agent's overall completion report. Verify this criterion independently.\n\n${run.report}`,
          reference: '',
          executionId: run.executionId,
          createdAt: new Date().toISOString(),
        },
        JSON.stringify({ runId: run.id, criterion: criterion.id }),
      );
    }
  }
  private recordReview(input: Record<string, unknown>, id: string): IntentEvidenceResponse {
    const reviewId = text(input.reviewId, 'review ID');
    const reason = text(input.reason, 'review reason', 8000);
    if (input.decision !== 'accept' && input.decision !== 'changes-requested')
      throw new Error('Invalid review decision');
    if (!Array.isArray(input.evidenceIds) || input.evidenceIds.length > 512)
      throw new Error('Invalid review evidence selection');
    const evidenceIds = [...new Set(input.evidenceIds.map((value) => text(value, 'evidence ID')))];
    const key = JSON.stringify({
      action: 'recordReview',
      id,
      expectedRevision: input.expectedRevision,
      reviewId,
      reason,
      decision: input.decision,
      evidenceIds,
    });
    const prior = this.existing<IntentReview>('intent_reviews', reviewId, key);
    if (prior) return { action: 'recordReview', review: prior };
    const workspace = this.current(id, input.expectedRevision);
    const evidence = evidenceIds.map((e) => this.evidence(e, id));
    if (evidence.some((e) => e.intentRevision !== workspace.revision))
      throw new Error('Review evidence must belong to the current intent revision');
    if (input.decision === 'accept') {
      const criteria = intentCriteria(workspace.revision, workspace.successCriteria);
      if (
        !criteria.length ||
        criteria.some(
          (c) => !evidence.some((e) => e.criterion.id === c.id && e.assessment === 'user-verified'),
        ) ||
        evidence.some((e) => e.assessment === 'unresolved')
      )
        throw new Error(
          'Acceptance requires selected user-verified evidence for every criterion and no selected unresolved evidence',
        );
    }
    const review: IntentReview = {
      id: reviewId,
      workspaceId: id,
      intentRevision: workspace.revision,
      decision: input.decision,
      reason,
      evidenceIds,
      author: 'user',
      createdAt: new Date().toISOString(),
    };
    this.insert('intent_reviews', review, key);
    this.onReview?.(review);
    return { action: 'recordReview', review };
  }

  async capture(
    input: Record<string, unknown>,
    sessions: readonly IntentLiveSession[],
  ): Promise<IntentEvidenceResponse> {
    const id = text(input.id, 'workspace ID');
    const evidenceId = text(input.evidenceId, 'evidence ID');
    const executionId = text(input.executionId, 'execution ID');
    const criterionId = text(input.criterionId, 'criterion ID');
    const key = JSON.stringify({
      action: 'captureEvidence',
      id,
      expectedRevision: input.expectedRevision,
      evidenceId,
      executionId,
      criterionId,
    });
    const prior = this.existing<IntentEvidence>('intent_evidence', evidenceId, key);
    if (prior) return { action: 'captureEvidence', evidence: prior };
    const workspace = this.current(id, input.expectedRevision);
    const criterion = this.criterion(workspace, criterionId);
    const execution = this.execution(executionId, id);
    if (!execution.session || execution.state !== 'linked' || execution.session.hub)
      throw new Error('Git evidence requires a linked local execution');
    const live = sessions.find(
      (s) => s.sessionId === execution.session!.sessionId && !(s.hub || '') && !s.hubOffline,
    );
    const cwd = live?.liveCwd || live?.cwd;
    if (!cwd) throw new Error('Local execution directory is not currently available from the host');
    const captured = await (this.options.capture ?? captureIntentGit)(cwd);
    const { artifact, ...facts } = captured;
    const bytes = Buffer.byteLength(artifact, 'utf8');
    if (bytes > INTENT_CAPTURE_LIMITS.bytes)
      throw new Error('Git evidence artifact exceeds the capture limit');
    const artifactId = randomUUID();
    let written: Pick<ReturnType<typeof openIntentArtifactFile>, 'remove' | 'close'> | undefined;
    try {
      this.current(id, input.expectedRevision);
      if (process.platform === 'win32') {
        writeIntentFile(
          this.artifactDirectory(),
          `${artifactId}.diff`,
          Buffer.from(artifact, 'utf8'),
          null,
          INTENT_CAPTURE_LIMITS.bytes,
        );
        written = {
          remove: () =>
            removeIntentFile(
              this.artifactDirectory(),
              `${artifactId}.diff`,
              digest(artifact),
              INTENT_CAPTURE_LIMITS.bytes,
            ),
          close: () => {},
        };
      } else {
        const file = openIntentArtifactFile(this.artifactDirectory(), `${artifactId}.diff`, true);
        written = file;
        fs.writeFileSync(file.fd, artifact, 'utf8');
        fs.fsyncSync(file.fd);
      }
      const result = this.transaction(() => {
        const existing = this.existing<IntentEvidence>('intent_evidence', evidenceId, key);
        if (existing) return { action: 'captureEvidence' as const, evidence: existing };
        this.current(id, input.expectedRevision);
        const current = this.execution(executionId, id);
        if (
          current.state !== 'linked' ||
          current.session?.sessionId !== execution.session!.sessionId ||
          current.session?.hub
        )
          throw new Error('Execution identity changed during evidence capture');
        const evidence: IntentEvidence = {
          id: evidenceId,
          workspaceId: id,
          intentRevision: workspace.revision,
          criterion,
          executionId,
          kind: 'git',
          author: 'user',
          assessment: 'reported',
          note: 'Host-captured Git snapshot. Criterion satisfaction has not been verified.',
          reference: '',
          createdAt: captured.capturedAt,
          git: { ...facts, artifactId, sha256: digest(artifact), bytes },
        };
        this.insert('intent_evidence', evidence, key);
        return { action: 'captureEvidence' as const, evidence };
      });
      if (written && result.evidence.git?.artifactId !== artifactId) {
        try {
          written.remove();
        } catch {
          /* Unreferenced loser bytes are safe to retain if cleanup fails. */
        }
      }
      return result;
    } catch (error) {
      if (written) {
        try {
          written.remove();
        } catch {
          /* A failed cleanup leaves only an unreferenced artifact. */
        }
      }
      throw error;
    } finally {
      written?.close();
    }
  }
}
