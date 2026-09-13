import React, { useCallback, useEffect, useRef, useState } from 'react';
import { ExternalLink, Link, Play, RefreshCw } from 'lucide-react';
import { Surface } from './Surface';
import {
  intentObservation,
  type IntentExecution,
  type IntentLiveSession,
  type IntentSessionRef,
  type IntentWorkLink,
  type IntentWorkspace,
  type IntentWorkspaceRequest,
} from '../../../main/shared/intentWorkspace';

export interface IntentExecutionsProps {
  workspace: IntentWorkspace;
  disabled?: boolean;
  visible?: boolean;
  refreshKey?: number;
  candidates?: IntentSessionRef[];
  sessions?: IntentLiveSession[];
  onStart?: (workspace: IntentWorkspace) => void;
  onOpenSession?: (session: IntentSessionRef) => void;
  onLinksChanged?: () => void;
  notice?: { workspaceId: string; text: string };
}
const NO_SESSIONS: IntentLiveSession[] = [];
const NO_CANDIDATES: IntentSessionRef[] = [];
const identity = (session: IntentSessionRef) => JSON.stringify([session.hub, session.sessionId]);
const stateLabel = (state: string) =>
  ({
    thinking: 'Working',
    streaming: 'Working',
    waiting_input: 'Needs input',
    waiting_approval: 'Needs approval',
    idle: 'Idle',
    stopped: 'Stopped',
  })[state] || 'Status unavailable';

