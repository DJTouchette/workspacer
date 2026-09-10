import React, { useCallback, useEffect, useRef, useState } from 'react';
import { taskDependencyState, taskOutcomeAccepted } from '../../../main/shared/managerRequests';
import { Check, ChevronRight, Copy, ExternalLink, FolderOpen, Pencil } from 'lucide-react';
import { Surface } from './Surface';
import { inputStyle, SmallButton } from './settings/primitives';
import { openTaskWorkflowSettings } from '../lib/settingsBus';
import {
  createTaskLinksDraft,
  normalizedDraftLinks,
  rebaseTaskLinksDraft,
} from '../lib/taskLinksDraft';
import {
  TASK_INSPECTOR_UNAVAILABLE,
  taskIsActive,
  taskSkipDisabledReason,
  type DispatchAttempt,
  type DispatchTask,
  type DispatchHistoryResponse,
  type TaskLinks,
  type TaskEditRequest,
  type TaskOpenRequest,
} from '../../../main/shared/dispatchHistory';

/* Task Inspector.
 *
 * Density rules this file follows, so the panel stays readable at a 360px rail
 * width (DESIGN_LANGUAGE.md §1–4, and see the Surface module comment):
 *   - The task title appears ONCE, in the header. Nothing below repeats it.
 *   - Identifiers (task/session/dispatch ids, absolute paths, snapshot hashes)
 *     are facts, not headlines: they live under "Details" with a copy button,
 *     never in the default view.
 *   - Every step row is one line plus, at most, one short consequence line.
 *     Rationale, reported outcomes and dispatch policy collapse behind a
 *     disclosure. A finished step shows no skip affordance at all.
 *   - Surfaces nest at most two deep: one raised header card, flat sections.
 */

const field: React.CSSProperties = { ...inputStyle, width: '100%', boxSizing: 'border-box' };
const meta: React.CSSProperties = {
  fontSize: '0.66rem',
  color: 'var(--wks-text-secondary)',
  fontFamily: 'var(--wks-font-mono)',
};
const overline: React.CSSProperties = {
  fontSize: '0.6rem',
  fontWeight: 600,
  letterSpacing: '0.08em',
  textTransform: 'uppercase',
  color: 'var(--wks-text-faint)',
};
const summaryStyle: React.CSSProperties = {
  ...overline,
  cursor: 'pointer',
  listStyle: 'none',
  display: 'flex',
  alignItems: 'center',
  gap: 4,
  padding: '6px 0',
};
const STEP_STATE: Record<string, { label: string; tone: string }> = {
  planned: { label: 'Planned', tone: 'var(--wks-text-muted)' },
  dispatched: { label: 'Working', tone: 'var(--wks-busy)' },
  blocked: { label: 'Needs attention', tone: 'var(--wks-warning)' },
  failed: { label: 'Failed', tone: 'var(--wks-error)' },
  completed: { label: 'Result received', tone: 'var(--wks-success)' },
  skipped: { label: 'Skipped by manager', tone: 'var(--wks-text-muted)' },
  waived: { label: 'Skipped by you', tone: 'var(--wks-purple)' },
};
const TERMINAL = ['completed', 'skipped', 'waived'];
/** Short name for a project or worktree folder. The full path stays under Details. */
const folderName = (p: string): string => p.split(/[\\/]/).filter(Boolean).pop() || p;

export function selectInspectorTasks(
  tasks: DispatchTask[],
  sessionId?: string,
  manager = false,
  project?: string,
): DispatchTask[] {
  return tasks
    .filter(
      (t) =>
        (!sessionId ||
          (manager
            ? t.ownerSessionId === sessionId
            : t.attempts.some((a) => a.sessionId === sessionId))) &&
        (!project ||
          t.projectCwd === project ||
          t.attempts.some((a) => a.executionCwd === project && a.worktree?.allocated)),
    )
    .sort(
      (a, b) =>
        Number(taskIsActive(b)) - Number(taskIsActive(a)) || b.createdAt.localeCompare(a.createdAt),
    );
}

/** A copyable identifier row. Ids are never in the default view; this is inside Details. */
function IdRow({ label, value }: { label: string; value: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <div style={{ display: 'flex', alignItems: 'center', gap: 6, padding: '3px 0', minWidth: 0 }}>
      <span style={{ ...overline, flex: '0 0 auto' }}>{label}</span>
      <span style={{ ...meta, flex: 1, minWidth: 0, overflowWrap: 'anywhere' }}>{value}</span>
      <button
        aria-label={`Copy ${label.toLowerCase()}`}
        title={`Copy ${label.toLowerCase()}`}
        onClick={() => {
          void navigator.clipboard?.writeText(value);
          setCopied(true);
          setTimeout(() => setCopied(false), 1200);
        }}
        style={{
          flex: '0 0 auto',
          display: 'flex',
          alignItems: 'center',
          background: 'transparent',
          border: 'none',
          padding: 2,
          margin: 0,
          cursor: 'pointer',
          color: copied ? 'var(--wks-success)' : 'var(--wks-text-muted)',
        }}
      >
        {copied ? <Check size={12} strokeWidth={2} /> : <Copy size={12} strokeWidth={2} />}
      </button>
    </div>
  );
}

