import React, { useCallback, useEffect, useRef, useState } from 'react';
import { RefreshCw } from 'lucide-react';
import { Surface } from './Surface';
import type {
  IntentExecution,
  IntentLiveSession,
  IntentSessionRef,
  IntentWorkspace,
  IntentWorkspaceRequest,
} from '../../../main/shared/intentWorkspace';
import type {
  IntentControl,
  IntentReconciliation,
  IntentReconciliationAssessment,
  IntentReconciliationInput,
} from '../../../main/shared/intentControl';

export interface IntentControlDraft {
  id?: string;
  executionId: string;
  kind: IntentControl['kind'];
  text: string;
  revision?: number;
}
const EMPTY: IntentControlDraft = { executionId: '', kind: 'interrupt', text: '' };
const labels = {
  accepted: 'Accepted by service',
  failed: 'Delivery failed',
  unknown: 'Delivery uncertain',
};
const assessmentLabels = {
  observed: 'I observed the intended effect',
  'not-observed': 'I did not observe the intended effect',
  unresolved: 'Still unresolved',
};
export interface IntentReconciliationDraft {
  assessment: IntentReconciliationAssessment;
  reason: string;
  id?: string;
  expectedCount?: number;
}
const EMPTY_ASSESSMENT: IntentReconciliationDraft = { assessment: 'unresolved', reason: '' };

export function IntentReconciliationForm({
  records = [],
  disabled,
  onSave,
  draft: controlled,
  onDraftChange,
}: {
  records?: IntentReconciliation[];
  disabled: boolean;
  onSave: (input: IntentReconciliationInput) => Promise<boolean>;
  draft?: IntentReconciliationDraft;
  onDraftChange?: (draft: IntentReconciliationDraft) => void;
}) {
  const [local, setLocal] = useState(EMPTY_ASSESSMENT);
  const draft = controlled ?? local,
    change = onDraftChange ?? setLocal;
  const { assessment, reason } = draft;
  const stale = draft.expectedCount !== undefined && draft.expectedCount !== records.length;
  const edit = (patch: Partial<IntentReconciliationDraft>) =>
    change({
      ...draft,
      ...patch,
      id: undefined,
      expectedCount: draft.expectedCount ?? records.length,
    });
  return (
    <details>
      <summary>Your assessment{records.length ? ` (${records.length})` : ''}</summary>
      <p className="intent-muted">
        Record what you observed and why. This preserves the service receipt and does not prove the
        agent applied the request.
      </p>
      {records.map((record) => (
        <div key={record.id}>
          <strong>{assessmentLabels[record.assessment]}</strong>
          <p className="intent-muted">By you · {new Date(record.at).toLocaleString()}</p>
          <p className="intent-result">{record.reason}</p>
        </div>
      ))}
      <form
        onSubmit={async (event) => {
          event.preventDefault();
          if (disabled || stale) return;
          const reconciliationId = draft.id || crypto.randomUUID();
          const expectedCount = draft.expectedCount ?? records.length;
          change({ ...draft, id: reconciliationId, expectedCount });
          if (await onSave({ reconciliationId, expectedCount, assessment, reason })) {
            change(EMPTY_ASSESSMENT);
          }
        }}
      >
        <fieldset disabled={disabled}>
          <label>
            Assessment
            <select
              value={assessment}
              onChange={(event) => {
                edit({ assessment: event.target.value as IntentReconciliationAssessment });
              }}
            >
              {Object.entries(assessmentLabels).map(([value, label]) => (
                <option key={value} value={value}>
                  {label}
                </option>
              ))}
            </select>
          </label>
          <label>
            What did you observe?
            <textarea
              required
              rows={3}
              maxLength={8000}
              value={reason}
              onChange={(event) => {
                edit({ reason: event.target.value });
              }}
            />
          </label>
          {stale && (
            <p className="intent-inline-notice">
              Another assessment was recorded while you were writing. Read it above before
              continuing.{' '}
              <button
                type="button"
                onClick={() => change({ ...draft, id: undefined, expectedCount: records.length })}
              >
                I reviewed the latest assessments
              </button>
            </p>
          )}
          <button type="submit" disabled={!reason.trim() || stale}>
            Record assessment
          </button>
        </fieldset>
      </form>
    </details>
  );
}

