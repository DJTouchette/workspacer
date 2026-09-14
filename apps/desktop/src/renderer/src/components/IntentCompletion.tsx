import React, { useCallback, useEffect, useRef, useState } from 'react';
import { Surface } from './Surface';
import type {
  IntentCompletionView,
  IntentCompletionProposal,
} from '../../../main/shared/intentCompletion';
import type { IntentEvidenceResponse } from '../../../main/shared/intentEvidence';
import type {
  IntentDirection,
  IntentSessionRef,
  IntentWorkspace,
  IntentWorkspaceRequest,
} from '../../../main/shared/intentWorkspace';

type Evidence = Extract<IntentEvidenceResponse, { action: 'evidence' }>;
export default function IntentCompletion({
  workspace,
  disabled,
  onChanged,
  onEvidence,
  onOpenSession,
}: {
  workspace: IntentWorkspace;
  disabled: boolean;
  onChanged: () => void;
  onEvidence: () => void;
  onOpenSession?: (session: IntentSessionRef) => void;
}) {
  const [view, setView] = useState<IntentCompletionView>();
  const [evidence, setEvidence] = useState<Evidence>();
  const [directions, setDirections] = useState<IntentDirection[]>([]);
  const [reason, setReason] = useState('');
  const [selected, setSelected] = useState<string[]>([]);
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  const [pending, setPending] = useState<IntentWorkspaceRequest>();
  const alive = useRef(true),
    lock = useRef(false),
    sequence = useRef(0);
  const load = useCallback(async () => {
    const mine = ++sequence.current;
    try {
      const results = await Promise.all([
        window.electronAPI.intentWorkspaceRequest!({
          action: 'completionProposals',
          id: workspace.id,
        }),
        window.electronAPI.intentWorkspaceRequest!({ action: 'evidence', id: workspace.id }),
        window.electronAPI.intentWorkspaceRequest!({ action: 'directions', id: workspace.id }),
      ]);
      if (!alive.current || mine !== sequence.current) return;
      if (
        results[0].action !== 'completionProposals' ||
        results[1].action !== 'evidence' ||
        results[2].action !== 'directions'
      )
        throw new Error('Update the host to review completion reports.');
      setView(results[0]);
      setEvidence(results[1]);
      setDirections(results[2].directions);
    } catch (e) {
      if (alive.current && mine === sequence.current) setError(String(e));
    }
  }, [workspace.id]);
  useEffect(() => {
    alive.current = true;
    void load();
    const timer = setInterval(() => {
      if (!lock.current) void load();
    }, 3000);
    return () => {
      alive.current = false;
      sequence.current++;
      clearInterval(timer);
    };
  }, [load, workspace.revision]);
  const proposal = view?.proposals.find((p) => p.id === view.currentProposalId);
  const review = evidence?.reviews.find(
    (r) => r.proposalId === proposal?.id && r.intentRevision === workspace.revision,
  );
  const latestReview = evidence?.reviews.find(
    (r) => r.proposalId && r.intentRevision === workspace.revision,
  );
  const receipt =
    latestReview?.directionId &&
    directions.find((d) => d.id === latestReview.directionId)?.attempts.at(-1);
  const eligible = !!proposal?.completedAt && proposal.reportState === 'reported';
  const covered =
    !!evidence?.criteria.length &&
    evidence.criteria.every((c) =>
      evidence.evidence.some(
        (e) =>
          e.intentRevision === workspace.revision &&
          e.criterion.id === c.id &&
          e.assessment === 'user-verified' &&
          selected.includes(e.id),
      ),
    );
  async function decide(decision: 'accept' | 'changes-requested') {
    if (lock.current || !proposal) return;
    lock.current = true;
    setBusy(true);
    setError('');
    const request = pending || {
      action: 'recordReview' as const,
      id: workspace.id,
      expectedRevision: workspace.revision,
      proposalId: proposal.id,
      reviewId: crypto.randomUUID(),
      decision,
      reason,
      evidenceIds: decision === 'accept' ? selected : [],
    };
    setPending(request);
    try {
      const result = await window.electronAPI.intentWorkspaceRequest!(request);
      if (result.action !== 'recordReview')
        throw new Error('Unconfirmed review response. Retry the same operation.');
      if (!alive.current) return;
      setPending(undefined);
      setReason('');
      setSelected([]);
      onChanged();
      await load();
    } catch (e) {
      if (alive.current) setError(String(e));
    } finally {
      lock.current = false;
      if (alive.current) setBusy(false);
    }
  }
  const report = (p: IntentCompletionProposal) => (
    <>
      <p className="intent-muted">
        Revision {p.intentRevision} · {new Date(p.capturedAt).toLocaleString()} · Agent report, not
        user verification
      </p>
      <pre className="intent-packet">{p.report || 'No final assistant report was available.'}</pre>
      {p.reportState !== 'reported' && (
        <p role="status">
          Report {p.reportState}. Open the session and request a current outcome report before
          approval.
        </p>
      )}
      {p.redacted && <p className="intent-muted">Recognizable credentials were redacted.</p>}
      {(['checks', 'artifacts', 'caveats', 'followUps'] as const).map((field) => (
        <div key={field}>
          <strong>
            {
              {
                checks: 'Checks reported',
                artifacts: 'Artifacts reported',
                caveats: 'Caveats',
                followUps: 'Follow-ups and unresolved questions',
              }[field]
            }
          </strong>
          {p[field]?.length ? (
            <ul>
              {p[field]!.map((text, i) => (
                <li key={i}>{text}</li>
              ))}
            </ul>
          ) : (
            <p className="intent-muted">No structured {field} provided; inspect the report.</p>
          )}
        </div>
      ))}
      {onOpenSession && (
        <button type="button" onClick={() => onOpenSession(p.session)}>
          Open execution session
        </button>
      )}
    </>
  );
  if (!view?.proposals.length && !error) return null;
  return (
    <Surface
      elevation="flat"
      className="intent-completion intent-executions"
      aria-label="Completion review"
    >
      <h3 aria-live="polite">
        {review?.decision === 'accept'
          ? 'Approved'
          : review?.decision === 'changes-requested' ||
              (!proposal && latestReview?.decision === 'changes-requested')
            ? 'Changes requested'
            : eligible
              ? 'Review needed'
              : 'Needs inspection'}
      </h3>
      {error && (
        <p role="alert" className="intent-error">
          {error}
        </p>
      )}
      {proposal && report(proposal)}
      {latestReview?.decision === 'changes-requested' && (
        <p role="status">
          Feedback saved: {latestReview.reason}.{' '}
          {receipt
            ? receipt.status === 'accepted'
              ? 'Direction sent; consumption is not confirmed.'
              : receipt.status === 'failed'
                ? `Direction rejected: ${receipt.detail}`
                : 'Direction delivery unknown. Inspect the session before sending anything else.'
            : 'Continuation queued. If the session is unavailable, inspect it and use Start replacement manager in Overview.'}
        </p>
      )}
      {proposal && !review && (
        <fieldset disabled={busy || disabled}>
          <legend>Review this revision</legend>
          <p>
            Approval records your acceptance of selected verified evidence. It does not merge a PR
            or approve an external tracker.
          </p>
          <label>
            Outcome review feedback
            <textarea
              required
              rows={3}
              maxLength={8000}
              value={reason}
              disabled={!!pending}
              onChange={(e) => setReason(e.target.value)}
            />
          </label>
          {eligible && (
            <>
              <p>Select evidence you have verified for every criterion:</p>
              {evidence?.criteria.map((c) => (
                <div key={c.id}>
                  <strong>{c.text}</strong>
                  {evidence.evidence
                    .filter(
                      (e) =>
                        e.intentRevision === workspace.revision &&
                        e.criterion.id === c.id &&
                        e.assessment === 'user-verified',
                    )
                    .map((e) => (
                      <label className="intent-check" key={e.id}>
                        <input
                          type="checkbox"
                          disabled={!!pending}
                          checked={selected.includes(e.id)}
                          onChange={(event) =>
                            setSelected(
                              event.target.checked
                                ? [...selected, e.id]
                                : selected.filter((id) => id !== e.id),
                            )
                          }
                        />
                        {e.note}
                      </label>
                    ))}
                </div>
              ))}
              {!covered && (
                <p>
                  Approval is gated until you select user-verified evidence for every criterion.
                </p>
              )}
              <button type="button" onClick={onEvidence}>
                Select or verify evidence in Review
              </button>
            </>
          )}
          <div className="intent-actions">
            <button
              type="button"
              disabled={
                !reason.trim() ||
                (!!pending &&
                  (pending.action !== 'recordReview' || pending.decision !== 'changes-requested'))
              }
              onClick={() => void decide('changes-requested')}
            >
              Request changes
            </button>
            <button
              type="button"
              disabled={
                !eligible ||
                !covered ||
                !reason.trim() ||
                (!!pending && (pending.action !== 'recordReview' || pending.decision !== 'accept'))
              }
              onClick={() => void decide('accept')}
            >
              Approve outcome
            </button>
          </div>
          {pending && !busy && (
            <p>
              Retry uses the same review operation.{' '}
              <button type="button" onClick={() => setPending(undefined)}>
                Edit a new review request
              </button>
            </p>
          )}
        </fieldset>
      )}
      {!!view?.proposals.some((p) => p.id !== view.currentProposalId) && (
        <details>
          <summary>Earlier completion reports</summary>
          {view.proposals
            .filter((p) => p.id !== view.currentProposalId)
            .map((p) => (
              <article key={p.id} className="intent-execution">
                <p>Historical report — cannot approve the current execution.</p>
                {report(p)}
              </article>
            ))}
        </details>
      )}
    </Surface>
  );
}