/** A compact link/fact chip. Chips carry the links; nothing here is a form. */
function Chip({
  label,
  onClick,
  icon,
  title,
}: {
  label: string;
  onClick?: () => void;
  icon?: React.ReactNode;
  title?: string;
}) {
  const style: React.CSSProperties = {
    display: 'inline-flex',
    alignItems: 'center',
    gap: 4,
    maxWidth: '100%',
    padding: '3px 8px',
    fontSize: '0.66rem',
    fontFamily: 'inherit',
    fontWeight: 500,
    lineHeight: 1.4,
    borderRadius: 'var(--wks-radius-pill)',
    border: '1px solid var(--wks-border-subtle)',
    background: 'transparent',
    color: onClick ? 'var(--wks-accent-text)' : 'var(--wks-text-secondary)',
    cursor: onClick ? 'pointer' : 'default',
    overflowWrap: 'anywhere',
    textAlign: 'left',
  };
  const body = (
    <>
      <span style={{ minWidth: 0, overflowWrap: 'anywhere' }}>{label}</span>
      {icon}
    </>
  );
  return onClick ? (
    <button type="button" style={style} onClick={onClick} title={title ?? label}>
      {body}
    </button>
  ) : (
    <span style={style} title={title ?? label}>
      {body}
    </span>
  );
}

function Disclosure({
  label,
  children,
  open,
}: {
  label: string;
  children: React.ReactNode;
  open?: boolean;
}) {
  return (
    <details open={open} style={{ borderTop: '1px solid var(--wks-border-subtle)' }}>
      <summary style={summaryStyle}>
        <ChevronRight size={10} strokeWidth={2.25} />
        {label}
      </summary>
      <div style={{ paddingBottom: 8 }}>{children}</div>
    </details>
  );
}