export default function IntentExecutions({
  workspace,
  disabled = false,
  visible = true,
  refreshKey = 0,
  candidates = NO_CANDIDATES,
  sessions = NO_SESSIONS,
  onStart,
  onOpenSession,
  onLinksChanged,
  notice,
}: IntentExecutionsProps) {
  const [executions, setExecutions] = useState<IntentExecution[]>([]);
  const [links, setLinks] = useState<IntentWorkLink[]>([]);
  const [candidateId, setCandidateId] = useState('');
  const [pending, setPending] = useState(false);
  const [error, setError] = useState('');
  const [loadError, setLoadError] = useState('');
  const [captureWarning, setCaptureWarning] = useState('');
  const [linkKind, setLinkKind] = useState<IntentWorkLink['kind']>('pull-request');
  const [linkTarget, setLinkTarget] = useState('');
  const sequence = useRef(0);
  const candidate = candidates.find((session) => identity(session) === candidateId);
  const call = useCallback(async (input: IntentWorkspaceRequest) => {
    if (!window.electronAPI.intentWorkspaceRequest)
      throw new Error('This host does not support intent workspaces.');
    return window.electronAPI.intentWorkspaceRequest(input);
  }, []);
  const load = useCallback(async () => {
    const mine = ++sequence.current;
    try {
      const result = await call({ action: 'executions', id: workspace.id });
      if (mine !== sequence.current) return;
      if (result.action !== 'executions')
        throw new Error('Update the host to use intent execution links.');
      setExecutions(result.executions);
      setLinks(result.links);
      setCaptureWarning(result.captureWarning || '');
      setLoadError('');
    } catch (error) {
      if (mine === sequence.current)
        setLoadError(error instanceof Error ? error.message : String(error));
    }
  }, [workspace.id, call]);
  useEffect(() => {
    if (visible) void load();
    return () => {
      sequence.current++;
    };
  }, [load, visible, refreshKey]);
  // Live display comes from the renderer snapshot. Refresh retained host results
  // after changes settle; background capture also runs while this view is closed.
  const observedKey = !visible
    ? ''
    : JSON.stringify(
        executions.map((execution) => {
          const live =
            execution.session &&
            sessions.find(
              (s) =>
                s.sessionId === execution.session!.sessionId &&
                (s.hub || '') === execution.session!.hub,
            );
          return live ? [execution.id, intentObservation(live, '')] : [execution.id, null];
        }),
      );
  useEffect(() => {
    if (!visible) return;
    const timer = setTimeout(() => void load(), 800);
    return () => clearTimeout(timer);
  }, [observedKey, visible, load]);

  async function mutate(input: IntentWorkspaceRequest) {
    setPending(true);
    setError('');
    sequence.current++;
    try {
      await call(input);
      await load();
      if (input.action === 'attachSession' || input.action === 'linkExecution') onLinksChanged?.();
      return true;
    } catch (error) {
      setError(error instanceof Error ? error.message : String(error));
      return false;
    } finally {
      setPending(false);
    }
  }
  // Keep React state (including reference drafts) while the surface is hidden,
  // without rebuilding reports for every streaming snapshot behind the agent pane.
  if (!visible) return null;
  return (
    <Surface
      elevation="flat"
      className="intent-executions"
      role="region"
      aria-label="Feature execution"
    >
      <div className="intent-heading">
        <h3>Execution</h3>
        <button
          type="button"
          disabled={pending}
          aria-label="Refresh execution"
          onClick={() => void load()}
        >
          <RefreshCw size={14} />
        </button>
      </div>
      {notice?.workspaceId === workspace.id && (
        <p role="status" className="intent-muted">
          {notice.text}
        </p>
      )}
      {(error || loadError) && (
        <p role="alert" className="intent-error">
          {error || loadError}
        </p>
      )}
      {captureWarning && (
        <p role="alert" className="intent-error">
          {captureWarning} Retained reports may be incomplete. Refresh to retry.
        </p>
      )}
      <div className="intent-actions">
        {onStart && (
          <button type="button" disabled={disabled || pending} onClick={() => onStart(workspace)}>
            <Play size={14} /> Start agent
          </button>
        )}
        <span className="intent-muted">
          Uses saved revision {workspace.revision}. Later edits are not sent automatically.
        </span>
      </div>
      {disabled && <p className="intent-muted">Save the intent before starting or linking work.</p>}
      <div className="intent-link-session">
        <label>
          Existing agent
          <select
            value={candidateId}
            onChange={(event) => setCandidateId(event.target.value)}
            disabled={pending}
          >
            <option value="">Choose an agent to link</option>
            {candidates.map((session) => (
              <option key={identity(session)} value={identity(session)}>
                {session.label} · {session.provider}
                {session.hub ? ` · ${session.hub}` : ''} · {session.cwd}
              </option>
            ))}
          </select>
        </label>
        <button
          type="button"
          disabled={disabled || pending || !candidate}
          onClick={() =>
            candidate &&
            void mutate({
              action: 'attachSession',
              id: workspace.id,
              expectedRevision: workspace.revision,
              session: candidate,
            })
          }
        >
          <Link size={14} /> Link agent
        </button>
        <p className="intent-muted">
          Linking records an association. It sends no instructions to the agent.
        </p>
      </div>
      {executions.length === 0 && !error && !loadError && (
        <p className="intent-muted">No executions linked yet.</p>
      )}
      {executions.map((execution) => {
        const session = execution.session;
        const live =
          session &&
          sessions.find(
            (s) =>
              !s.hubOffline && s.sessionId === session.sessionId && (s.hub || '') === session.hub,
          );
        const observation = live ? intentObservation(live, '') : execution.lastObservation;
        const report = observation?.summary || execution.lastObservation?.summary;
        const retainedReport = !!live && !observation?.summary && !!report;
        const available =
          session &&
          candidates.some((candidate) => identity(candidate) === identity(session)) &&
          !sessions.some(
            (s) =>
              s.sessionId === session.sessionId && (s.hub || '') === session.hub && s.hubOffline,
          );
        return (
          <article className="intent-execution" key={execution.id}>
            <div className="intent-heading">
              <strong>{session?.label || 'Unconfirmed launch'}</strong>
              <span className="intent-muted">
                Revision {execution.intentRevision}
                {execution.intentRevision !== workspace.revision ? ' · Earlier intent' : ''}
              </span>
            </div>
            {session ? (
              <>
                <p className="intent-muted">
                  {session.provider}
                  {session.hub ? ` · ${session.hub}` : ''} ·{' '}
                  {live ? stateLabel(observation!.state) : 'Not currently observed'}
                  {execution.kind === 'attached' ? ' · Tracking link' : ' · Launched from intent'}
                </p>
                {report && (
                  <div>
                    <span className="intent-muted">
                      {retainedReport ? 'Saved agent report excerpt' : 'Agent report'}
                    </span>
                    <p className="intent-result">{report}</p>
                  </div>
                )}
                {retainedReport && (
                  <p className="intent-muted">
                    Report retained{' '}
                    {new Date(execution.lastObservation!.observedAt).toLocaleString()}
                  </p>
                )}
                {!live && observation && (
                  <p className="intent-muted">
                    Last observed {stateLabel(observation.state).toLowerCase()} ·{' '}
                    {new Date(observation.observedAt).toLocaleString()}
                  </p>
                )}
                {observation?.cwd && (
                  <p className="intent-root">Working directory: {observation.cwd}</p>
                )}
                {onOpenSession && (
                  <button
                    type="button"
                    disabled={!available}
                    onClick={() => onOpenSession(session)}
                  >
                    <ExternalLink size={14} /> Open agent
                  </button>
                )}
                {!available && (
                  <p className="intent-muted">
                    This session is not in the current agent list. Its workspace record is retained;
                    restore it from History to continue.
                  </p>
                )}
              </>
            ) : (
              <>
                <p className="intent-muted">
                  Launch has no confirmed session link. Check existing agents before starting
                  another. Select the matching agent above to reconcile this attempt.
                </p>
                <button
                  type="button"
                  disabled={pending || !candidate}
                  onClick={() =>
                    candidate &&
                    void mutate({
                      action: 'linkExecution',
                      id: workspace.id,
                      executionId: execution.id,
                      session: candidate,
                    })
                  }
                >
                  Link to this attempt
                </button>
              </>
            )}
            {execution.contextPacket && (
              <details>
                <summary>Context recorded at launch</summary>
                <pre className="intent-packet">{execution.contextPacket}</pre>
              </details>
            )}
          </article>
        );
      })}
      <h3>Branches and pull requests</h3>
      <form
        onSubmit={async (event) => {
          event.preventDefault();
          if (
            await mutate({
              action: 'addWorkLink',
              id: workspace.id,
              kind: linkKind,
              target: linkTarget,
            })
          )
            setLinkTarget('');
        }}
      >
        <fieldset disabled={pending}>
          <label>
            Reference type
            <select
              value={linkKind}
              onChange={(event) => setLinkKind(event.target.value as IntentWorkLink['kind'])}
            >
              <option value="pull-request">Pull request</option>
              <option value="branch">Branch</option>
            </select>
          </label>
          <label>
            {linkKind === 'pull-request' ? 'Pull request URL' : 'Branch reference'}
            <input
              required
              type={linkKind === 'pull-request' ? 'url' : 'text'}
              value={linkTarget}
              maxLength={4096}
              onChange={(event) => setLinkTarget(event.target.value)}
              placeholder={linkKind === 'pull-request' ? 'https://…' : 'Repository / branch name'}
            />
          </label>
          <button type="submit">Add reference</button>
        </fieldset>
      </form>
      {links.map((link) => (
        <p key={link.id} className="intent-work-link">
          {link.kind === 'pull-request' ? (
            <a href={link.target} target="_blank" rel="noopener noreferrer">
              {link.target}
            </a>
          ) : (
            <span>Branch: {link.target}</span>
          )}
        </p>
      ))}
    </Surface>
  );
}
