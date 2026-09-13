import React, { useCallback, useEffect, useRef, useState } from 'react';
import { RefreshCw } from 'lucide-react';
import { Surface } from './Surface';
import type {
  IntentCriterion,
  IntentEvidence as Evidence,
  IntentEvidenceAssessment,
  IntentReview,
} from '../../../main/shared/intentEvidence';
import type {
  IntentExecution,
  IntentWorkspace,
  IntentWorkspaceRequest,
} from '../../../main/shared/intentWorkspace';

export interface IntentEvidenceDraft {
  criterionId: string;
  executionId: string;
  note: string;
  reference: string;
  assessment: IntentEvidenceAssessment;
  linkedEvidenceId?: string;
  evidenceId?: string;
  expectedRevision?: number;
  reviewDecision: IntentReview['decision'];
  reviewReason: string;
  reviewEvidenceIds: string[];
  reviewId?: string;
  reviewRevision?: number;
  captureRequest?: { id: string; revision: number; criterionId: string; executionId: string };
}
const EMPTY: IntentEvidenceDraft = {
  criterionId: '',
  executionId: '',
  note: '',
  reference: '',
  assessment: 'reported',
  reviewDecision: 'changes-requested',
  reviewReason: '',
  reviewEvidenceIds: [],
};
const assessmentLabel = {
  reported: 'Reported',
  unresolved: 'Unresolved',
  'user-verified': 'Verified by you',
};
interface Props {
  workspace: IntentWorkspace;
  visible: boolean;
  disabled: boolean;
  draft?: IntentEvidenceDraft;
  onDraftChange: (draft: IntentEvidenceDraft) => void;
}