/** Task-scoped sibling. InspectorCard remains a pure session snapshot projection. */
export default function TaskInspector({
  taskId,
  sessionId,
  manager = false,
  remote = false,
  projectCwd,
}: {
  taskId?: string;
  sessionId?: string;
  manager?: boolean;
  remote?: boolean;
  projectCwd?: string;
}) {
  const [data, setData] = useState<DispatchHistoryResponse>();
  const [error, setError] = useState('');
  const [selected, setSelected] = useState(taskId ?? '');
  const [all, setAll] = useState(false);
  const [project, setProject] = useState<string>();
  const generation = useRef(0);
  const reload = useCallback(async () => {
    const id = ++generation.current;
    try {
      const result = remote
        ? { available: false as const, reason: TASK_INSPECTOR_UNAVAILABLE }
        : ((await window.electronAPI?.dispatchHistoryRead?.()) ?? {
            available: false as const,
            reason: TASK_INSPECTOR_UNAVAILABLE,
          });
      if (id === generation.current) {
        setData(result);
        setError('');
      }
    } catch (e) {
      if (id === generation.current) setError(String(e));
    }
  }, [remote]);
  useEffect(() => {
    setData(undefined);
    setSelected(taskId ?? '');
    setAll(false);
    setProject(undefined);
    void reload();
    const timer = setInterval(() => void reload(), 3000);
    return () => {
      generation.current++;
      clearInterval(timer);
    };
  }, [reload, taskId, sessionId]);
  const tasks = data?.available ? data.tasks : [];
  const isManager = manager || (!!sessionId && tasks.some((t) => t.ownerSessionId === sessionId));
  const scope =
    all || taskId
      ? undefined
      : (sessionId ?? (data?.available ? data.currentOwnerSessionId : undefined));
  const selectedProject = project ?? (isManager && !all && !taskId ? projectCwd : undefined) ?? '';
  const choices = selectInspectorTasks(tasks, scope, sessionId ? isManager : true, selectedProject);
  const firstChoice = choices[0]?.taskId;
  useEffect(() => {
    if (!selected && firstChoice) setSelected(firstChoice);
  }, [selected, firstChoice]);
  const task = choices.find((t) => t.taskId === selected) ?? (!selected ? choices[0] : undefined);
  const update = (next: DispatchTask) => {
    generation.current++;
    setData((d) =>
      d?.available ? { ...d, tasks: d.tasks.map((t) => (t.taskId === next.taskId ? next : t)) } : d,
    );
  };
  const projects = Array.from(
    new Set([...tasks.map((t) => t.projectCwd), ...(selectedProject ? [selectedProject] : [])]),
  );
  return (
    <section
      aria-label="Task Inspector"
      style={{
        padding: 12,
        overflow: 'auto',
        minWidth: 0,
        minHeight: 0,
        flex: 1,
        display: 'flex',
        flexDirection: 'column',
        gap: 8,
        fontFamily: 'var(--wks-font-sans)',
        fontSize: '0.72rem',
        color: 'var(--wks-text-primary)',
        overflowWrap: 'anywhere',
      }}
    >
      {error && (
        <p role="alert" style={{ margin: 0, color: 'var(--wks-warning)' }}>
          Could not refresh tasks. {error}{' '}
          <SmallButton label="Retry" onClick={() => void reload()} />
        </p>
      )}
      {!data && !error && (
        <p role="status" style={{ margin: 0, color: 'var(--wks-text-secondary)' }}>
          Loading tasks…
        </p>
      )}
      {data && !data.available && (
        <p role="status" style={{ margin: 0, color: 'var(--wks-text-secondary)' }}>
          {data.reason}
        </p>
      )}
      {data?.available && (
        <>
          {data.requests?.some((r) => !r.resolved && r.ownerSessionId === (sessionId ?? data.currentOwnerSessionId)) && (
            <p role="status" style={{ margin: 0, ...meta }}>
              Request inbox: {data.requests.filter((r) => !r.resolved && r.delivery !== 'rejected' && r.ownerSessionId === (sessionId ?? data.currentOwnerSessionId)).length} awaiting resolution.
              {data.requests.some((r) => !r.resolved && r.delivery === 'unknown' && r.ownerSessionId === (sessionId ?? data.currentOwnerSessionId)) && ' Some chat deliveries are unknown; inbox requests remain available without resending.'}
            </p>
          )}
          <details>
            <summary style={summaryStyle}>
              <ChevronRight size={10} strokeWidth={2.25} />
              Filters
            </summary>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 6, paddingBottom: 6 }}>
              <select
                aria-label="Current and recent tasks"
                style={{ ...field, fontWeight: 500, color: 'var(--wks-text-primary)' }}
                value={task?.taskId ?? ''}
                onChange={(e) => setSelected(e.target.value)}
              >
                {!task && <option value="">Select a task</option>}
                {[true, false].map((active) => (
                  <optgroup key={String(active)} label={active ? 'Current' : 'Recent'}>
                    {choices
                      .filter((t) => taskIsActive(t) === active)
                      .map((t) => (
                        <option key={t.taskId} value={t.taskId}>
                          {t.title}
                        </option>
                      ))}
                  </optgroup>
                ))}
              </select>
              <label style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                <input
                  type="checkbox"
                  checked={all}
                  onChange={(e) => {
                    setAll(e.target.checked);
                    setSelected('');
                  }}
                />
                All recorded tasks
              </label>
              <label style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
                <span style={overline}>Project</span>
                <select
                  aria-label="Task project"
                  style={field}
                  value={selectedProject}
                  onChange={(e) => {
                    setProject(e.target.value);
                    setSelected('');
                  }}
                >
                  <option value="">All projects</option>
                  {projects.map((p) => (
                    <option key={p} value={p}>
                      {folderName(p)}
                    </option>
                  ))}
                </select>
              </label>
              {sessionId && !isManager && (
                <p style={{ margin: 0, ...meta }}>
                  Tasks linked to worker {sessionId} by a recorded attempt.
                </p>
              )}
            </div>
          </details>
          {!data.currentOwnerSessionId && !sessionId && (
            <p style={{ margin: 0, ...meta }}>No current manager. Showing recorded tasks.</p>
          )}
          {!choices.length && (
            <p style={{ margin: 0, color: 'var(--wks-text-secondary)' }}>
              No recorded tasks match this selection. Task ownership is shown only when recorded.
            </p>
          )}
          {selected && !task && (
            <p role="status" style={{ margin: 0, color: 'var(--wks-text-secondary)' }}>
              The selected task is no longer available in this selection. Select another task or
              reload.
            </p>
          )}
          {task && (
            <TaskDetails
              key={task.taskId}
              task={task}
              tasks={data.tasks}
              selectTask={setSelected}
              workerId={sessionId && !isManager ? sessionId : undefined}
              update={update}
              reload={reload}
              stale={!!error}
            />
          )}
        </>
      )}
    </section>
  );
}

