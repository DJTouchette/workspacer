import type { DatabaseSync } from 'node:sqlite';
import {
  buildIntentDirectionContext,
  type IntentDirectionAttempt,
  type IntentExecution,
  type IntentLiveSession,
  type IntentSessionRef,
  type IntentWorkspace,
} from '../shared/intentWorkspace';
import type {
  IntentControl,
  IntentControlResponse,
  IntentReconciliation,
} from '../shared/intentControl';

export type IntentControlDelivery = (
  target: IntentSessionRef,
  kind: IntentControl['kind'],
  packet: string,
) => Promise<Pick<IntentDirectionAttempt, 'status' | 'detail'>>;
export const INTENT_CONTROL_SCHEMA = `
  CREATE TABLE IF NOT EXISTS intent_controls (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES intent_workspaces(id),
    execution_id TEXT NOT NULL REFERENCES intent_executions(id),
    intent_revision INTEGER NOT NULL,
    snapshot TEXT NOT NULL,
    FOREIGN KEY(workspace_id, intent_revision) REFERENCES intent_revisions(workspace_id, revision)
  );
  CREATE INDEX IF NOT EXISTS intent_controls_workspace ON intent_controls(workspace_id);
`;
export function intentRequired(value: unknown, name: string, max = 128): string {
  if (typeof value !== 'string' || !value.trim() || value.length > max || value.includes('\0'))
    throw new Error(`Invalid ${name}`);
  return value.trim();
}
/** Append-only, optimistic concurrency; idempotent only for identical content. */
export function reconcileIntent(
  records: IntentReconciliation[],
  input: Record<string, unknown>,
): void {
  const id = intentRequired(input.reconciliationId, 'reconciliation ID');
  const reason = intentRequired(input.reason, 'assessment reason', 8000);
  const assessment = input.assessment;
  if (assessment !== 'observed' && assessment !== 'not-observed' && assessment !== 'unresolved')
    throw new Error('Invalid assessment');
  if (!Number.isSafeInteger(input.expectedCount) || Number(input.expectedCount) < 0)
    throw new Error('Invalid assessment count');
  const prior = records.find((r) => r.id === id);
  if (prior) {
    if (prior.reason !== reason || prior.assessment !== assessment)
      throw new Error('Assessment ID was already used for different content');
    return;
  }
  if (input.expectedCount !== records.length)
    throw new Error('Assessment changed elsewhere. Refresh before recording your assessment.');
  if (records.length >= 100) throw new Error('Assessment history limit reached');
  records.push({ id, author: 'user', assessment, reason, at: new Date().toISOString() });
}