interface Props {
  workspace: IntentWorkspace;
  visible: boolean;
  disabled: boolean;
  sessions?: IntentLiveSession[];
  draft?: IntentControlDraft;
  onDraftChange?: (draft: IntentControlDraft) => void;
  onOpenSession?: (session: IntentSessionRef) => void;
  assessmentDrafts?: Record<string, IntentReconciliationDraft>;
  onAssessmentDraftsChange?: (drafts: Record<string, IntentReconciliationDraft>) => void;
}
export default function IntentControls({
  workspace,
  visible,
  disabled,
  sessions = [],
  draft: controlledDraft,
  onDraftChange,
  onOpenSession,
  assessmentDrafts,
  onAssessmentDraftsChange,
}: Props) {
  const [localDraft, setLocalDraft] = useState(EMPTY);
  const draft = controlledDraft ?? localDraft;
  const change = onDraftChange ?? setLocalDraft;
  const [controls, setControls] = useState<IntentControl[]>([]);
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
    setControls([]);
  }, [workspace.id]);
  const sequence = useRef(0);
  const call = useCallback((request: IntentWorkspaceRequest) => {
    if (!window.electronAPI.intentWorkspaceRequest)
      throw new Error('Update the host to use execution controls.');
    return window.electronAPI.intentWorkspaceRequest(request);
  }, []);
  const load = useCallback(async () => {
    const mine = ++sequence.current;
    setLoading(true);
    try {
      const [records, runs] = await Promise.all([
        call({ action: 'controls', id: workspace.id }),
        call({ action: 'executions', id: workspace.id }),
      ]);
      if (mine !== sequence.current || !current()) return;
      if (records.action !== 'controls' || runs.action !== 'executions')
        throw new Error('Update the host to use execution controls.');
      setControls(records.controls);
      setExecutions(runs.executions);
      setLoadError('');
      setLoaded(true);
    } catch (error) {
      if (mine === sequence.current && current()) setLoadError(String(error));
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
  async function mutate(request: IntentWorkspaceRequest) {
    if (operation.current === owner || !current()) return false;
    operation.current = owner;
    setPending(true);
    setError('');
    sequence.current++;
    try {
      const response = await call(request);
      if (!current()) return false;
      if (response.action !== request.action)
        throw new Error('Unexpected control response. Refresh before continuing.');
      await load();
      return current();
    } catch (error) {
      if (!current()) return false;
      setError(String(error));
      if (request.action === 'sendControl') await load();
      return false;
    } finally {
      if (current()) {
        setPending(false);
        operation.current = null;
      }
    }
  }
  if (!visible) return null;
  const edit = (patch: Partial<IntentControlDraft>) =>
    change({ ...draft, ...patch, id: crypto.randomUUID(), revision: undefined });
  return (
    <Surface
      elevation="flat"
      className="intent-executions"
      role="region"
      aria-label="Execution controls"
    >
      <div className="intent-heading">
        <h3>Interrupt and continue</h3>
        <button
          type="button"
          aria-label="Refresh controls"
          disabled={pending || loading}
          onClick={() => void load()}
        >
          <RefreshCw size={14} />
        </button>
      </div>
      <p className="intent-muted">
        Interrupt requests an end to the current turn. It does not roll back changes, cancel queued
        messages, or guarantee that background work stops. Continue sends a reviewed message to the
        same linked session; it does not restart an ended agent.
      </p>
      {loading && !loaded && (
        <p role="status" className="intent-muted">
          Loading controls…
        </p>
      )}
      {(error || loadError) && (
        <p role="alert" className="intent-feedback intent-error">
          {error || loadError}
        </p>
      )}
      {disabled && (
        <p className="intent-muted">Save the intent before recording or sending a control.</p>
      )}
      {loaded && !executions.some((run) => run.session) && (
        <p className="intent-inline-notice">
          Start or link an agent above before recording a control.
        </p>
      )}
      <details open={!controls.length || !!draft.text}>
        <summary>New control</summary>
        <form
          onSubmit={async (event) => {
            event.preventDefault();
            const saved = {
              ...draft,
              id: draft.id || crypto.randomUUID(),
              revision: draft.revision ?? workspace.revision,
            };
            change(saved);
            if (
              await mutate({
                action: 'prepareControl',
                id: workspace.id,
                controlId: saved.id,
                executionId: saved.executionId,
                expectedRevision: saved.revision,
                kind: saved.kind,
                text: saved.text,
              })
            )
              change(EMPTY);
          }}
        >
          <fieldset disabled={pending || disabled || !loaded}>
            <label>
              Control target
              <select
                required
                value={draft.executionId}
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
            <label>
              Action
              <select
                value={draft.kind}
                onChange={(event) => edit({ kind: event.target.value as IntentControl['kind'] })}
              >
                <option value="interrupt">Interrupt current turn</option>
                <option value="continue">Continue with a message</option>
              </select>
            </label>
            <label>
              {draft.kind === 'interrupt'
                ? 'Reason for interrupting (recorded here)'
                : 'Continuation message'}
              <textarea
                required
                maxLength={8000}
                rows={3}
                value={draft.text}
                onChange={(event) => edit({ text: event.target.value })}
              />
            </label>
            <div className="intent-actions">
              <button type="submit" disabled={!draft.executionId || !draft.text.trim()}>
                Save control for review
              </button>
              {draft.text && (
                <button type="button" onClick={() => change(EMPTY)}>
                  Discard control draft
                </button>
              )}
            </div>
          </fieldset>
        </form>
      </details>
      {controls.map((control) => {
        const last = control.attempts.at(-1);
        const stale = control.intentRevision !== workspace.revision;
        const live = sessions.find(
          (s) =>
            s.sessionId === control.target.sessionId &&
            (s.hub || '') === control.target.hub &&
            !s.hubOffline &&
            !['stopped', 'ended'].includes(s.status || ''),
        );
        return (
          <article
            className="intent-execution"
            key={control.id}
            aria-label={`${control.kind === 'interrupt' ? 'Interrupt' : 'Continue'} ${control.target.label}`}
          >
            <div className="intent-heading">
              <strong>
                {control.kind === 'interrupt'
                  ? 'Interrupt current turn'
                  : 'Continue with a message'}{' '}
                · {control.target.label}
              </strong>
              <span className="intent-muted">
                Revision {control.intentRevision}
                {stale ? ' · Earlier intent' : ''}
              </span>
            </div>
            <p className="intent-muted">
              By you · {new Date(control.createdAt).toLocaleString()} ·{' '}
              {control.target.hub || 'This host'} · {control.target.sessionId}
            </p>
            <p className="intent-result">{control.text}</p>
            <p role="status">{last ? labels[last.status] : 'Saved · Not sent'}</p>
            <details className="intent-record-details" open={!last}>
              <summary>
                {control.kind === 'continue'
                  ? 'Message recorded for delivery'
                  : 'Interrupt request recorded for delivery'}
              </summary>
              <pre className="intent-packet">{control.packet}</pre>
            </details>
            {!!control.attempts.length && (
              <ol className="intent-receipts">
                {control.attempts.map((attempt) => (
                  <li key={attempt.id}>
                    <strong>{labels[attempt.status]}</strong> ·{' '}
                    {new Date(attempt.startedAt).toLocaleString()}
                    <p className="intent-muted">{attempt.detail}</p>
                  </li>
                ))}
              </ol>
            )}
            {last?.status === 'unknown' && (
              <p className="intent-muted">
                Inspect the agent conversation. This uncertain request cannot be resent, even after
                recording an assessment.
              </p>
            )}
            {last?.status === 'accepted' && (
              <p className="intent-muted">
                Service acceptance does not establish the current session state or the effect of
                this request.
              </p>
            )}
            {stale && !last && (
              <p className="intent-muted">
                Record a new control using the saved intent before sending.
              </p>
            )}
            <div className="intent-actions">
              {!stale && (!last || last.status === 'failed') && (
                <button
                  type="button"
                  disabled={pending || disabled || !live || !loaded}
                  onClick={() =>
                    void mutate({
                      action: 'sendControl',
                      id: workspace.id,
                      controlId: control.id,
                      attemptId: crypto.randomUUID(),
                    })
                  }
                >
                  {last
                    ? 'Retry control delivery'
                    : control.kind === 'interrupt'
                      ? 'Send interrupt request'
                      : 'Send continuation message'}
                </button>
              )}
              {onOpenSession && (
                <button
                  type="button"
                  disabled={!live}
                  onClick={() => onOpenSession(control.target)}
                >
                  Open agent conversation
                </button>
              )}
            </div>
            {!live && (
              <p className="intent-muted">The target session is not currently available.</p>
            )}
            {!!control.attempts.length && (
              <IntentReconciliationForm
                records={control.reconciliations}
                draft={assessments[control.id]}
                onDraftChange={(next) =>
                  setAssessments((previous) => ({ ...previous, [control.id]: next }))
                }
                disabled={pending}
                onSave={(input) =>
                  mutate({
                    action: 'reconcileControl',
                    id: workspace.id,
                    controlId: control.id,
                    ...input,
                  })
                }
              />
            )}
          </article>
        );
      })}
    </Surface>
  );
}