function TaskDetails({
  task,
  tasks,
  selectTask,
  workerId,
  update,
  reload,
  stale,
}: {
  task: DispatchTask;
  tasks: DispatchTask[];
  selectTask: (id: string) => void;
  workerId?: string;
  update: (task: DispatchTask) => void;
  reload: () => Promise<void>;
  stale: boolean;
}) {
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  const [skip, setSkip] = useState<{ stepId: string; revision: number; reason: string }>();
  const edit = async (request: TaskEditRequest): Promise<boolean> => {
    setBusy(true);
    setError('');
    try {
      const response = await window.electronAPI?.taskInspectorEdit?.(request);
      if (!response) throw new Error(TASK_INSPECTOR_UNAVAILABLE);
      if (response.task) update(response.task);
      if (!response.ok) throw new Error(response.error);
      return true;
    } catch (e) {
      setError(String(e));
      return false;
    } finally {
      setBusy(false);
    }
  };
  const open = async (request: TaskOpenRequest) => {
    try {
      const response = await window.electronAPI?.taskInspectorOpen?.(request);
      if (!response?.ok) throw new Error(response?.error ?? TASK_INSPECTOR_UNAVAILABLE);
    } catch (e) {
      setError(String(e));
    }
  };
  const active = taskIsActive(task);
  const skipStep = skip && task.workflow?.definition.steps.find((s) => s.id === skip.stepId);
  return (
    <>
      <Surface elevation="raised" pad="md" tone={active ? 'var(--wks-busy)' : undefined}>
        <h2 style={{ margin: 0, fontSize: '0.8rem', fontWeight: 600, lineHeight: 1.35 }}>
          {task.title}
        </h2>
        <div style={{ ...meta, marginTop: 4 }}>
          {active ? 'Current task' : 'Recent task'} ·{' '}
          {task.ownerLabel === task.ownerSessionId ? 'Manager' : task.ownerLabel} ·{' '}
          {folderName(task.projectCwd)}
        </div>
        {!!task.sources?.length && <div style={{ ...meta, marginTop: 4 }}>Source: {task.sources.at(-1)!.label}{task.sources.at(-1)!.delivery === 'rejected' ? ' · delivery rejected' : task.sources.at(-1)!.delivery === 'unknown' ? ' · chat delivery unknown' : ''}</div>}
        {(task.cancelled || !!task.dependsOn?.length) && <div style={{ ...meta, marginTop: 4 }}>{taskDependencyState(task, tasks) === 'ready' ? 'Ready for manager decision' : task.cancelled ? 'Cancelled · workers unchanged' : 'Waiting for accepted evidence'}</div>}
        {task.dependsOn?.map((id) => {
          const dependency = tasks.find((t) => t.taskId === id && t.ownerSessionId === task.ownerSessionId && t.projectCwd === task.projectCwd);
          return <div key={id} style={{ ...meta, marginTop: 4 }}>
            {dependency ? <SmallButton label={dependency.title} onClick={() => selectTask(dependency.taskId)} /> : 'Dependency unavailable'}
            {' · '}{dependency && taskOutcomeAccepted(dependency) ? 'Accepted' : dependency?.cancelled ? 'Cancelled' : 'Waiting'}
          </div>;
        })}
        {!!task.sources?.length && <Disclosure label="Source details">{task.sources.map((s) => <IdRow key={`${s.requestId}:${s.intentKey}`} label={s.intentKey} value={s.requestId} />)}</Disclosure>}
      </Surface>
      {error && (
        <p role="alert" style={{ margin: 0, color: 'var(--wks-error)' }}>
          {error} <SmallButton label="Reload task" onClick={() => void reload()} />
        </p>
      )}
      <TaskReferences task={task} edit={edit} open={open} busy={busy || stale} />
      {task.workflow ? (
        <section aria-label="Workflow steps">
          <div style={overline}>{task.workflow.definition.name}</div>
          <ol style={{ listStyle: 'none', margin: '6px 0 0', padding: 0 }}>
            {task.workflow.steps.map((run, i) => {
              const definition = task.workflow!.definition.steps[i];
              const audit = task.audit?.find((a) => a.id === run.waiverId);
              const state = STEP_STATE[run.state] ?? {
                label: 'Unknown status',
                tone: 'var(--wks-text-muted)',
              };
              const terminal = TERMINAL.includes(run.state);
              const disabled = stale
                ? 'Refresh tasks before making changes'
                : taskSkipDisabledReason(task, run.id);
              // A finished step gets no skip affordance and no explanation for one.
              const showSkip = !terminal && !disabled;
              const detail =
                (terminal && run.reason) ||
                disabled ||
                run.outcome !== undefined ||
                definition.instructions;
              return (
                <li
                  key={run.id}
                  style={{ padding: '6px 0', borderTop: '1px solid var(--wks-border-subtle)' }}
                >
                  <div
                    style={{
                      display: 'flex',
                      flexWrap: 'wrap',
                      alignItems: 'center',
                      gap: 6,
                      minWidth: 0,
                    }}
                  >
                    <span
                      aria-hidden
                      style={{
                        flex: '0 0 auto',
                        width: 6,
                        height: 6,
                        borderRadius: '50%',
                        background: state.tone,
                      }}
                    />
                    <span style={{ fontWeight: 500, minWidth: 0 }}>{definition.label}</span>
                    <span style={{ color: state.tone, fontSize: '0.66rem', minWidth: 0 }}>
                      {state.label}
                    </span>
                    {definition.kind === 'review' && !terminal && (
                      <span style={{ ...meta, flex: '0 0 auto' }}>required</span>
                    )}
                    <span style={{ flex: 1 }} />
                    {showSkip && (
                      <SmallButton
                        label={`Skip ${definition.label}…`}
                        disabled={busy || !!disabled}
                        onClick={() =>
                          setSkip({
                            stepId: run.id,
                            revision: task.revision ?? 0,
                            reason: 'Not needed for this task',
                          })
                        }
                      />
                    )}
                  </div>
                  {audit && (
                    <div style={{ ...meta, color: 'var(--wks-purple)', marginTop: 2 }}>
                      Skipped by you · {new Date(audit.createdAt).toLocaleString()} · {audit.reason}
                    </div>
                  )}
                  {!terminal && run.reason && (
                    <div style={{ ...meta, marginTop: 2 }}>{run.reason}</div>
                  )}
                  {detail && (
                    <Disclosure label="Step details">
                      {!terminal && disabled && (
                        <p style={{ margin: '0 0 4px', ...meta }}>{disabled}</p>
                      )}
                      {run.reason && <p style={{ margin: '0 0 4px', ...meta }}>{run.reason}</p>}
                      {run.sessionId && (
                        <>
                          <IdRow label="Worker" value={run.sessionId} />
                          <IdRow label="Attempt" value={run.dispatchId ?? 'unknown'} />
                        </>
                      )}
                      {run.outcome !== undefined && (
                        <>
                          <div style={overline}>Reported outcome, not an inferred pass</div>
                          <pre style={{ ...meta, whiteSpace: 'pre-wrap', margin: '2px 0 6px' }}>
                            {JSON.stringify(run.outcome, null, 2)}
                          </pre>
                        </>
                      )}
                      <div style={overline}>Dispatch contract</div>
                      <p style={{ margin: '2px 0', ...meta }}>{definition.instructions}</p>
                      <pre style={{ ...meta, whiteSpace: 'pre-wrap', margin: '2px 0' }}>
                        {task.workflow!.templates[definition.template]?.body}
                      </pre>
                    </Disclosure>
                  )}
                </li>
              );
            })}
          </ol>
        </section>
      ) : (
        <p style={{ margin: 0, color: 'var(--wks-text-secondary)' }}>
          No workflow was recorded for this task.
        </p>
      )}
      {skip && skipStep && (
        <Surface elevation="flat" pad="md">
          <div role="dialog" aria-label="Skip task step" aria-modal="false">
            <div style={{ fontWeight: 600 }}>Skip {skipStep.label}?</div>
            <p style={{ margin: '4px 0', ...meta }}>
              This task only. It does not mark the step passed or reviewed.
              {skipStep.kind === 'review' && (
                <>
                  {' '}
                  <span style={{ color: 'var(--wks-warning)' }}>
                    This skips the required independent review for this task.
                  </span>
                </>
              )}
            </p>
            <label style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
              <span style={overline}>Reason (optional)</span>
              <input
                style={field}
                maxLength={2000}
                value={skip.reason}
                onChange={(e) => setSkip({ ...skip, reason: e.target.value })}
              />
            </label>
            {skip.revision !== (task.revision ?? 0) && (
              <p role="status" style={{ margin: '6px 0', ...meta }}>
                This task changed. Refresh this confirmation before skipping.{' '}
                <SmallButton
                  label="Refresh confirmation"
                  onClick={() => setSkip({ ...skip, revision: task.revision ?? 0 })}
                />
              </p>
            )}
            {taskSkipDisabledReason(task, skip.stepId) && (
              <p role="status" style={{ margin: '6px 0', ...meta }}>
                {taskSkipDisabledReason(task, skip.stepId)}
              </p>
            )}
            <div style={{ display: 'flex', gap: 6, marginTop: 8 }}>
              <SmallButton
                label="Skip this step"
                primary
                disabled={
                  busy ||
                  stale ||
                  !!taskSkipDisabledReason(task, skip.stepId) ||
                  skip.revision !== (task.revision ?? 0)
                }
                onClick={() =>
                  void edit({
                    taskId: task.taskId,
                    expectedTaskRevision: skip.revision,
                    action: 'waive',
                    stepId: skip.stepId,
                    reason: skip.reason.trim() || undefined,
                  }).then((ok) => {
                    if (ok) setSkip(undefined);
                  })
                }
              />
              <SmallButton label="Cancel" disabled={busy} onClick={() => setSkip(undefined)} />
            </div>
          </div>
        </Surface>
      )}
      <Disclosure label="Details">
        <IdRow label="Task" value={task.taskId} />
        <IdRow label="Manager" value={task.ownerSessionId} />
        <IdRow label="Project" value={task.projectCwd} />
        <div aria-label="Reference edit history" style={{ ...meta, marginTop: 6 }}>
          <div style={overline}>Reference edit history</div>
          {(task.audit ?? [])
            .filter((entry) => entry.action === 'links')
            .map((entry) => (
              <div key={entry.id} style={{ marginTop: 4 }}>
                References updated by{' '}
                {entry.actor === 'host-user'
                  ? 'you'
                  : entry.actor === 'manager'
                    ? 'manager'
                    : 'unknown actor'}{' '}
                ·{' '}
                <time dateTime={entry.createdAt}>{new Date(entry.createdAt).toLocaleString()}</time>
              </div>
            ))}
          <p style={{ margin: '4px 0 0' }}>
            Individual link origins and older edits outside this recorded history are unknown.
          </p>
        </div>
        {task.workflow && (
          <>
            <IdRow
              label="Workflow"
              value={`${task.workflow.definition.name} v${task.workflow.definition.revision}`}
            />
            <IdRow label="Snapshot" value={task.workflow.hash} />
          </>
        )}
        <div style={{ ...overline, marginTop: 8 }}>Recorded work</div>
        {!task.attempts.length && (
          <p style={{ margin: '2px 0', ...meta }}>No dispatches recorded yet.</p>
        )}
        {task.attempts.map((a) => (
          <details key={a.dispatchId} open={a.sessionId === workerId}>
            <summary style={{ ...summaryStyle, textTransform: 'none', letterSpacing: 0 }}>
              <ChevronRight size={10} strokeWidth={2.25} />
              {a.stage ?? 'Attempt'} · {a.sessionId === workerId ? 'Selected worker · ' : ''}
              {a.stale ? 'Status unknown — last recorded' : a.lifecycle}
            </summary>
            <IdRow label="Worker" value={a.sessionId} />
            <IdRow label="Attempt" value={a.dispatchId} />
            <IdRow label="Folder" value={a.executionCwd} />
            <IdRow label="Branch" value={a.worktree?.branch ?? 'Not recorded'} />
            {a.worktree?.fallback && (
              <p style={{ margin: '2px 0', ...meta, color: 'var(--wks-warning)' }}>
                Worktree allocation fell back to the project folder. {a.worktree.error}
              </p>
            )}
            {a.worktree?.allocated && (
              <SmallButton
                label="Open folder"
                disabled={!a.worktree.directoryIdentity}
                onClick={() =>
                  void open({ taskId: task.taskId, kind: 'worktree', dispatchId: a.dispatchId })
                }
              />
            )}
            {a.worktree?.allocated && !a.worktree.directoryIdentity && (
              <p style={{ margin: '2px 0', ...meta }}>
                This older record cannot verify the worktree folder for opening.
              </p>
            )}
          </details>
        ))}
        <div style={{ marginTop: 8 }}>
          <SmallButton
            label="Configure workflow"
            onClick={() => openTaskWorkflowSettings(task.projectCwd)}
          />
          <p style={{ margin: '4px 0 0', ...meta }}>
            Settings apply to new tasks only. This task keeps its recorded workflow.
          </p>
        </div>
      </Disclosure>
    </>
  );
}