export default function IntentEvidence({
  workspace,
  visible,
  disabled,
  draft = EMPTY,
  onDraftChange,
}: Props) {
  const [criteria, setCriteria] = useState<IntentCriterion[]>([]);
  const [evidence, setEvidence] = useState<Evidence[]>([]);
  const [reviews, setReviews] = useState<IntentReview[]>([]);
  const [executions, setExecutions] = useState<IntentExecution[]>([]);
  const [artifacts, setArtifacts] = useState<Record<string, string>>({});
  const [pending, setPending] = useState(false);
  const [error, setError] = useState('');
  const [loadError, setLoadError] = useState('');
  const sequence = useRef(0);
  const mounted = useRef(true),
    active = useRef(workspace.id),
    busy = useRef(false),
    visibility = useRef(visible);
  active.current = workspace.id;
  visibility.current = visible;
  const [loading, setLoading] = useState(false),
    [loaded, setLoaded] = useState(false),
    [notice, setNotice] = useState('');
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  const call = useCallback((request: IntentWorkspaceRequest) => {
    if (!window.electronAPI.intentWorkspaceRequest)
      throw new Error('Update the host to use evidence review.');
    return window.electronAPI.intentWorkspaceRequest(request);
  }, []);
  const load = useCallback(async () => {
    const mine = ++sequence.current;
    setLoading(true);
    try {
      const [result, runs] = await Promise.all([
        call({ action: 'evidence', id: workspace.id }),
        call({ action: 'executions', id: workspace.id }),
      ]);
      if (!mounted.current || active.current !== workspace.id || mine !== sequence.current) return;
      if (result.action !== 'evidence' || runs.action !== 'executions')
        throw new Error('Update the host to use evidence review.');
      setCriteria(result.criteria);
      setEvidence(result.evidence);
      setReviews(result.reviews);
      setExecutions(runs.executions);
      setLoadError('');
      setLoaded(true);
    } catch (error) {
      if (mounted.current && active.current === workspace.id && mine === sequence.current)
        setLoadError(error instanceof Error ? error.message : String(error));
    } finally {
      if (mounted.current && active.current === workspace.id && mine === sequence.current)
        setLoading(false);
    }
  }, [call, workspace.id]);
  useEffect(() => {
    setCriteria([]);
    setEvidence([]);
    setReviews([]);
    setExecutions([]);
    setArtifacts({});
    setLoaded(false);
    setPending(false);
    setError('');
    setNotice('');
    busy.current = false;
  }, [workspace.id]);
  useEffect(() => {
    if (visible) void load();
    return () => {
      sequence.current++;
    };
  }, [visible, workspace.revision, load]);
  async function mutate(request: IntentWorkspaceRequest): Promise<boolean> {
    if (busy.current) return false;
    busy.current = true;
    setPending(true);
    setError('');
    setNotice('');
    if (request.action !== 'readEvidence') {
      sequence.current++;
      setLoading(false);
    }
    try {
      const result = await call(request);
      if (!mounted.current || active.current !== workspace.id) return false;
      if (result.action !== request.action)
        throw new Error('Unexpected evidence response; refresh before continuing.');
      if (result.action === 'readEvidence')
        setArtifacts((current) => ({ ...current, [result.evidence.id]: result.artifact }));
      else {
        setNotice(
          request.action === 'recordReview'
            ? 'Review decision recorded.'
            : request.action === 'captureEvidence'
              ? 'Git snapshot saved. Review it before recording verification.'
              : 'Evidence recorded.',
        );
        if (visibility.current) await load();
      }
      return true;
    } catch (error) {
      if (!mounted.current || active.current !== workspace.id) return false;
      setError(error instanceof Error ? error.message : String(error));
      return false;
    } finally {
      if (mounted.current && active.current === workspace.id) {
        setPending(false);
        busy.current = false;
      }
    }
  }
  if (!visible) return null;
  const edit = (patch: Partial<IntentEvidenceDraft>) =>
    onDraftChange({
      ...draft,
      ...patch,
      evidenceId: undefined,
      expectedRevision: undefined,
      ...('criterionId' in patch || 'executionId' in patch ? { captureRequest: undefined } : {}),
    });
  const selected = evidence.filter((item) => draft.reviewEvidenceIds.includes(item.id));
  const captureExecution = executions.find((run) => run.id === draft.executionId);
  const covered =
    criteria.length > 0 &&
    criteria.every((criterion) =>
      selected.some(
        (item) =>
          item.intentRevision === workspace.revision &&
          item.criterion.id === criterion.id &&
          item.assessment === 'user-verified',
      ),
    ) &&
    selected.every(
      (item) => item.intentRevision === workspace.revision && item.assessment !== 'unresolved',
    );
  return (
    <Surface
      elevation="flat"
      className="intent-executions"
      role="region"
      aria-label="Evidence and review"
    >
      <div className="intent-heading">
        <h3>Evidence and review</h3>
        <button
          type="button"
          disabled={pending}
          aria-label="Refresh evidence"
          onClick={() => void load()}
        >
          <RefreshCw size={14} />
        </button>
      </div>
      <p className="intent-muted">
        Review each saved success criterion against concrete evidence. Agent reports and Git
        snapshots do not establish that a criterion passed.
      </p>
      {(error || loadError) && (
        <p role="alert" className="intent-error">
          {error || loadError}
        </p>
      )}
      {error && loadError && (
        <p role="alert" className="intent-error">
          {loadError}
        </p>
      )}
      {(pending || loading || notice) && (
        <p role="status" className="intent-feedback">
          {pending ? 'Saving review changes…' : loading ? 'Loading evidence…' : notice}
        </p>
      )}
      {disabled && (
        <p className="intent-muted">
          Save the intent before adding evidence or recording a review.
        </p>
      )}
      {loaded && criteria.length === 0 ? (
        <p className="intent-muted">
          Add success criteria in Intent, one per line, to review this work.
        </p>
      ) : (
        <ol className="intent-criteria">
          {criteria.map((criterion) => {
            const relevant = evidence.filter(
              (item) =>
                item.criterion.id === criterion.id && item.intentRevision === workspace.revision,
            );
            const verified = relevant.some((item) => item.assessment === 'user-verified');
            return (
              <li key={criterion.id}>
                <strong>{criterion.text}</strong>
                <span className="intent-muted">
                  {' '}
                  ·{' '}
                  {verified
                    ? 'User verification recorded'
                    : relevant.length
                      ? 'Evidence needs review'
                      : 'No evidence yet'}
                </span>
              </li>
            );
          })}
        </ol>
      )}
      <details open={evidence.length === 0 || !!draft.note || !!draft.linkedEvidenceId}>
        <summary>{draft.linkedEvidenceId ? 'Record your verification' : 'Add evidence'}</summary>
        <form
          onSubmit={async (event) => {
            event.preventDefault();
            if (busy.current) return;
            const next = {
              ...draft,
              evidenceId: draft.evidenceId || crypto.randomUUID(),
              expectedRevision: draft.expectedRevision ?? workspace.revision,
            };
            onDraftChange(next);
            if (
              await mutate({
                action: 'addEvidence',
                id: workspace.id,
                expectedRevision: next.expectedRevision,
                evidenceId: next.evidenceId,
                criterionId: next.criterionId,
                note: next.note,
                reference: next.reference,
                assessment: next.assessment,
                ...(next.executionId ? { executionId: next.executionId } : {}),
                ...(next.linkedEvidenceId ? { linkedEvidenceId: next.linkedEvidenceId } : {}),
              })
            )
              onDraftChange({
                ...EMPTY,
                criterionId: next.criterionId,
                executionId: next.executionId,
                reviewDecision: draft.reviewDecision,
                reviewReason: draft.reviewReason,
                reviewEvidenceIds: draft.reviewEvidenceIds,
                reviewId: draft.reviewId,
                reviewRevision: draft.reviewRevision,
                captureRequest: draft.captureRequest,
              });
          }}
        >
          <fieldset disabled={disabled || pending || !criteria.length}>
            <label>
              Success criterion
              <select
                required
                value={draft.criterionId}
                onChange={(event) =>
                  edit({ criterionId: event.target.value, linkedEvidenceId: undefined })
                }
              >
                <option value="">Choose a criterion</option>
                {criteria.map((criterion) => (
                  <option key={criterion.id} value={criterion.id}>
                    {criterion.text}
                  </option>
                ))}
              </select>
            </label>
            <label>
              Related execution
              <select
                value={draft.executionId}
                onChange={(event) => edit({ executionId: event.target.value })}
              >
                <option value="">No execution selected</option>
                {executions
                  .filter((run) => run.session)
                  .map((run) => (
                    <option key={run.id} value={run.id}>
                      {run.session!.label} · {run.session!.hub || 'This host'}
                    </option>
                  ))}
              </select>
            </label>
            <label>
              Evidence assessment
              <select
                value={draft.assessment}
                onChange={(event) =>
                  edit({ assessment: event.target.value as IntentEvidenceAssessment })
                }
              >
                <option value="reported">Reported · still needs verification</option>
                <option value="unresolved">Unresolved · question or blocker</option>
                <option value="user-verified">I verified this criterion</option>
              </select>
            </label>
            {draft.linkedEvidenceId && (
              <p className="intent-muted">
                This assessment will reference the captured evidence you selected.
              </p>
            )}
            <label>
              Evidence or verification notes
              <textarea
                aria-label="Evidence or verification notes"
                required
                rows={4}
                value={draft.note}
                maxLength={8000}
                onChange={(event) => edit({ note: event.target.value })}
              />
            </label>
            <label>
              Evidence reference URL
              <input
                value={draft.reference}
                maxLength={4096}
                type="url"
                placeholder="Test run or review URL; put file references in the notes"
                onChange={(event) => edit({ reference: event.target.value })}
              />
            </label>
            <div className="intent-actions">
              <button type="submit" disabled={!draft.criterionId || !draft.note.trim()}>
                Record evidence
              </button>
              <button
                type="button"
                disabled={
                  !draft.criterionId || !captureExecution?.session || !!captureExecution.session.hub
                }
                onClick={async () => {
                  if (busy.current) return;
                  const capture = draft.captureRequest ?? {
                    id: crypto.randomUUID(),
                    revision: workspace.revision,
                    criterionId: draft.criterionId,
                    executionId: draft.executionId,
                  };
                  const next = { ...draft, captureRequest: capture };
                  onDraftChange(next);
                  if (
                    await mutate({
                      action: 'captureEvidence',
                      id: workspace.id,
                      expectedRevision: capture.revision,
                      evidenceId: capture.id,
                      criterionId: capture.criterionId,
                      executionId: capture.executionId,
                    })
                  )
                    onDraftChange({ ...next, captureRequest: undefined });
                }}
              >
                Capture Git evidence
              </button>
            </div>
            <p className="intent-muted">
              Git capture reads the selected local execution's tracked changes against its current
              HEAD. It does not run tests or include every change made during the execution.
            </p>
            {captureExecution?.session?.hub && (
              <p className="intent-inline-notice">
                Git capture is available for executions on this host. You can still record evidence
                for this remote execution.
              </p>
            )}
          </fieldset>
        </form>
      </details>
      {evidence.map((item) => (
        <article
          className="intent-execution"
          key={item.id}
          aria-label={`Evidence for ${item.criterion.text}`}
        >
          <div className="intent-heading">
            <strong>{item.criterion.text}</strong>
            <span className="intent-muted">
              Revision {item.intentRevision}
              {item.intentRevision !== workspace.revision ? ' · Earlier intent' : ''}
            </span>
          </div>
          <p className="intent-muted">
            {assessmentLabel[item.assessment]} · {new Date(item.createdAt).toLocaleString()}
            {item.git ? ' · Git facts captured by host' : ''}
          </p>
          <p className="intent-result">{item.note}</p>
          {item.reference && (
            <p className="intent-work-link">
              {/^https?:\/\//i.test(item.reference) ? (
                <a href={item.reference} target="_blank" rel="noopener noreferrer">
                  {item.reference}
                </a>
              ) : (
                item.reference
              )}
            </p>
          )}
          {item.git && (
            <>
              <p className="intent-root">
                {item.git.repositoryRoot} · HEAD {item.git.headCommit.slice(0, 12)} ·{' '}
                {item.git.changedFiles.length} tracked files
              </p>
              {item.git.omissions.map((omission, index) => (
                <p key={index} className="intent-muted">
                  {omission}
                </p>
              ))}
              <button
                type="button"
                disabled={pending}
                onClick={() =>
                  void mutate({ action: 'readEvidence', id: workspace.id, evidenceId: item.id })
                }
              >
                Inspect captured diff
              </button>
              {artifacts[item.id] !== undefined && (
                <pre className="intent-packet">
                  {artifacts[item.id] || 'No tracked working-tree diff was captured.'}
                </pre>
              )}
            </>
          )}
          {item.intentRevision === workspace.revision && (
            <div className="intent-actions">
              <label className="intent-check">
                <input
                  type="checkbox"
                  checked={draft.reviewEvidenceIds.includes(item.id)}
                  disabled={pending || disabled}
                  onChange={(event) =>
                    onDraftChange({
                      ...draft,
                      reviewId: undefined,
                      reviewRevision: undefined,
                      reviewEvidenceIds: event.target.checked
                        ? [...draft.reviewEvidenceIds, item.id]
                        : draft.reviewEvidenceIds.filter((id) => id !== item.id),
                    })
                  }
                />
                Include in review
              </label>
              {item.assessment !== 'user-verified' && (
                <button
                  type="button"
                  disabled={disabled || pending || !!draft.note}
                  onClick={() =>
                    edit({
                      criterionId: item.criterion.id,
                      executionId: item.executionId || '',
                      linkedEvidenceId: item.id,
                      assessment: 'user-verified',
                      note: '',
                    })
                  }
                >
                  Record my verification
                </button>
              )}
            </div>
          )}
        </article>
      ))}
      <h3>Review decision</h3>
      <form
        onSubmit={async (event) => {
          event.preventDefault();
          if (busy.current) return;
          const next = {
            ...draft,
            reviewId: draft.reviewId || crypto.randomUUID(),
            reviewRevision: draft.reviewRevision ?? workspace.revision,
          };
          onDraftChange(next);
          if (
            await mutate({
              action: 'recordReview',
              id: workspace.id,
              expectedRevision: next.reviewRevision,
              reviewId: next.reviewId,
              decision: next.reviewDecision,
              reason: next.reviewReason,
              evidenceIds: next.reviewEvidenceIds,
            })
          )
            onDraftChange({
              ...draft,
              reviewId: undefined,
              reviewRevision: undefined,
              reviewReason: '',
              reviewEvidenceIds: [],
            });
        }}
      >
        <fieldset disabled={pending || disabled}>
          <label>
            Review outcome
            <select
              value={draft.reviewDecision}
              onChange={(event) =>
                onDraftChange({
                  ...draft,
                  reviewId: undefined,
                  reviewRevision: undefined,
                  reviewDecision: event.target.value as IntentReview['decision'],
                })
              }
            >
              <option value="changes-requested">Request changes</option>
              <option value="accept">Accept reviewed work</option>
            </select>
          </label>
          <label>
            Review reason
            <textarea
              aria-label="Review reason"
              required
              rows={3}
              maxLength={8000}
              value={draft.reviewReason}
              onChange={(event) =>
                onDraftChange({
                  ...draft,
                  reviewId: undefined,
                  reviewRevision: undefined,
                  reviewReason: event.target.value,
                })
              }
            />
          </label>
          {draft.reviewDecision === 'accept' && !covered && (
            <p className="intent-muted">
              Select user-verified evidence for every current criterion, with no unresolved evidence
              selected, before accepting.
            </p>
          )}
          <button
            type="submit"
            disabled={!draft.reviewReason.trim() || (draft.reviewDecision === 'accept' && !covered)}
          >
            Record review decision
          </button>
        </fieldset>
      </form>
      {reviews.map((review) => (
        <article key={review.id} className="intent-execution">
          <strong>{review.decision === 'accept' ? 'Review accepted' : 'Changes requested'}</strong>
          <p className="intent-muted">
            By you · Revision {review.intentRevision}
            {review.intentRevision !== workspace.revision ? ' · Earlier intent' : ''} ·{' '}
            {new Date(review.createdAt).toLocaleString()} · {review.evidenceIds.length} evidence
            records
          </p>
          <p className="intent-result">{review.reason}</p>
        </article>
      ))}
    </Surface>
  );
}
