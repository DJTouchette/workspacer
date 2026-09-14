import React, { useEffect, useState } from 'react';
import { Play, Pause, MessageSquare } from 'lucide-react';
import { Surface } from './Surface';
import type { IntentRun, IntentAutomationRequest } from '../../../main/shared/intentAutomation';
import type {
  IntentWorkspace,
  IntentSessionRef,
  IntentLiveSession,
} from '../../../main/shared/intentWorkspace';

export default function IntentAutomation({
  workspace,
  disabled,
  sessions = [],
  onChanged,
  onOpenSession,
}: {
  workspace: IntentWorkspace;
  disabled: boolean;
  sessions?: IntentLiveSession[];
  onChanged: () => void;
  onOpenSession?: (session: IntentSessionRef) => void;
}) {
  const [run, setRun] = useState<IntentRun | null>(null);
  const [answer, setAnswer] = useState('');
  const [minutes, setMinutes] = useState(60);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  useEffect(() => {
    let alive = true;
    const load = async () => {
      try {
        const result = await window.electronAPI.intentWorkspaceRequest?.({
          action: 'automation',
          id: workspace.id,
        });
        if (alive && result?.action === 'automation') {
          setRun(result.run);
          setError('');
        }
      } catch (error) {
        if (alive) setError(String(error));
      }
    };
    void load();
    const timer = setInterval(() => void load(), 3000);
    return () => {
      alive = false;
      clearInterval(timer);
    };
  }, [workspace.id, workspace.revision]);
  async function mutate(request: IntentAutomationRequest) {
    if (busy) return;
    setBusy(true);
    setError('');
    try {
      const result = await window.electronAPI.intentWorkspaceRequest?.(request);
      if (!result || !('run' in result)) throw new Error('Update the host to activate work.');
      setRun(result.run);
      setAnswer('');
      onChanged();
    } catch (error) {
      setError(String(error));
    } finally {
      setBusy(false);
    }
  }
  const live =
    run?.session &&
    sessions.find(
      (s) => s.sessionId === run.session!.sessionId && (s.hub || '') === run.session!.hub,
    );
  const ended = !!run?.session && (!live || ['ended', 'stopped'].includes(live.status || ''));
  const nativeQuestion = live && (live.pendingApproval || live.pendingQuestions?.length);
  const labels = {
    queued: 'Queued',
    starting: 'Starting · do not launch again',
    working: 'Working',
    waiting: 'Waiting for you',
    paused: 'Paused',
    review: 'Ready for your review',
    complete: 'Complete',
    uncertain: 'Needs inspection · delivery unconfirmed',
  };
  return (
    <Surface elevation="flat" className="intent-executions" aria-label="Autonomous work">
      <h3>{run ? labels[run.state] : 'Activate this intent'}</h3>
      {!run && (
        <p className="intent-muted">
          A dedicated manager pursues your outcome, asks for meaningful decisions, and returns the
          result for review. Uses your Fleet Manager model and existing permissions.
        </p>
      )}
      {run?.report && <p className="intent-result">{run.report}</p>}
      {run && (
        <p className="intent-muted">
          Revision {run.intentRevision} · Work limit: {new Date(run.deadline).toLocaleString()}
        </p>
      )}
      {error && (
        <p role="alert" className="intent-error">
          {error}
        </p>
      )}
      <fieldset disabled={busy || disabled}>
        {(!run || ['paused', 'complete'].includes(run.state)) && (
          <>
            <label>
              Work limit (minutes)
              <input
                type="number"
                min={1}
                max={480}
                value={minutes}
                onChange={(event) => setMinutes(Number(event.target.value))}
              />
            </label>
            <button
              type="button"
              disabled={!!run?.operation}
              onClick={() =>
                void mutate(
                  ended && run
                    ? {
                        action: 'restartIntent',
                        id: workspace.id,
                        runId: run.id,
                        expectedRevision: workspace.revision,
                        minutes,
                      }
                    : {
                        action: 'activateIntent',
                        id: workspace.id,
                        expectedRevision: workspace.revision,
                        minutes,
                      },
                )
              }
            >
              <Play size={14} />
              {ended ? 'Start replacement manager' : run ? 'Resume work' : 'Activate'}
            </button>
          </>
        )}
        {run && ['queued', 'starting', 'working', 'waiting'].includes(run.state) && (
          <button
            type="button"
            onClick={() => void mutate({ action: 'pauseIntent', id: workspace.id, runId: run.id })}
          >
            <Pause size={14} />
            Pause work
          </button>
        )}
        {run?.session &&
          (run.state === 'uncertain' || (run.state === 'paused' && run.operation)) && (
            <form
              onSubmit={(event) => {
                event.preventDefault();
                void mutate({
                  action: 'resumeInspectedIntent',
                  id: workspace.id,
                  runId: run.id,
                  expectedRevision: workspace.revision,
                  text: answer,
                });
              }}
            >
              <p className="intent-muted">
                Inspect the manager conversation first. Describe what happened and what should
                happen next. This sends a new direction; it does not replay the uncertain request.
              </p>
              <label>
                Direction after inspection
                <textarea
                  required
                  maxLength={8000}
                  value={answer}
                  onChange={(event) => setAnswer(event.target.value)}
                />
              </label>
              <button type="submit" disabled={!answer.trim()}>
                Resume with inspected direction
              </button>
            </form>
          )}
        {run?.state === 'waiting' && !nativeQuestion && (
          <form
            onSubmit={(event) => {
              event.preventDefault();
              void mutate({
                action: 'answerIntent',
                id: workspace.id,
                runId: run.id,
                expectedRevision: workspace.revision,
                text: answer,
              });
            }}
          >
            <label>
              Your answer
              <textarea
                required
                maxLength={8000}
                value={answer}
                onChange={(event) => setAnswer(event.target.value)}
              />
            </label>
            <button type="submit" disabled={!answer.trim()}>
              <MessageSquare size={14} />
              Answer and continue
            </button>
          </form>
        )}
      </fieldset>
      {run?.session && onOpenSession && (
        <button type="button" onClick={() => onOpenSession(run.session!)}>
          {nativeQuestion ? 'Answer agent question or approval' : 'Open manager'}
        </button>
      )}
      {run?.state === 'review' && (
        <p className="intent-muted">
          Review the report and criterion evidence in Review. Accepting verified work completes this
          intent; requesting changes resumes the manager.
        </p>
      )}
      {run?.state === 'paused' && (
        <p className="intent-muted">
          Pause requests an interrupt for the manager and its current workers. Check external
          background work separately.
        </p>
      )}
      {disabled && <p className="intent-muted">Save your intent edits before controlling work.</p>}
    </Surface>
  );
}
