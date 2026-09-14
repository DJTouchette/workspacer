import { createHash } from 'node:crypto';
import type { DatabaseSync } from 'node:sqlite';
import {
  boundIntentReport,
  type IntentCompletionProposal,
  type IntentCompletionView,
} from '../shared/intentCompletion';
import type { IntentRun } from '../shared/intentAutomation';
import type { IntentReview } from '../shared/intentEvidence';
import type { IntentExecution, IntentWorkspace } from '../shared/intentWorkspace';
import type { CapturedIntentSession } from './intentWorkspaceStore';

export const INTENT_COMPLETION_SCHEMA = `
CREATE TABLE IF NOT EXISTS intent_completion_proposals (
 id TEXT PRIMARY KEY, workspace_id TEXT NOT NULL, intent_revision INTEGER NOT NULL,
 execution_id TEXT NOT NULL REFERENCES intent_executions(id), snapshot TEXT NOT NULL,
 FOREIGN KEY(workspace_id, intent_revision) REFERENCES intent_revisions(workspace_id, revision)
);
CREATE INDEX IF NOT EXISTS intent_completion_workspace ON intent_completion_proposals(workspace_id);
`;
/** Immutable proposals; currentness is derived, never rewritten into history.
 * All mutation methods execute inside the workspace/evidence write transaction. */
