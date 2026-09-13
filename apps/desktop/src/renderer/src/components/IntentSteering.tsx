import React, { useCallback, useEffect, useRef, useState } from 'react';
import { ArrowUp, RefreshCw } from 'lucide-react';
import { Surface } from './Surface';
import { IntentReconciliationForm, type IntentReconciliationDraft } from './IntentControls';
import type {
  IntentDirection,
  IntentExecution,
  IntentLiveSession,
  IntentSessionRef,
  IntentWorkspace,
  IntentWorkspaceRequest,
} from '../../../main/shared/intentWorkspace';

export interface IntentDirectionDraft {
  id?: string;
  executionId: string;
  text: string;
  supersedesId?: string;
  revision?: number;
}
const EMPTY: IntentDirectionDraft = { executionId: '', text: '' };
interface Props {
  workspace: IntentWorkspace;
  visible: boolean;
  disabled: boolean;
  draft?: IntentDirectionDraft;
  onDraftChange: (draft: IntentDirectionDraft) => void;
  sessions?: IntentLiveSession[];
  onOpenSession?: (session: IntentSessionRef) => void;
  assessmentDrafts?: Record<string, IntentReconciliationDraft>;
  onAssessmentDraftsChange?: (drafts: Record<string, IntentReconciliationDraft>) => void;
}
const statusLabel = {
  accepted: 'Accepted by messaging service',
  failed: 'Delivery failed',
  unknown: 'Delivery uncertain',
};