export class IntentControlStore {
  constructor(
    private db: DatabaseSync,
    private contextPacket: (workspaceId: string) => string = () => '',
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
  private control(id: string, workspaceId: string): IntentControl {
    const row = this.db
      .prepare('SELECT snapshot FROM intent_controls WHERE id=? AND workspace_id=?')
      .get(id, workspaceId);
    if (!row) throw new Error('Control does not belong to this workspace');
    return JSON.parse(String(row.snapshot));
  }
  private save(control: IntentControl): void {
    this.db
      .prepare('UPDATE intent_controls SET snapshot=? WHERE id=?')
      .run(JSON.stringify(control), control.id);
  }
  request(input: Record<string, unknown>): IntentControlResponse {
    const id = intentRequired(input.id, 'workspace ID');
    return this.transaction(() => {
      const workspace = this.workspace(id);
      if (input.action === 'controls')
        return {
          action: 'controls',
          controls: this.db
            .prepare(
              'SELECT snapshot FROM intent_controls WHERE workspace_id=? ORDER BY rowid DESC',
            )
            .all(id)
            .map((row) => JSON.parse(String(row.snapshot))),
        };
      const controlId = intentRequired(input.controlId, 'control ID');
      if (input.action === 'reconcileControl') {
        const control = this.control(controlId, id);
        if (!control.attempts.length)
          throw new Error('Send the control before assessing its outcome');
        reconcileIntent(control.reconciliations, input);
        this.save(control);
        return { action: 'reconcileControl', control };
      }
      if (input.action !== 'prepareControl') throw new Error('Unknown control action');
      const executionId = intentRequired(input.executionId, 'execution ID');
      const text = intentRequired(input.text, 'control reason or message', 8000);
      const kind = input.kind;
      if (kind !== 'interrupt' && kind !== 'continue') throw new Error('Invalid control kind');
      const row = this.db.prepare('SELECT snapshot FROM intent_controls WHERE id=?').get(controlId);
      if (row) {
        const control: IntentControl = JSON.parse(String(row.snapshot));
        if (
          control.workspaceId !== id ||
          control.executionId !== executionId ||
          control.kind !== kind ||
          control.text !== text ||
          control.intentRevision !== input.expectedRevision
        )
          throw new Error('Control ID was already used for different content');
        return { action: 'prepareControl', control, created: false };
      }
      if (input.expectedRevision !== workspace.revision)
        throw new Error('Intent changed elsewhere. Reload before recording a control.');
      const run = this.db
        .prepare('SELECT snapshot FROM intent_executions WHERE id=? AND workspace_id=?')
        .get(executionId, id);
      const execution: IntentExecution | undefined = run && JSON.parse(String(run.snapshot));
      if (!execution?.session)
        throw new Error('Link this execution to a session before recording a control');
      const control: IntentControl = {
        id: controlId,
        workspaceId: id,
        executionId,
        intentRevision: workspace.revision,
        target: execution.session,
        kind,
        text,
        author: 'user',
        packet: '',
        createdAt: new Date().toISOString(),
        attempts: [],
        reconciliations: [],
      };
      control.packet =
        kind === 'continue'
          ? buildIntentDirectionContext(workspace, {
              id: controlId,
              executionId,
              text: `Continue the work as follows:\n${text}`,
            })
          : 'Interrupt the current turn (SIGINT). This does not roll back work, cancel queued messages, or guarantee that background work stops.';
      if (kind === 'continue') {
        const context = this.contextPacket(id);
        if (context) control.packet += `\n\nRecorded workspace context:\n${context}`;
      }
      if (Buffer.byteLength(control.packet, 'utf8') > 240 * 1024)
        throw new Error('Saved intent and continuation are too large to send together');
      this.db
        .prepare('INSERT INTO intent_controls VALUES (?, ?, ?, ?, ?)')
        .run(control.id, id, executionId, workspace.revision, JSON.stringify(control));
      return { action: 'prepareControl', control, created: true };
    });
  }
  async send(
    input: Record<string, unknown>,
    sessions: readonly IntentLiveSession[],
    deliver: IntentControlDelivery,
  ): Promise<IntentControlResponse> {
    const id = intentRequired(input.id, 'workspace ID'),
      controlId = intentRequired(input.controlId, 'control ID'),
      attemptId = intentRequired(input.attemptId, 'attempt ID');
    const claim = this.transaction(() => {
      const control = this.control(controlId, id);
      if (control.attempts.some((a) => a.id === attemptId || a.status !== 'failed'))
        return { control, dispatched: false };
      if (this.workspace(id).revision !== control.intentRevision)
        throw new Error(
          'Intent changed since this control was saved. Record and review a new control.',
        );
      if (control.attempts.length >= 8) throw new Error('Control retry limit reached');
      const live = sessions.find(
        (s) => s.sessionId === control.target.sessionId && (s.hub || '') === control.target.hub,
      );
      const unavailable =
        !live || live.hubOffline || ['ended', 'stopped'].includes(live.status || '');
      const at = new Date().toISOString();
      control.attempts.push({
        id: attemptId,
        status: unavailable ? 'failed' : 'unknown',
        startedAt: at,
        ...(unavailable ? { finishedAt: at } : {}),
        detail: unavailable
          ? 'Target is not currently available. No control was sent.'
          : 'Control delivery started; acknowledgment is not recorded yet.',
      });
      this.save(control);
      return { control, dispatched: !unavailable };
    });
    if (!claim.dispatched) return { action: 'sendControl', ...claim };
    let receipt: Pick<IntentDirectionAttempt, 'status' | 'detail'>;
    try {
      receipt = await deliver(claim.control.target, claim.control.kind, claim.control.packet);
      if (!receipt || !['accepted', 'failed', 'unknown'].includes(receipt.status))
        throw new Error('Unrecognized control response');
    } catch (error) {
      receipt = {
        status: 'unknown',
        detail: `Control acknowledgment is unknown: ${String(error)}`,
      };
    }
    try {
      const control = this.transaction(() => {
        const current = this.control(controlId, id);
        const attempt = current.attempts.find((a) => a.id === attemptId)!;
        if (attempt.status === 'unknown')
          Object.assign(attempt, {
            status: receipt.status,
            detail: String(receipt.detail).slice(0, 4000),
            finishedAt: new Date().toISOString(),
          });
        this.save(current);
        return current;
      });
      return { action: 'sendControl', control, dispatched: true };
    } catch {
      throw new Error(
        'Control delivery was attempted, but its receipt could not be saved. Do not resend; refresh and inspect the agent conversation.',
      );
    }
  }
}