export class IntentCompletionStore {
  constructor(
    private db: DatabaseSync,
    private onReady?: (workspace: IntentWorkspace) => void,
  ) {}
  private workspace(id: string): IntentWorkspace {
    const row = this.db.prepare('SELECT snapshot FROM intent_workspaces WHERE id=?').get(id);
    if (!row) throw new Error('Workspace no longer exists');
    return JSON.parse(String(row.snapshot));
  }
  private run(id: string): IntentRun | null {
    const row = this.db.prepare('SELECT snapshot FROM intent_runs WHERE workspace_id=?').get(id);
    return row ? JSON.parse(String(row.snapshot)) : null;
  }
  private execution(id: string): IntentExecution | undefined {
    const row = this.db
      .prepare(
        'SELECT snapshot FROM intent_executions WHERE workspace_id=? ORDER BY rowid DESC LIMIT 1',
      )
      .get(id);
    return row ? JSON.parse(String(row.snapshot)) : undefined;
  }
  view(id: string): IntentCompletionView {
    const workspace = this.workspace(id),
      run = this.run(id),
      execution = this.execution(id);
    const proposals: IntentCompletionProposal[] = this.db
      .prepare(
        'SELECT snapshot FROM intent_completion_proposals WHERE workspace_id=? ORDER BY rowid DESC',
      )
      .all(id)
      .map((r) => JSON.parse(String(r.snapshot)));
    const current = proposals.find(
      (p) =>
        p.intentRevision === workspace.revision &&
        p.executionId === execution?.id &&
        (run ? p.runId === run.id && !run.operation : !p.runId),
    );
    return { action: 'completionProposals', proposals, currentProposalId: current?.id ?? null };
  }
  observe(samples: readonly CapturedIntentSession[]) {
    for (const row of this.db.prepare('SELECT snapshot FROM intent_workspaces').all()) {
      const workspace = JSON.parse(String(row.snapshot)) as IntentWorkspace;
      const execution = this.execution(workspace.id),
        run = this.run(workspace.id);
      if (
        !execution?.session ||
        execution.state !== 'linked' ||
        (run ? run.intentRevision : execution.intentRevision) !== workspace.revision ||
        (run &&
          (run.executionId !== execution.id ||
            run.operation ||
            !['working', 'waiting', 'review'].includes(run.state)))
      )
        continue;
      const sample = samples.find(
        (s) =>
          s.sessionId === execution.session!.sessionId && (s.hub || '') === execution.session!.hub,
      );
      if (
        !sample?.completionIdle ||
        !sample.finalReport ||
        (execution.lastObservation &&
          execution.lastObservation.observedAt > sample.observation.observedAt)
      )
        continue;
      const { text, truncated, interrupted } = sample.finalReport;
      if (interrupted) continue;
      // A question is not a completion, and a previous run's reply cannot complete a successor.
      let value: Record<string, unknown> | undefined;
      const match = /```intent-report\s*\n([\s\S]*?)\n```\s*$/.exec(text);
      try {
        if (match) value = JSON.parse(match[1]);
      } catch {
        /* explicit malformed below */
      }
      if (!run && execution.kind === 'attached' && !value) continue;
      if (value && typeof value === 'object') {
        if (value.state === 'waiting') continue;
        if (
          value.revision !== workspace.revision ||
          (run ? value.runId !== run.id : value.executionId !== execution.id)
        )
          continue;
      }
      const valid =
        value &&
        value.state === 'review' &&
        typeof value.summary === 'string' &&
        value.summary.trim() &&
        value.summary.length <= 3000;
      const bounded = boundIntentReport(text);
      const reportState: IntentCompletionProposal['reportState'] =
        truncated || bounded.truncated
          ? 'oversized'
          : !text.trim()
            ? 'missing'
            : valid
              ? 'reported'
              : 'malformed';
      const id = createHash('sha256')
        .update(
          JSON.stringify([
            workspace.id,
            workspace.revision,
            execution.id,
            run?.id,
            reportState,
            bounded.report,
          ]),
        )
        .digest('hex');
      const proposal: IntentCompletionProposal = {
        id,
        workspaceId: workspace.id,
        intentRevision: workspace.revision,
        executionId: execution.id,
        ...(run ? { runId: run.id } : {}),
        session: execution.session,
        completedAt: reportState === 'reported' ? sample.observation.observedAt : null,
        capturedAt: sample.observation.observedAt,
        reportState,
        report: bounded.report,
        redacted: !!sample.finalReport.redacted || bounded.redacted,
        provenance: 'owner-host-final-assistant/v1',
      };
      if (valid && !truncated) {
        proposal.structuredState = 'missing';
        proposal.summary = boundIntentReport(String(value!.summary)).report;
        for (const field of ['checks', 'artifacts', 'caveats', 'followUps'] as const) {
          const items = value![field];
          if (
            Array.isArray(items) &&
            items.length <= 32 &&
            items.every((s) => typeof s === 'string' && s.length <= 1000)
          ) {
            proposal[field] = items.map((s) => boundIntentReport(s).report);
            if (proposal.structuredState !== 'malformed') proposal.structuredState = 'provided';
          } else if (items !== undefined) proposal.structuredState = 'malformed';
        }
      }
      const inserted = this.db
        .prepare('INSERT OR IGNORE INTO intent_completion_proposals VALUES (?, ?, ?, ?, ?)')
        .run(id, workspace.id, workspace.revision, execution.id, JSON.stringify(proposal));
      if (inserted.changes && !run && proposal.completedAt) this.onReady?.(workspace);
    }
  }
  /** Called after the existing evidence gate, under the same write lock. */
  validateReview(review: IntentReview): IntentCompletionProposal | undefined {
    const view = this.view(review.workspaceId),
      run = this.run(review.workspaceId);
    // Legacy manual evidence reviews without an execution remain readable/writable.
    if (
      !review.proposalId &&
      !run &&
      !view.proposals.length &&
      this.execution(review.workspaceId)?.completionContract !== 1
    )
      return;
    const proposal = view.proposals.find((p) => p.id === review.proposalId);
    if (!proposal || proposal.id !== view.currentProposalId)
      throw new Error('Completion proposal changed. Refresh and select the current proposal.');
    if (
      this.db
        .prepare(
          "SELECT id FROM intent_reviews WHERE workspace_id=? AND json_extract(snapshot, '$.proposalId')=?",
        )
        .get(review.workspaceId, proposal.id)
    )
      throw new Error('This proposal has already been reviewed. Refresh before continuing.');
    if (review.decision === 'accept') {
      const uncertainDirection = this.db
        .prepare(
          'SELECT snapshot FROM intent_directions WHERE execution_id=? AND intent_revision=?',
        )
        .all(proposal.executionId, proposal.intentRevision)
        .some((row) => {
          const direction = JSON.parse(String(row.snapshot));
          return (
            !direction.supersededBy &&
            !['observed', 'not-observed'].includes(direction.reconciliations?.at(-1)?.assessment) &&
            direction.attempts.some((attempt: { status: string }) => attempt.status === 'unknown')
          );
        });
      if (uncertainDirection)
        throw new Error(
          'Direction delivery is unknown. Inspect and reconcile the execution before approval.',
        );
      const execution = this.execution(review.workspaceId);
      if (
        !proposal.completedAt ||
        proposal.reportState !== 'reported' ||
        execution?.lastObservation?.state !== 'idle' ||
        !execution.lastObservation.completionIdle ||
        (run && (run.state !== 'review' || run.operation))
      )
        throw new Error(
          'Approval requires the current completed execution. Inspect the agent and its report.',
        );
    }
    return proposal;
  }
}