export default function IntentSteering({
  workspace,
  visible,
  disabled,
  draft = EMPTY,
  onDraftChange,
  sessions = [],
  onOpenSession,
  assessmentDrafts,
  onAssessmentDraftsChange,
}: Props) {
  const [directions, setDirections] = useState<IntentDirection[]>([]);
  const [executions, setExecutions] = useState<IntentExecution[]>([]);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState('');
  const [loadError, setLoadError] = useState('');
  const [loading, setLoading] = useState(false);
  const [loaded, setLoaded] = useState(false);
  const [localAssessments, setLocalAssessments] = useState<
    Record<string, IntentReconciliationDraft>
  >({});
  const assessments = assessmentDrafts ?? localAssessments;
  const setAssessments: React.Dispatch<
    React.SetStateAction<Record<string, IntentReconciliationDraft>>
  > = (update) => {
    const next = typeof update === 'function' ? update(assessments) : update;
    setLocalAssessments(next);
    onAssessmentDraftsChange?.(next);
  };
  const identity = useRef(workspace.id),
    epoch = useRef(0),
    mounted = useRef(true),
    operation = useRef<number | null>(null);
  if (identity.current !== workspace.id) {
    identity.current = workspace.id;
    epoch.current++;
  }
  const owner = epoch.current;
  const current = () => mounted.current && epoch.current === owner;
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  useEffect(() => {
    setPending(false);
    operation.current = null;
    setLoaded(false);
    setError('');
    setLoadError('');
    setExecutions([]);
    setLocalAssessments({});
    setDirections([]);
  }, [workspace.id]);
  const sequence = useRef(0);
  const call = useCallback((request: IntentWorkspaceRequest) => {
    if (!window.electronAPI.intentWorkspaceRequest)
      throw new Error('Update the host to use intent directions.');
    return window.electronAPI.intentWorkspaceRequest(request);
  }, []);
  const load = useCallback(async () => {
    const mine = ++sequence.current;
    setLoading(true);
    try {
      const [records, runs] = await Promise.all([
        call({ action: 'directions', id: workspace.id }),
        call({ action: 'executions', id: workspace.id }),
      ]);
      if (mine !== sequence.current || !current()) return;
      if (records.action !== 'directions' || runs.action !== 'executions')
        throw new Error('Update the host to use intent directions.');
      setDirections(records.directions);
      setExecutions(runs.executions);
      setLoadError('');
      setLoaded(true);
    } catch (error) {
      if (mine === sequence.current && current())
        setLoadError(error instanceof Error ? error.message : String(error));
    } finally {
      if (mine === sequence.current && current()) setLoading(false);
    }
  }, [workspace.id, call]);
  useEffect(() => {
    if (visible) void load();
    return () => {
      sequence.current++;
    };
  }, [visible, load]);
  async function mutate(request: IntentWorkspaceRequest): Promise<boolean> {
    if (operation.current === owner || !current()) return false;
    operation.current = owner;
    setPending(true);
    setError('');
    sequence.current++;
    try {
      const result = await call(request);
      if (!current()) return false;
      if (result.action !== request.action)
        throw new Error(
          'The host returned an unexpected direction response. Refresh before continuing.',
        );
      await load();
      return current();
    } catch (error) {
      if (!current()) return false;
      setError(error instanceof Error ? error.message : String(error));
      // A send may have reached the host even when its response was lost.
      // Read its durable receipt; never retry the send from this error path.
      if (request.action === 'sendDirection') await load();
      return false;
    } finally {
      if (current()) {
        setPending(false);
        operation.current = null;
      }
    }
  }
  if (!visible) return null;
  const edit = (patch: Partial<IntentDirectionDraft>) =>
    onDraftChange({ ...draft, ...patch, id: crypto.randomUUID(), revision: undefined });
  return (
    <Surface
      elevation="flat"
      className="intent-executions"
      role="region"
      aria-label="Intent directions"
    >
      <div className="intent-heading">
        <h3>Direction</h3>
        <button
          type="button"
          aria-label="Refresh directions"
          disabled={pending || loading}
          onClick={() => void load()}
        >
          <RefreshCw size={14} />
        </button>
      </div>
      <p className="intent-muted">
        Record a direction for one linked execution, review the saved message, then send it.
        Acceptance means the messaging service accepted it; the agent may still have it queued.
      </p>
      {loading && !loaded && (
        <p role="status" className="intent-muted">
          Loading directions…
        </p>
      )}
      {(error || loadError) && (
        <p role="alert" className="intent-feedback intent-error">
          {error || loadError}
        </p>
      )}
      {disabled && (
        <p className="intent-muted">Save the intent before recording or sending direction.</p>
      )}
      {loaded && !executions.some((run) => run.session) && (
        <p className="intent-inline-notice">
          Start or link an agent in Execution before recording a direction.
        </p>
      )}
      <details open={!directions.length || !!draft.text || !!draft.supersedesId}>
        <summary>{draft.supersedesId ? 'Replacement direction' : 'New direction'}</summary>
        <form
          onSubmit={async (event) => {
            event.preventDefault();
            const saved = {
              ...draft,
              id: draft.id || crypto.randomUUID(),
              revision: draft.revision ?? workspace.revision,
            };
            onDraftChange(saved);
            if (
              await mutate({
                action: 'prepareDirection',
                id: workspace.id,
                directionId: saved.id,
                executionId: saved.executionId,
                expectedRevision: saved.revision,
                text: saved.text,
                ...(saved.supersedesId ? { supersedesId: saved.supersedesId } : {}),
              })
            )
              onDraftChange(EMPTY);
          }}
        >
          <fieldset disabled={disabled || pending || !loaded}>
            <label>
              Target execution
              <select
                required
                value={draft.executionId}
                disabled={!!draft.supersedesId}
                onChange={(event) => edit({ executionId: event.target.value })}
              >
                <option value="">Choose a linked execution</option>
                {executions
                  .filter((run) => run.session)
                  .map((run) => (
                    <option key={run.id} value={run.id}>
                      {run.session!.label} · {run.session!.hub || 'This host'} ·{' '}
                      {run.id.slice(0, 8)}
                    </option>
                  ))}
              </select>
            </label>
            {draft.supersedesId && (
              <p className="intent-muted">
                Replaces direction {draft.supersedesId}. Earlier messages and actions remain in
                history.
              </p>
            )}
            <label>
              What should change?
              <textarea
                required
                maxLength={8000}
                rows={4}
                value={draft.text}
                onChange={(event) => edit({ text: event.target.value })}
              />
            </label>
            <div className="intent-actions">
              <button type="submit" disabled={!draft.executionId || !draft.text.trim()}>
                Save direction for review
              </button>
              {(draft.text || draft.supersedesId) && (
                <button type="button" onClick={() => onDraftChange(EMPTY)}>
                  Discard direction draft
                </button>
              )}
            </div>
          </fieldset>
        </form>
      </details>
      {directions.length === 0 && loaded && !loadError && (
        <p className="intent-muted">No directions recorded yet.</p>
      )}
      {directions.map((direction) => {
        const last = direction.attempts.at(-1);
        const live = sessions.find(
          (session) =>
            session.sessionId === direction.target.sessionId &&
            (session.hub || '') === direction.target.hub &&
            !session.hubOffline &&
            !['ended', 'stopped'].includes(session.status || ''),
        );
        const stale = direction.intentRevision !== workspace.revision;
        const canSend = !direction.supersededBy && !stale && (!last || last.status === 'failed');
        return (
          <article
            className="intent-execution"
            key={direction.id}
            aria-label={`Direction to ${direction.target.label}`}
          >
            <div className="intent-heading">
              <strong>{direction.target.label}</strong>
              <span className="intent-muted">
                Revision {direction.intentRevision}
                {stale ? ' · Earlier intent' : ''}
              </span>
            </div>
            <p className="intent-muted">
              By you · {new Date(direction.createdAt).toLocaleString()} ·{' '}
              {direction.target.hub || 'This host'} · {direction.target.sessionId}
            </p>
            <p className="intent-result">{direction.text}</p>
            <p role="status">{last ? statusLabel[last.status] : 'Saved · Not sent'}</p>
            {direction.supersededBy && (
              <p className="intent-muted">
                Replaced by direction {direction.supersededBy}. This does not retract any earlier
                delivery.
              </p>
            )}
            {direction.supersedesId && (
              <p className="intent-muted">
                Recorded as the replacement for direction {direction.supersedesId}. Send this
                replacement separately to deliver it.
              </p>
            )}
            {stale && !last && (
              <p className="intent-muted">
                Replace this direction using the current intent before sending.
              </p>
            )}
            <details className="intent-record-details" open={!last && !direction.supersededBy}>
              <summary>Message recorded for delivery</summary>
              <pre className="intent-packet">{direction.packet}</pre>
            </details>
            {direction.attempts.length > 0 && (
              <ol className="intent-receipts">
                {direction.attempts.map((attempt) => (
                  <li key={attempt.id}>
                    <strong>{statusLabel[attempt.status]}</strong> ·{' '}
                    {new Date(attempt.startedAt).toLocaleString()}
                    <p className="intent-muted">{attempt.detail}</p>
                  </li>
                ))}
              </ol>
            )}
            {last?.status === 'unknown' && (
              <p className="intent-muted">
                Inspect the agent conversation before taking further action. This uncertain delivery
                cannot be resent, even after recording an assessment.
              </p>
            )}
            {last?.status === 'accepted' && (
              <p className="intent-muted">
                This receipt does not confirm that the agent understood or applied the direction.
              </p>
            )}
            {last && (
              <IntentReconciliationForm
                records={direction.reconciliations}
                draft={assessments[direction.id]}
                onDraftChange={(next) =>
                  setAssessments((previous) => ({ ...previous, [direction.id]: next }))
                }
                disabled={pending}
                onSave={(input) =>
                  mutate({
                    action: 'reconcileDirection',
                    id: workspace.id,
                    directionId: direction.id,
                    ...input,
                  })
                }
              />
            )}
            <div className="intent-actions">
              {canSend && (
                <button
                  className="intent-primary"
                  type="button"
                  disabled={pending || disabled || !live || !loaded}
                  onClick={() =>
                    void mutate({
                      action: 'sendDirection',
                      id: workspace.id,
                      directionId: direction.id,
                      attemptId: crypto.randomUUID(),
                    })
                  }
                >
                  <ArrowUp size={14} />
                  {last ? 'Retry delivery' : 'Send direction'}
                </button>
              )}
              {!direction.supersededBy && (
                <button
                  type="button"
                  disabled={pending || disabled || !!draft.text || !!draft.supersedesId}
                  onClick={() =>
                    onDraftChange({
                      id: crypto.randomUUID(),
                      executionId: direction.executionId,
                      text: direction.text,
                      supersedesId: direction.id,
                    })
                  }
                >
                  Replace direction
                </button>
              )}
              {onOpenSession && (
                <button
                  type="button"
                  disabled={!live}
                  onClick={() => onOpenSession(direction.target)}
                >
                  Open agent conversation
                </button>
              )}
            </div>
            {canSend && !live && (
              <p className="intent-muted">The target session is not currently available.</p>
            )}
          </article>
        );
      })}
    </Surface>
  );
}