/** The latest attempt whose worktree the host can still verify for opening. */
function openableWorktree(task: DispatchTask): DispatchAttempt | undefined {
  return [...task.attempts]
    .reverse()
    .find((a) => a.worktree?.allocated && a.worktree.directoryIdentity);
}

function TaskReferences({
  task,
  edit,
  open,
  busy,
}: {
  task: DispatchTask;
  edit: (r: TaskEditRequest) => Promise<boolean>;
  open: (r: TaskOpenRequest) => Promise<void>;
  busy: boolean;
}) {
  const [draft, setDraft] = useState(() => createTaskLinksDraft(task.links));
  const links = draft.links;
  const [revision, setRevision] = useState(task.revision ?? 0);
  const [dirty, setDirty] = useState(false);
  const [editing, setEditing] = useState(false);
  const [error, setError] = useState('');
  useEffect(() => {
    if (!dirty) {
      setDraft(createTaskLinksDraft(task.links));
      setRevision(task.revision ?? 0);
    }
  }, [task, dirty]);
  const change = (
    next: TaskLinks,
    removed?: { kind: 'ticketOrigins' | 'referenceOrigins'; index: number },
  ) => {
    setDirty(true);
    setError('');
    setDraft((d) => ({
      ...d,
      links: next,
      ticketOrigins: Array.from(
        { length: next.tickets?.length ?? 0 },
        (_, i) => d.ticketOrigins[i],
      ),
      referenceOrigins: Array.from(
        { length: next.references?.length ?? 0 },
        (_, i) => d.referenceOrigins[i],
      ),
      ...(removed ? { [removed.kind]: d[removed.kind].filter((_, i) => i !== removed.index) } : {}),
    }));
  };
  const conflict = dirty && revision !== (task.revision ?? 0);
  const worktree = openableWorktree(task);
  const branch = worktree?.worktree?.branch;
  const external = <ExternalLink size={10} strokeWidth={2.25} />;
  const hasLinks =
    !!task.links?.pullRequest || !!task.links?.tickets?.length || !!task.links?.references?.length;
  return (
    <section aria-label="Task references">
      <div style={{ display: 'flex', flexWrap: 'wrap', gap: 4, alignItems: 'center' }}>
        {task.links?.pullRequest && (
          <Chip
            label={`PR ${task.links.pullRequest.number ?? 'link'}`}
            icon={task.links.pullRequest.url ? external : undefined}
            title="Recorded reference; edit history in Details. Not verified with the provider."
            onClick={
              task.links.pullRequest.url
                ? () => void open({ taskId: task.taskId, kind: 'url', reference: 'pullRequest' })
                : undefined
            }
          />
        )}
        {task.links?.tickets?.map((t, index) => (
          <Chip
            key={`ticket-${index}`}
            label={t.id}
            icon={t.url ? external : undefined}
            title="Recorded reference; edit history in Details. Not verified with the provider."
            onClick={
              t.url
                ? () => void open({ taskId: task.taskId, kind: 'url', reference: 'tickets', index })
                : undefined
            }
          />
        ))}
        {task.links?.references?.map((r, index) => (
          <Chip
            key={`reference-${index}`}
            label={r.label}
            icon={external}
            title="Recorded reference; edit history in Details. Not verified with the provider."
            onClick={() =>
              void open({ taskId: task.taskId, kind: 'url', reference: 'references', index })
            }
          />
        ))}
        {branch && <Chip label={branch} title={`Recorded branch ${branch}`} />}
        {worktree && (
          <Chip
            label="Open worktree"
            icon={<FolderOpen size={10} strokeWidth={2.25} />}
            onClick={() =>
              void open({ taskId: task.taskId, kind: 'worktree', dispatchId: worktree.dispatchId })
            }
          />
        )}
        {!hasLinks && !branch && !worktree && <span style={meta}>No links recorded yet.</span>}
        <SmallButton
          label={
            <span style={{ display: 'inline-flex', alignItems: 'center', gap: 4 }}>
              <Pencil size={10} strokeWidth={2.25} />
              {editing ? 'Done' : 'Links'}
            </span>
          }
          onClick={() => setEditing((v) => !v)}
        />
      </div>
      {editing && (
        <Surface elevation="flat" pad="md" style={{ marginTop: 6 }}>
          <p style={{ margin: '0 0 6px', ...meta }}>
            Links you or your manager record. Nothing here is checked with the provider.
          </p>
          <label style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
            <span style={overline}>PR number</span>
            <input
              style={field}
              maxLength={10}
              value={links.pullRequest?.number ?? ''}
              onChange={(e) =>
                change({
                  ...links,
                  pullRequest: { ...links.pullRequest, number: e.target.value || undefined },
                })
              }
            />
          </label>
          <label style={{ display: 'flex', flexDirection: 'column', gap: 4, marginTop: 6 }}>
            <span style={overline}>PR URL</span>
            <input
              style={field}
              maxLength={2048}
              value={links.pullRequest?.url ?? ''}
              onChange={(e) =>
                change({
                  ...links,
                  pullRequest: { ...links.pullRequest, url: e.target.value || undefined },
                })
              }
            />
          </label>
          {links.tickets?.map((ticket, index) => (
            <div key={index} style={{ marginTop: 6 }}>
              <label style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
                <span style={overline}>Ticket ID</span>
                <input
                  style={field}
                  maxLength={200}
                  value={ticket.id}
                  onChange={(e) =>
                    change({
                      ...links,
                      tickets: links.tickets!.map((r, i) =>
                        i === index ? { ...r, id: e.target.value } : r,
                      ),
                    })
                  }
                />
              </label>
              <label style={{ display: 'flex', flexDirection: 'column', gap: 4, marginTop: 4 }}>
                <span style={overline}>Ticket URL (optional)</span>
                <input
                  style={field}
                  maxLength={2048}
                  value={ticket.url ?? ''}
                  onChange={(e) =>
                    change({
                      ...links,
                      tickets: links.tickets!.map((r, i) =>
                        i === index ? { ...r, url: e.target.value || undefined } : r,
                      ),
                    })
                  }
                />
              </label>
              <div style={{ marginTop: 4 }}>
                <SmallButton
                  label="Remove ticket"
                  onClick={() =>
                    change(
                      { ...links, tickets: links.tickets!.filter((_, i) => i !== index) },
                      { kind: 'ticketOrigins', index },
                    )
                  }
                />
              </div>
            </div>
          ))}
          {links.references?.map((reference, index) => (
            <div key={index} style={{ marginTop: 6 }}>
              <label style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
                <span style={overline}>Reference label</span>
                <input
                  style={field}
                  maxLength={200}
                  value={reference.label}
                  onChange={(e) =>
                    change({
                      ...links,
                      references: links.references!.map((r, i) =>
                        i === index ? { ...r, label: e.target.value } : r,
                      ),
                    })
                  }
                />
              </label>
              <label style={{ display: 'flex', flexDirection: 'column', gap: 4, marginTop: 4 }}>
                <span style={overline}>Reference URL</span>
                <input
                  style={field}
                  maxLength={2048}
                  value={reference.url}
                  onChange={(e) =>
                    change({
                      ...links,
                      references: links.references!.map((r, i) =>
                        i === index ? { ...r, url: e.target.value } : r,
                      ),
                    })
                  }
                />
              </label>
              <div style={{ marginTop: 4 }}>
                <SmallButton
                  label="Remove reference"
                  onClick={() =>
                    change(
                      {
                        ...links,
                        references: links.references!.filter((_, i) => i !== index),
                      },
                      { kind: 'referenceOrigins', index },
                    )
                  }
                />
              </div>
            </div>
          ))}
          <div style={{ display: 'flex', flexWrap: 'wrap', gap: 6, marginTop: 8 }}>
            <SmallButton
              label="Add ticket"
              disabled={(links.tickets?.length ?? 0) >= 20}
              onClick={() => change({ ...links, tickets: [...(links.tickets ?? []), { id: '' }] })}
            />
            <SmallButton
              label="Add reference"
              disabled={(links.references?.length ?? 0) >= 20}
              onClick={() =>
                change({
                  ...links,
                  references: [...(links.references ?? []), { label: '', url: '' }],
                })
              }
            />
          </div>
          {error && (
            <p role="alert" style={{ margin: '6px 0 0', color: 'var(--wks-error)' }}>
              {error}
            </p>
          )}
          {conflict && (
            <p role="status" style={{ margin: '6px 0 0', ...meta }}>
              This task changed. Your draft is preserved.{' '}
              <SmallButton
                label="Keep draft on current task"
                disabled={busy}
                onClick={() => {
                  try {
                    const rebased = rebaseTaskLinksDraft(draft, task.links);
                    setDraft(rebased);
                    setRevision(task.revision ?? 0);
                    setError('');
                  } catch (e) {
                    setError(String(e));
                  }
                }}
              />
            </p>
          )}
          <div style={{ display: 'flex', flexWrap: 'wrap', gap: 6, marginTop: 8 }}>
            <SmallButton
              label="Save references"
              primary
              disabled={busy || !dirty || conflict}
              onClick={() => {
                try {
                  const valid = normalizedDraftLinks(links);
                  setError('');
                  void edit({
                    taskId: task.taskId,
                    expectedTaskRevision: revision,
                    action: 'links',
                    links: valid,
                  }).then((ok) => {
                    if (ok) setDirty(false);
                  });
                } catch (e) {
                  setError(String(e));
                }
              }}
            />
            <SmallButton
              label="Reload references"
              disabled={busy}
              onClick={() => {
                setDraft(createTaskLinksDraft(task.links));
                setRevision(task.revision ?? 0);
                setDirty(false);
                setError('');
              }}
            />
          </div>
        </Surface>
      )}
    </section>
  );
}
