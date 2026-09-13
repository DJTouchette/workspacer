import type { DatabaseSync } from 'node:sqlite';
import { reconcileIntent } from './intentControlStore';
import {
  buildIntentDirectionContext,
  type IntentDirection,
  type IntentDirectionAttempt,
  type IntentExecution,
  type IntentLiveSession,
  type IntentSessionRef,
  type IntentWorkspace,
  type IntentWorkspaceResponse,
} from '../shared/intentWorkspace';

export type IntentDirectionDelivery = (
  target: IntentSessionRef,
  packet: string,
) => Promise<Pick<IntentDirectionAttempt, 'status' | 'detail'>>;
function required(value: unknown, name: string, max = 128): string {
  if (typeof value !== 'string' || !value.trim() || value.length > max || value.includes('\0'))
    throw new Error(`Invalid ${name}`);
  return value.trim();
}

/** The host owns both content and receipts. There is no public finish-attempt
 * action: only the delivery adapter can record the outcome of actual I/O.
 */
export class IntentSteeringStore {
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
  private direction(id: string, workspaceId: string): IntentDirection {
    const row = this.db
      .prepare('SELECT snapshot FROM intent_directions WHERE id=? AND workspace_id=?')
      .get(id, workspaceId);
    if (!row) throw new Error('Direction does not belong to this workspace');
    return JSON.parse(String(row.snapshot));
  }
  private save(direction: IntentDirection): void {
    this.db
      .prepare('UPDATE intent_directions SET snapshot=? WHERE id=?')
      .run(JSON.stringify(direction), direction.id);
  }
  request(input: Record<string, unknown>): IntentWorkspaceResponse {
    const id = required(input.id, 'workspace ID');
    return this.transaction(() => {
      const workspace = this.workspace(id);
      if (input.action === 'directions')
        return {
          action: 'directions',
          directions: this.db
            .prepare(
              'SELECT snapshot FROM intent_directions WHERE workspace_id=? ORDER BY rowid DESC',
            )
            .all(id)
            .map((row) => JSON.parse(String(row.snapshot)) as IntentDirection),
        };
      if (input.action === 'reconcileDirection') {
        const direction = this.direction(required(input.directionId, 'direction ID'), id);
        if (!direction.attempts.length)
          throw new Error('Send the direction before assessing its outcome');
        direction.reconciliations ??= [];
        reconcileIntent(direction.reconciliations, input);
        this.save(direction);
        return { action: 'reconcileDirection', direction };
      }
      if (input.action !== 'prepareDirection') throw new Error('Unknown direction action');
      const directionId = required(input.directionId, 'direction ID');
      const executionId = required(input.executionId, 'execution ID');
      const text = required(input.text, 'direction text', 8000);
      const supersedesId =
        input.supersedesId === undefined
          ? undefined
          : required(input.supersedesId, 'replaced direction ID');
      const existing = this.db
        .prepare('SELECT snapshot FROM intent_directions WHERE id=?')
        .get(directionId);
      if (existing) {
        const direction = JSON.parse(String(existing.snapshot)) as IntentDirection;
        if (
          direction.workspaceId !== id ||
          direction.executionId !== executionId ||
          direction.text !== text ||
          direction.intentRevision !== input.expectedRevision ||
          direction.supersedesId !== supersedesId
        )
          throw new Error('Direction ID was already used for different content');
        return { action: 'prepareDirection', direction, created: false };
      }
      if (input.expectedRevision !== workspace.revision)
        throw new Error(
          'Intent changed elsewhere. Reload the saved revision before recording direction.',
        );
      const row = this.db
        .prepare('SELECT snapshot FROM intent_executions WHERE id=? AND workspace_id=?')
        .get(executionId, id);
      const execution = row && (JSON.parse(String(row.snapshot)) as IntentExecution | undefined);
      if (!execution?.session)
        throw new Error('Link this execution to a session before recording direction');
      const prior = supersedesId ? this.direction(supersedesId, id) : undefined;
      if (prior && (prior.executionId !== executionId || prior.supersededBy))
        throw new Error('Replace the current direction for this execution; it changed elsewhere.');
      const direction: IntentDirection = {
        id: directionId,
        workspaceId: id,
        executionId,
        intentRevision: workspace.revision,
        target: execution.session,
        author: 'user',
        text,
        packet: '',
        createdAt: new Date().toISOString(),
        ...(supersedesId ? { supersedesId } : {}),
        attempts: [],
      };
      direction.packet = buildIntentDirectionContext(workspace, direction);
      const context = this.contextPacket(id);
      if (context) direction.packet += `\n\nRecorded workspace context:\n${context}`;
      if (Buffer.byteLength(direction.packet, 'utf8') > 240 * 1024)
        throw new Error('Saved intent and direction are too large to send together');
      this.db
        .prepare('INSERT INTO intent_directions VALUES (?, ?, ?, ?, ?)')
        .run(direction.id, id, executionId, workspace.revision, JSON.stringify(direction));
      if (prior) {
        prior.supersededBy = direction.id;
        this.save(prior);
      }
      return { action: 'prepareDirection', direction, created: true };
    });
  }

