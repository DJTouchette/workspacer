import React, { useCallback, useEffect, useRef, useState } from 'react';
import { Surface } from './Surface';
import { inputStyle, SmallButton } from './settings/primitives';
import FleetWorkflowTask from './FleetWorkflowTask';
import { openTaskWorkflowSettings } from '../lib/settingsBus';
import {
  TASK_INSPECTOR_UNAVAILABLE,
  taskIsActive,
  taskSkipDisabledReason,
  validateTaskLinks,
  type DispatchTask,
  type DispatchHistoryResponse,
  type TaskLinks,
  type TaskEditRequest,
  type TaskOpenRequest,
} from '../../../main/shared/dispatchHistory';

const field: React.CSSProperties = { ...inputStyle, width: '100%', boxSizing: 'border-box' };
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
  return (
    <section
      aria-label="Task Inspector"
      style={{
        padding: 12,
        overflow: 'auto',
        minWidth: 0,
        minHeight: 0,
        flex: 1,
        fontFamily: 'var(--wks-font-sans)',
        fontSize: '0.8rem',
        color: 'var(--wks-text-primary)',
        overflowWrap: 'anywhere',
      }}
    >
      <h3 style={{ fontSize: '0.9rem', marginTop: 0 }}>Tasks</h3>
      {error && (
        <p role="alert">
          Could not refresh tasks. {error}{' '}
          <SmallButton label="Retry" onClick={() => void reload()} />
        </p>
      )}
      {!data && !error && <p role="status">Loading tasks…</p>}
      {data && !data.available && <p role="status">{data.reason}</p>}
      {data?.available && (
        <>
          {!data.currentOwnerSessionId && !sessionId && (
            <p>No current manager. Showing recorded tasks.</p>
          )}
          {sessionId && !isManager && (
            <p>Tasks linked to worker {sessionId} by a recorded attempt.</p>
          )}
          <label>
            <input
              type="checkbox"
              checked={all}
              onChange={(e) => {
                setAll(e.target.checked);
                setSelected('');
              }}
            />{' '}
            All recorded tasks
          </label>
          <label style={{ display: 'block', marginTop: 8 }}>
            Project
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
              {Array.from(
                new Set([
                  ...tasks.map((t) => t.projectCwd),
                  ...(selectedProject ? [selectedProject] : []),
                ]),
              ).map((p) => (
                <option key={p}>{p}</option>
              ))}
            </select>
          </label>
          <label style={{ display: 'block', marginTop: 8 }}>
            Current and recent tasks
            <select
              aria-label="Current and recent tasks"
              style={field}
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
                        {t.title} · {t.ownerLabel}
                      </option>
                    ))}
                </optgroup>
              ))}
            </select>
          </label>
          {!choices.length && (
            <p>
              No recorded tasks match this selection. Task ownership is shown only when recorded.
            </p>
          )}
          {selected && !task && (
            <p role="status">
              The selected task is no longer available in this selection. Select another task or
              reload.
            </p>
          )}
          {task && (
            <TaskDetails
              key={task.taskId}
              task={task}
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
  workerId,
  update,
  reload,
  stale,
}: {
  task: DispatchTask;
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
  return (
    <>
      <h3 style={{ fontSize: '0.9rem' }}>{task.title}</h3>
      <p>
        {taskIsActive(task) ? 'Current task' : 'Recent task'} · {task.taskId}
        <br />
        Manager: {task.ownerLabel} ({task.ownerSessionId})<br />
        Project: {task.projectCwd}
      </p>
      {error && (
        <p role="alert">
          {error} <SmallButton label="Reload task" onClick={() => void reload()} />
        </p>
      )}
      {task.workflow ? (
        <>
          <p>
            {task.workflow.definition.name} · version {task.workflow.definition.revision}
          </p>
          <ol style={{ paddingLeft: 20 }}>
            {task.workflow.steps.map((run, i) => {
              const definition = task.workflow!.definition.steps[i];
              const audit = task.audit?.find((a) => a.id === run.waiverId);
              const disabled = stale
                ? 'Refresh tasks before making changes'
                : taskSkipDisabledReason(task, run.id);
              return (
                <li key={run.id} style={{ marginBottom: 12 }}>
                  <strong>{definition.label}</strong> ·{' '}
                  {{
                    planned: 'Planned',
                    dispatched: 'Working',
                    blocked: 'Needs attention',
                    failed: 'Failed',
                    completed: 'Result received',
                    skipped: 'Skipped by manager',
                    waived: 'Skipped by you',
                  }[run.state] ?? 'Unknown status'}
                  {definition.kind === 'review' && <span> · Required review</span>}
                  {run.sessionId && (
                    <p>
                      Worker {run.sessionId}
                      <br />
                      Attempt {run.dispatchId ?? 'unknown'}
                    </p>
                  )}
                  {audit && (
                    <p>
                      Skipped by you · {new Date(audit.createdAt).toLocaleString()}
                      <br />
                      {audit.reason}
                    </p>
                  )}
                  {run.reason && <p>{run.reason}</p>}
                  {run.outcome !== undefined && (
                    <details>
                      <summary>Reported outcome</summary>
                      <pre style={{ whiteSpace: 'pre-wrap' }}>
                        {JSON.stringify(run.outcome, null, 2)}
                      </pre>
                    </details>
                  )}
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
                  {disabled && <div style={{ color: 'var(--wks-text-secondary)' }}>{disabled}</div>}
                </li>
              );
            })}
          </ol>
          {skip && (
            <Surface elevation="flat" style={{ padding: 12 }}>
              <div role="dialog" aria-label="Skip task step" aria-modal="false">
                <strong>
                  Skip {task.workflow.definition.steps.find((s) => s.id === skip.stepId)?.label}?
                </strong>
                <p>
                  This affects this task only. It does not mark the step as passed or reviewed.
                  Earlier unfinished steps still need to finish.
                </p>
                {task.workflow.definition.steps.find((s) => s.id === skip.stepId)?.kind ===
                  'review' && <p>This skips the required independent review for this task.</p>}
                <label>
                  Reason (optional)
                  <input
                    style={field}
                    maxLength={2000}
                    value={skip.reason}
                    onChange={(e) => setSkip({ ...skip, reason: e.target.value })}
                  />
                </label>
                {skip.revision !== (task.revision ?? 0) && (
                  <p role="status">
                    This task changed. Refresh this confirmation before skipping.
                    <SmallButton
                      label="Refresh confirmation"
                      onClick={() => setSkip({ ...skip, revision: task.revision ?? 0 })}
                    />
                  </p>
                )}
                {taskSkipDisabledReason(task, skip.stepId) && (
                  <p role="status">{taskSkipDisabledReason(task, skip.stepId)}</p>
                )}
                <SmallButton
                  label="Skip this step"
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
                />{' '}
                <SmallButton label="Cancel" disabled={busy} onClick={() => setSkip(undefined)} />
              </div>
            </Surface>
          )}
          <FleetWorkflowTask task={task} />
        </>
      ) : (
        <p>No workflow was recorded for this task.</p>
      )}
      <h4>Recorded work</h4>
      {!task.attempts.length && <p>No dispatches recorded yet.</p>}
      {task.attempts.map((a) => (
        <details key={a.dispatchId} open={a.sessionId === workerId}>
          <summary>
            {a.stage ?? 'Attempt'} · {a.sessionId === workerId ? 'Selected worker · ' : ''}
            {a.stale ? 'Status unknown — last recorded' : a.lifecycle}
          </summary>
          <p>
            Worker {a.sessionId}
            <br />
            Attempt {a.dispatchId}
            <br />
            Execution folder: {a.executionCwd}
            <br />
            Branch: {a.worktree?.branch ?? 'Not recorded'}
          </p>
          {a.worktree?.fallback && (
            <p>Worktree allocation fell back to the project folder. {a.worktree.error}</p>
          )}
          {a.worktree?.allocated && (
            <SmallButton
              label="Open worktree"
              disabled={!a.worktree.directoryIdentity}
              onClick={() =>
                void open({ taskId: task.taskId, kind: 'worktree', dispatchId: a.dispatchId })
              }
            />
          )}
          {a.worktree?.allocated && !a.worktree.directoryIdentity && (
            <p>This older record cannot verify the worktree folder for opening.</p>
          )}
        </details>
      ))}
      <TaskReferences task={task} edit={edit} open={open} busy={busy || stale} />
      <p>
        <SmallButton
          label="Configure workflow"
          onClick={() => openTaskWorkflowSettings(task.projectCwd)}
        />
        <br />
        Changes in Settings apply to new tasks only. This task keeps its recorded workflow.
      </p>
    </>
  );
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
  const [links, setLinks] = useState<TaskLinks>(task.links ?? {});
  const [revision, setRevision] = useState(task.revision ?? 0);
  const [dirty, setDirty] = useState(false);
  const [error, setError] = useState('');
  useEffect(() => {
    if (!dirty) {
      setLinks(task.links ?? {});
      setRevision(task.revision ?? 0);
    }
  }, [task, dirty]);
  const change = (next: TaskLinks) => {
    setDirty(true);
    setLinks(next);
  };
  const conflict = dirty && revision !== (task.revision ?? 0);
  return (
    <section aria-label="Task references">
      <h4>Your references</h4>
      <p>Entered by you; external services have not verified these links.</p>
      {!task.links || !Object.keys(task.links).length ? <p>No references recorded.</p> : null}
      {task.links?.pullRequest && (
        <p>
          PR {task.links.pullRequest.number ?? ''}{' '}
          {task.links.pullRequest.url && (
            <SmallButton
              label="Open PR"
              onClick={() =>
                void open({ taskId: task.taskId, kind: 'url', reference: 'pullRequest' })
              }
            />
          )}
        </p>
      )}
      {(['tickets', 'references'] as const).map((key) =>
        task.links?.[key]?.map((r, index) => (
          <p key={`${key}-${index}`}>
            {'id' in r ? r.id : r.label}{' '}
            {r.url && (
              <SmallButton
                label={`Open ${'id' in r ? r.id : r.label}`}
                onClick={() =>
                  void open({ taskId: task.taskId, kind: 'url', reference: key, index })
                }
              />
            )}
          </p>
        )),
      )}
      <label>
        PR number
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
      <label>
        PR URL
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
        <div key={index} style={{ marginTop: 8 }}>
          <label>
            Ticket ID
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
          <label>
            Ticket URL (optional)
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
          <SmallButton
            label="Remove ticket"
            onClick={() =>
              change({ ...links, tickets: links.tickets!.filter((_, i) => i !== index) })
            }
          />
        </div>
      ))}
      <SmallButton
        label="Add ticket"
        disabled={(links.tickets?.length ?? 0) >= 20}
        onClick={() => change({ ...links, tickets: [...(links.tickets ?? []), { id: '' }] })}
      />
      {links.references?.map((reference, index) => (
        <div key={index} style={{ marginTop: 8 }}>
          <label>
            Reference label
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
          <label>
            Reference URL
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
          <SmallButton
            label="Remove reference"
            onClick={() =>
              change({ ...links, references: links.references!.filter((_, i) => i !== index) })
            }
          />
        </div>
      ))}
      <SmallButton
        label="Add reference"
        disabled={(links.references?.length ?? 0) >= 20}
        onClick={() =>
          change({ ...links, references: [...(links.references ?? []), { label: '', url: '' }] })
        }
      />
      {error && <p role="alert">{error}</p>}
      {conflict && (
        <p role="status">
          This task changed. Your draft is preserved.{' '}
          <SmallButton
            label="Keep draft on current task"
            onClick={() => setRevision(task.revision ?? 0)}
          />
        </p>
      )}
      <p>
        <SmallButton
          label="Save references"
          disabled={busy || !dirty || conflict}
          onClick={() => {
            try {
              const normalized = { ...links };
              if (!normalized.pullRequest?.number && !normalized.pullRequest?.url)
                delete normalized.pullRequest;
              const valid = validateTaskLinks(normalized);
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
        />{' '}
        <SmallButton
          label="Reload references"
          disabled={busy}
          onClick={() => {
            setLinks(task.links ?? {});
            setRevision(task.revision ?? 0);
            setDirty(false);
            setError('');
          }}
        />
      </p>
    </section>
  );
}
