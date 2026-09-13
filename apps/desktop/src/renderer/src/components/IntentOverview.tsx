import React, { useCallback, useEffect, useRef, useState } from 'react';
import {
  AlertTriangle,
  ArrowUpRight,
  CheckCircle2,
  Circle,
  ClipboardCheck,
  Compass,
  GitBranch,
  MessageSquare,
  Play,
  RefreshCw,
} from 'lucide-react';
import { Surface } from './Surface';
import type {
  IntentLiveSession,
  IntentSessionRef,
  IntentWorkspace,
  IntentWorkspaceResponse,
} from '../../../main/shared/intentWorkspace';
import {
  intentSessionAttention,
  summarizeIntent,
  type IntentSummaryInput,
} from '../../../main/shared/intentSummary';

export type IntentOverviewDestination =
  'intent' | 'execution' | 'direction' | 'review' | 'sources' | 'artifacts';
export function IntentAttentionBadge({
  refs,
  sessions,
}: {
  refs: readonly IntentSessionRef[];
  sessions: readonly IntentLiveSession[];
}) {
  const attention = intentSessionAttention(refs, sessions);
  return attention.length ? (
    <span
      className="intent-muted"
      style={{ color: 'var(--wks-warning)', display: 'inline-flex', alignItems: 'center', gap: 4 }}
      title="Linked agents need your attention"
      aria-label={`${attention.length} linked agents need attention`}
    >
      <AlertTriangle size={12} />
      {attention.length}
    </span>
  ) : null;
}
const actions = ['executions', 'evidence', 'directions', 'sources', 'artifacts'] as const;
const sectionLabels = {
  executions: 'Execution',
  evidence: 'Evidence and review',
  directions: 'Direction',
  sources: 'Sources',
  artifacts: 'Artifacts and alternatives',
};
interface Props {
  workspace: IntentWorkspace;
  visible: boolean;
  sessions?: IntentLiveSession[];
  onNavigate: (view: IntentOverviewDestination) => void;
  onOpenSession?: (session: IntentSessionRef) => void;
}
export default function IntentOverview({
  workspace,
  visible,
  sessions = [],
  onNavigate,
  onOpenSession,
}: Props) {
  const [data, setData] = useState<IntentSummaryInput>({});
  const [errors, setErrors] = useState<string[]>([]);
  const [loading, setLoading] = useState(true);
  const sequence = useRef(0);
  const load = useCallback(async () => {
    const mine = ++sequence.current;
    setLoading(true);
    const results = await Promise.allSettled(
      actions.map(async (action) => {
        if (!window.electronAPI.intentWorkspaceRequest)
          throw new Error('Update the host to view this summary.');
        const result = await window.electronAPI.intentWorkspaceRequest({
          action,
          id: workspace.id,
        });
        const fields = {
          executions: ['executions'],
          evidence: ['criteria', 'evidence', 'reviews'],
          directions: ['directions'],
          sources: ['sources', 'comments'],
          artifacts: ['artifacts', 'annotations', 'demonstrations', 'groups', 'selections'],
        }[action];
        if (
          result?.action !== action ||
          !fields.every((field) =>
            Array.isArray((result as unknown as Record<string, unknown>)[field]),
          )
        )
          throw new Error('Unexpected response; update the host.');
        return result;
      }),
    );
    if (mine !== sequence.current) return;
    const next: IntentSummaryInput = {},
      failures: string[] = [];
    results.forEach((result, index) => {
      if (result.status === 'rejected') {
        failures.push(`${sectionLabels[actions[index]]}: ${String(result.reason)}`);
        return;
      }
      const response: IntentWorkspaceResponse = result.value;
      if (response.action === 'executions') {
        next.executions = response.executions;
        if (response.captureWarning) failures.push(`Execution: ${response.captureWarning}`);
      }
      if (response.action === 'evidence') next.evidence = response;
      if (response.action === 'directions') next.directions = response.directions;
      if (response.action === 'sources') next.sources = response;
      if (response.action === 'artifacts') next.artifacts = response;
    });
    setData(next);
    setErrors(failures);
    setLoading(false);
  }, [workspace.id, workspace.revision]);
  useEffect(() => {
    setData({});
    setErrors([]);
    if (visible) void load();
    return () => {
      sequence.current++;
    };
  }, [visible, load]);
  if (!visible) return null;
  const summary = summarizeIntent(workspace, data);
  const refs = (data.executions ?? []).flatMap((run) => (run.session ? [run.session] : []));
  const attention = intentSessionAttention(refs, sessions);
  const decisions: { text: string; view: IntentOverviewDestination; label: string }[] = [];
  if (!workspace.outcome.trim())
    decisions.push({
      text: 'Describe the outcome you want.',
      view: 'intent',
      label: 'Refine intent',
    });
  if (!summary.coverage.length)
    decisions.push({
      text: 'Record success criteria for this intent.',
      view: 'intent',
      label: 'Add success criteria',
    });
  if (data.executions && !data.executions.length)
    decisions.push({
      text: 'No executions are linked to this work.',
      view: 'execution',
      label: 'Start or link an agent',
    });
  if (summary.unconfirmedExecutions)
    decisions.push({
      text: `${summary.unconfirmedExecutions} execution attempts need a confirmed session link.`,
      view: 'execution',
      label: 'Inspect execution attempts',
    });
  if (data.evidence && summary.uncoveredCriteria)
    decisions.push({
      text: `${summary.uncoveredCriteria} success criteria have no evidence for this revision.`,
      view: 'review',
      label: 'Add criterion evidence',
    });
  if (summary.unresolvedCriteria)
    decisions.push({
      text: `${summary.unresolvedCriteria} success criteria have unresolved evidence.`,
      view: 'review',
      label: 'Review unresolved evidence',
    });
  if (summary.review?.decision === 'changes-requested')
    decisions.push({
      text: `Your review requested changes: ${summary.review.reason}`,
      view: 'review',
      label: 'Open your review',
    });
  else if (data.evidence && summary.coverage.length && !summary.review)
    decisions.push({
      text: 'No user review decision is recorded for this revision.',
      view: 'review',
      label: 'Review this work',
    });
  if (summary.unsentDirections)
    decisions.push({
      text: `${summary.unsentDirections} saved directions have not been sent.`,
      view: 'direction',
      label: 'Review saved directions',
    });
  if (summary.uncertainDirections)
    decisions.push({
      text: `${summary.uncertainDirections} directions have uncertain delivery receipts.`,
      view: 'direction',
      label: 'Inspect direction receipts',
    });
  if (summary.driftingSources)
    decisions.push({
      text: `${summary.driftingSources} sources have changed since their accepted snapshot.`,
      view: 'sources',
      label: 'Review source changes',
    });
  if (summary.uncertainPublications)
    decisions.push({
      text: `${summary.uncertainPublications} source comments have uncertain publishing receipts.`,
      view: 'sources',
      label: 'Inspect publishing receipts',
    });
  if (summary.unselectedAlternatives)
    decisions.push({
      text: `${summary.unselectedAlternatives} alternative groups await your selection.`,
      view: 'artifacts',
      label: 'Compare alternatives',
    });
  return (
    <Surface
      elevation="flat"
      className="intent-executions intent-overview"
      role="region"
      aria-label="Work overview"
    >
      <div className="intent-heading">
        <div className="intent-section-label">
          <Compass size={14} aria-hidden="true" />
          <h3>Overview</h3>
        </div>
        <button
          type="button"
          disabled={loading}
          aria-label="Refresh work overview"
          onClick={() => void load()}
        >
          <RefreshCw size={14} />
        </button>
      </div>
      <div className="intent-outcome">
        <span className="intent-eyebrow">Desired outcome</span>
        <p>{workspace.outcome || 'Describe what this work should make possible.'}</p>
      </div>
      {loading && (
        <p role="status" className="intent-inline-notice">
          Loading saved work summary…
        </p>
      )}
      {errors.map((error) => (
        <p role="alert" key={error} className="intent-error">
          {error}
        </p>
      ))}
      {!!errors.length && (
        <p className="intent-muted">The summary is incomplete where reads failed.</p>
      )}
      <div className="intent-overview-stats">
        <button
          type="button"
          onClick={() => onNavigate('review')}
          aria-label="View criterion evidence"
        >
          <ClipboardCheck size={16} aria-hidden="true" />
          <strong>
            {data.evidence ? `${summary.verifiedCriteria} / ${summary.coverage.length}` : '—'}
          </strong>
          <span>Criteria verified by you</span>
        </button>
        <button
          type="button"
          onClick={() => onNavigate('execution')}
          aria-label="View linked executions"
        >
          <Play size={16} aria-hidden="true" />
          <strong>{data.executions ? refs.length : '—'}</strong>
          <span>Linked executions</span>
        </button>
        <button
          type="button"
          onClick={() => onNavigate(attention.length ? 'execution' : 'review')}
          aria-label="View pending decisions"
        >
          <MessageSquare size={16} aria-hidden="true" />
          <strong>{loading ? '—' : attention.length + decisions.length}</strong>
          <span>Needs your decision</span>
        </button>
      </div>
      <section className="intent-overview-section" aria-label="Current agent blockers">
        <div className="intent-section-heading">
          <h4>Needs your attention</h4>
          {attention.length > 0 && (
            <span className="intent-count-badge" data-tone="warning">
              {attention.length}
            </span>
          )}
        </div>
        {attention.map((item) => (
          <div
            key={JSON.stringify([item.target.hub, item.target.sessionId])}
            className="intent-attention-row"
          >
            <AlertTriangle size={16} aria-hidden="true" />
            <div>
              <strong>
                {item.target.label} · {item.target.hub || 'This host'}
              </strong>
              {item.approval && <p>Approval needed: {item.approval}</p>}
              {item.questions.map((question, index) => (
                <p key={index}>Agent question: {question}</p>
              ))}
              <button
                type="button"
                disabled={!onOpenSession}
                onClick={() => onOpenSession?.(item.target)}
              >
                Open agent to respond <ArrowUpRight size={12} aria-hidden="true" />
              </button>
            </div>
          </div>
        ))}
        {!!data.executions && !attention.length && (
          <p className="intent-quiet-state">
            <CheckCircle2 size={14} aria-hidden="true" />
            No approval or question is currently observed in linked sessions.
          </p>
        )}
      </section>
      <section className="intent-overview-section" aria-label="Next work decisions">
        <div className="intent-section-heading">
          <h4>Next decisions</h4>
          {decisions.length > 0 && <span className="intent-count-badge">{decisions.length}</span>}
        </div>
        {decisions.map((decision) => (
          <div key={decision.label} className="intent-decision-row">
            <p>{decision.text}</p>
            <button type="button" onClick={() => onNavigate(decision.view)}>
              {decision.label}
              <ArrowUpRight size={12} aria-hidden="true" />
            </button>
          </div>
        ))}
        {!loading && !errors.length && !decisions.length && (
          <p className="intent-quiet-state">
            <CheckCircle2 size={14} aria-hidden="true" />
            No outstanding decisions are recorded. Review the evidence before changing the work
            status.
          </p>
        )}
      </section>
      <section className="intent-overview-section" aria-label="Success criterion coverage">
        <div className="intent-section-heading">
          <h4>Success criteria</h4>
          <button type="button" onClick={() => onNavigate('review')}>
            Open evidence and review
            <ArrowUpRight size={12} aria-hidden="true" />
          </button>
        </div>
        {data.evidence ? (
          <>
            <p className="intent-muted">
              {summary.verifiedCriteria} of {summary.coverage.length} criteria have user-verified
              evidence for this revision.
            </p>
            {!!summary.coverage.length && (
              <div
                className="intent-progress-track"
                role="progressbar"
                aria-label="Criteria verified by you"
                aria-valuemin={0}
                aria-valuemax={summary.coverage.length}
                aria-valuenow={summary.verifiedCriteria}
              >
                <span
                  style={{
                    width: `${(100 * summary.verifiedCriteria) / summary.coverage.length}%`,
                  }}
                />
              </div>
            )}
            <ul className="intent-coverage-list">
              {summary.coverage.map((item) => (
                <li key={item.criterion.id}>
                  {item.verified > 0 ? (
                    <CheckCircle2 size={16} className="intent-verified-icon" aria-hidden="true" />
                  ) : (
                    <Circle size={16} aria-hidden="true" />
                  )}
                  <div>
                    <p>{item.criterion.text}</p>
                    <span className="intent-muted">
                      {item.verified} user-verified · {item.reported} reported · {item.unresolved}{' '}
                      unresolved{!item.records ? ' · No evidence recorded' : ''}
                    </span>
                  </div>
                </li>
              ))}
            </ul>
            <div
              className="intent-review-summary"
              data-decision={summary.review?.decision || 'pending'}
            >
              <p>
                Your review:{' '}
                {summary.review
                  ? summary.review.decision === 'accept'
                    ? 'Accepted this intent revision'
                    : 'Changes requested'
                  : 'Not recorded for this revision'}
              </p>
              {summary.review && <p className="intent-result">{summary.review.reason}</p>}
            </div>
          </>
        ) : (
          <p className="intent-muted">Evidence coverage is unavailable.</p>
        )}
        <p className="intent-muted">
          Agent reports and idle session state do not establish verified completion.
        </p>
      </section>
      {!!summary.alternatives.length && (
        <section className="intent-overview-section" aria-label="Selected alternatives">
          <div className="intent-section-heading">
            <h4>Alternatives</h4>
            <GitBranch size={14} aria-hidden="true" />
          </div>
          {summary.alternatives.map((item) => (
            <div key={item.group.id} className="intent-alternative-summary">
              <strong>{item.group.title}</strong>
              <p>
                {item.selected
                  ? `Your selection: ${item.selected.title}`
                  : 'No selection recorded for this revision'}
              </p>
              {item.selection && <p className="intent-muted">{item.selection.reason}</p>}
            </div>
          ))}
          <button type="button" onClick={() => onNavigate('artifacts')}>
            Open alternatives
            <ArrowUpRight size={12} aria-hidden="true" />
          </button>
        </section>
      )}
      <section className="intent-overview-section" aria-label="Retained session reports">
        <div className="intent-section-heading">
          <h4>Retained session excerpts</h4>
        </div>
        {(data.executions ?? [])
          .filter((run) => run.lastObservation?.summary)
          .slice(0, 4)
          .map((run) => (
            <details key={run.id} className="intent-report-summary" open>
              <summary>{run.session?.label || 'Unlinked execution'}</summary>
              <p className="intent-muted">
                Captured {new Date(run.lastObservation!.observedAt).toLocaleString()} · Observed
                state: {run.lastObservation!.state} · Not verification
              </p>
              <p className="intent-result">{run.lastObservation!.summary}</p>
            </details>
          ))}
        {data.executions && !data.executions.some((run) => run.lastObservation?.summary) && (
          <p className="intent-muted">Reports will appear here as linked agents make progress.</p>
        )}
        <button type="button" onClick={() => onNavigate('execution')}>
          Open all executions and reports
          <ArrowUpRight size={12} aria-hidden="true" />
        </button>
      </section>
    </Surface>
  );
}