  async send(
    input: Record<string, unknown>,
    sessions: readonly IntentLiveSession[],
    deliver: IntentDirectionDelivery,
  ): Promise<IntentWorkspaceResponse> {
    const id = required(input.id, 'workspace ID');
    const directionId = required(input.directionId, 'direction ID');
    const attemptId = required(input.attemptId, 'attempt ID');
    const claim = this.transaction(() => {
      const direction = this.direction(directionId, id);
      // A lost response is read/reconciliation work, never permission to replay.
      if (direction.attempts.some((a) => a.id === attemptId || a.status !== 'failed'))
        return { direction, dispatched: false };
      if (direction.supersededBy)
        throw new Error('This direction was replaced. Review its replacement before sending.');
      if (this.workspace(id).revision !== direction.intentRevision)
        throw new Error(
          'Intent changed since this direction was saved. Replace it using the current revision before sending.',
        );
      if (direction.attempts.length >= 8) throw new Error('Direction delivery retry limit reached');
      const target = direction.target;
      const live = sessions.find(
        (s) => s.sessionId === target.sessionId && (s.hub || '') === target.hub,
      );
      const unavailable =
        !live || live.hubOffline || ['ended', 'stopped'].includes(live.status || '');
      const attempt: IntentDirectionAttempt = {
        id: attemptId,
        status: unavailable ? 'failed' : 'unknown',
        startedAt: new Date().toISOString(),
        detail: unavailable
          ? 'Target is not currently available. No message was sent.'
          : 'Delivery started; acknowledgment is not recorded yet.',
      };
      if (unavailable) attempt.finishedAt = attempt.startedAt;
      direction.attempts.push(attempt);
      this.save(direction);
      return { direction, dispatched: !unavailable };
    });
    if (!claim.dispatched) return { action: 'sendDirection', ...claim };
    let receipt: Pick<IntentDirectionAttempt, 'status' | 'detail'>;
    try {
      receipt = await deliver(claim.direction.target, claim.direction.packet);
      if (!receipt || !['accepted', 'failed', 'unknown'].includes(receipt.status))
        throw new Error('Unrecognized delivery response');
    } catch (error) {
      receipt = {
        status: 'unknown',
        detail: `Delivery acknowledgment is unknown: ${error instanceof Error ? error.message : String(error)}`,
      };
    }
    try {
      const direction = this.transaction(() => {
        // A replacement may have been recorded during I/O. Preserve it.
        const current = this.direction(directionId, id);
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
      return { action: 'sendDirection', direction, dispatched: true };
    } catch {
      throw new Error(
        'Message delivery was attempted, but its receipt could not be saved. Do not resend; refresh and inspect the agent conversation.',
      );
    }
  }
}
