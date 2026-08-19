import React, { useCallback, useEffect, useRef, useState } from 'react';
import { ChevronDown, ChevronRight, FolderOpen, Play, X } from 'lucide-react';
import { Section, SmallButton, inputStyle } from './primitives';
import type { HubJob, HubJobRun, HubJobView } from '../../../../main/shared/ipcTypes';

/**
 * Jobs — recurring and one-off tasks the hub runs on your behalf: spawn an
 * agent with a prompt, call a bus capability, or run a shell command, on an
 * interval, at a daily time, once, or manually. The hub owns storage,
 * validation and scheduling (services/hub/internal/jobs); this section is a
 * thin editor over the trusted-only jobs.* RPCs. The list polls while the
 * section is open so run-state chips stay roughly live without any new bus
 * topic; run-now flips the chip optimistically so the click lands instantly.
 */

const DAY_LABELS = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'];

function triggerSummary(j: HubJobView): string {
  const t = j.trigger;
  switch (t.kind) {
    case 'interval': {
      const m = t.everyMinutes ?? 0;
      return m % 60 === 0 && m > 0 ? `every ${m / 60}h` : `every ${m}m`;
    }
    case 'daily': {
      const days =
        t.days && t.days.length > 0
          ? ' · ' + t.days.map((d) => DAY_LABELS[d] ?? d).join(' ')
          : '';
      return `daily ${t.at}${days}`;
    }
    case 'once':
      return t.once ? `once, ${new Date(t.once).toLocaleString()}` : 'once';
    case 'manual':
      return 'manual';
  }
}

function actionSummary(j: HubJobView): string {
  const a = j.action;
  switch (a.kind) {
    case 'spawn':
      return `agent in ${a.spawn?.cwd ?? '?'}`;
    case 'call':
      return `call ${a.call?.method ?? '?'}`;
    case 'shell':
      return `$ ${a.shell?.command ?? '?'}`;
  }
}

function ago(ms?: number): string {
  if (!ms) return '';
  const mins = Math.round((Date.now() - ms) / 60000);
  if (mins < 1) return 'just now';
  if (mins < 60) return `${mins}m ago`;
  const h = Math.round(mins / 60);
  return h < 24 ? `${h}h ago` : `${Math.round(h / 24)}d ago`;
}

function inFuture(ms?: number): string {
  if (!ms) return '';
  const mins = Math.round((ms - Date.now()) / 60000);
  if (mins < 1) return 'due now';
  if (mins < 60) return `in ${mins}m`;
  const h = Math.round(mins / 60);
  return h < 24 ? `in ${h}h` : `in ${Math.round(h / 24)}d`;
}

function duration(r: HubJobRun): string {
  if (!r.finishedAt || r.finishedAt <= r.startedAt) return '';
  const s = Math.round((r.finishedAt - r.startedAt) / 1000);
  if (s < 1) return '<1s';
  if (s < 60) return `${s}s`;
  return `${Math.floor(s / 60)}m ${s % 60}s`;
}

const RUN_COLORS: Record<HubJobRun['status'], string> = {
  ok: 'var(--wks-success)',
  error: 'var(--wks-error)',
  skipped: 'var(--wks-text-faint)',
};

/** Colored status dot — the app-wide status-token idiom, not an icon. */
const Dot: React.FC<{ color: string; pulse?: boolean }> = ({ color, pulse }) => (
  <span
    style={{
      width: 6,
      height: 6,
      borderRadius: '50%',
      background: color,
      flexShrink: 0,
      animation: pulse ? 'wks-pulse 1.2s ease-in-out infinite' : undefined,
    }}
  />
);

/** Last-run status chip: running > error > skipped > ok. */
const RunChip: React.FC<{ j: HubJobView; optimisticRunning: boolean }> = ({
  j,
  optimisticRunning,
}) => {
  if (j.running || optimisticRunning) {
    return (
      <span
        style={{
          display: 'inline-flex',
          alignItems: 'center',
          gap: 4,
          fontSize: '0.6rem',
          color: 'var(--wks-busy)',
          flexShrink: 0,
        }}
      >
        <Dot color="var(--wks-busy)" pulse />
        running
      </span>
    );
  }
  if (!j.lastRun) return null;
  const color = RUN_COLORS[j.lastRun.status];
  const label =
    j.lastRun.status === 'error'
      ? 'failed'
      : j.lastRun.status === 'skipped'
        ? 'skipped'
        : 'ok';
  return (
    <span
      title={j.lastRun.detail}
      style={{
        display: 'inline-flex',
        alignItems: 'center',
        gap: 4,
        fontSize: '0.6rem',
        color,
        flexShrink: 0,
      }}
    >
      <Dot color={color} />
      {label} {ago(j.lastRun.finishedAt ?? j.lastRun.startedAt)}
    </span>
  );
};

/** Expanded per-job run history, fetched on open. */
const RunHistory: React.FC<{ jobId: string; refreshKey: number }> = ({ jobId, refreshKey }) => {
  const [runs, setRuns] = useState<HubJobRun[] | null>(null);
  useEffect(() => {
    let live = true;
    window.electronAPI
      .jobsHistory(jobId)
      .then((res) => {
        if (live) setRuns(res?.runs ?? []);
      })
      .catch(() => {
        if (live) setRuns([]);
      });
    return () => {
      live = false;
    };
  }, [jobId, refreshKey]);

  if (runs === null) {
    return (
      <div style={{ fontSize: '0.66rem', color: 'var(--wks-text-faint)', padding: '4px 0 4px 24px' }}>
        Loading runs…
      </div>
    );
  }
  if (runs.length === 0) {
    return (
      <div style={{ fontSize: '0.66rem', color: 'var(--wks-text-faint)', padding: '4px 0 4px 24px' }}>
        No runs yet.
      </div>
    );
  }
  return (
    <div
      style={{
        display: 'flex',
        flexDirection: 'column',
        gap: 2,
        padding: '2px 0 6px 24px',
      }}
    >
      {runs.slice(0, 8).map((r, i) => (
        <div
          key={`${r.startedAt}-${i}`}
          title={r.detail}
          style={{ display: 'flex', alignItems: 'center', gap: 6, minWidth: 0 }}
        >
          <Dot color={RUN_COLORS[r.status]} />
          <span
            style={{
              fontSize: '0.66rem',
              color: 'var(--wks-text-muted)',
              fontVariantNumeric: 'tabular-nums',
              flexShrink: 0,
            }}
          >
            {new Date(r.startedAt).toLocaleString()}
          </span>
          {duration(r) && (
            <span style={{ fontSize: '0.6rem', color: 'var(--wks-text-faint)', flexShrink: 0 }}>
              {duration(r)}
            </span>
          )}
          {r.detail && (
            <span
              style={{
                fontSize: '0.66rem',
                color: r.status === 'error' ? 'var(--wks-error)' : 'var(--wks-text-faint)',
                fontFamily: 'var(--wks-font-mono)',
                overflow: 'hidden',
                textOverflow: 'ellipsis',
                whiteSpace: 'nowrap',
                minWidth: 0,
              }}
            >
              {r.detail}
            </span>
          )}
        </div>
      ))}
    </div>
  );
};

/** Editable state, flat for form inputs; toJob() rebuilds the wire shape. */
interface Draft {
  id: string;
  name: string;
  triggerKind: 'interval' | 'daily' | 'once' | 'manual';
  everyMinutes: string;
  at: string;
  days: number[];
  once: string;
  actionKind: 'spawn' | 'call' | 'shell';
  cwd: string;
  prompt: string;
  provider: string;
  model: string;
  method: string;
  params: string;
  command: string;
  shellCwd: string;
}

const EMPTY_DRAFT: Draft = {
  id: '',
  name: '',
  triggerKind: 'daily',
  everyMinutes: '60',
  at: '09:00',
  days: [],
  once: '',
  actionKind: 'spawn',
  cwd: '',
  prompt: '',
  provider: 'claude',
  model: '',
  method: '',
  params: '',
  command: '',
  shellCwd: '',
};

/** Empty-state starters: one click pre-fills the editor with a sensible shape
 *  (nothing is saved until the user hits Create). */
const TEMPLATES: Array<{ label: string; hint: string; draft: Partial<Draft> }> = [
  {
    label: 'Morning triage agent',
    hint: 'Weekdays at 9:00, an agent triages the repo and leaves a summary',
    draft: {
      name: 'Morning triage',
      triggerKind: 'daily',
      at: '09:00',
      days: [1, 2, 3, 4, 5],
      actionKind: 'spawn',
      prompt:
        'Triage anything that happened overnight in this repo — new issues, failed CI, stale branches — and finish with a short summary of what needs my attention.',
    },
  },
  {
    label: 'Command on a timer',
    hint: 'A shell command every hour',
    draft: { name: '', triggerKind: 'interval', everyMinutes: '60', actionKind: 'shell' },
  },
  {
    label: 'One-off agent',
    hint: 'Spawn an agent once, at a time you pick',
    draft: { name: '', triggerKind: 'once', actionKind: 'spawn' },
  },
];

function toDraft(j: HubJobView): Draft {
  return {
    id: j.id,
    name: j.name,
    triggerKind: j.trigger.kind,
    everyMinutes: String(j.trigger.everyMinutes ?? 60),
    at: j.trigger.at ?? '09:00',
    days: j.trigger.days ?? [],
    // datetime-local wants "YYYY-MM-DDTHH:MM" in local time.
    once: j.trigger.once ? isoToLocalInput(j.trigger.once) : '',
    actionKind: j.action.kind,
    cwd: j.action.spawn?.cwd ?? '',
    prompt: j.action.spawn?.prompt ?? '',
    provider: j.action.spawn?.provider ?? 'claude',
    model: j.action.spawn?.model ?? '',
    method: j.action.call?.method ?? '',
    params: j.action.call?.params ? JSON.stringify(j.action.call.params) : '',
    command: j.action.shell?.command ?? '',
    shellCwd: j.action.shell?.cwd ?? '',
  };
}

function isoToLocalInput(iso: string): string {
  const d = new Date(iso);
  if (isNaN(d.getTime())) return '';
  const pad = (n: number) => String(n).padStart(2, '0');
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

/** What still blocks saving, or null when the draft is complete. Mirrors the
 *  hub's Validate loosely — the hub stays authoritative. */
function draftProblem(d: Draft): string | null {
  if (d.triggerKind === 'interval' && !(Number(d.everyMinutes) >= 1))
    return 'Interval needs a minute count.';
  if (d.triggerKind === 'daily' && !d.at) return 'Pick a time of day.';
  if (d.triggerKind === 'once' && !d.once) return 'Pick a date and time.';
  if (d.actionKind === 'spawn') {
    if (!d.cwd.trim()) return 'Pick the directory the agent should work in.';
    if (!d.prompt.trim()) return 'Write the prompt the agent starts with.';
  }
  if (d.actionKind === 'shell' && !d.command.trim()) return 'Enter the command to run.';
  if (d.actionKind === 'call') {
    if (!d.method.trim()) return 'Name the capability method.';
    if (d.params.trim()) {
      try {
        JSON.parse(d.params);
      } catch {
        return 'Params must be valid JSON.';
      }
    }
  }
  return null;
}

/** Draft → wire job. Callers gate on draftProblem() first. */
function toJob(d: Draft, enabled: boolean): HubJob | Omit<HubJob, 'id'> {
  const trigger: HubJob['trigger'] = { kind: d.triggerKind };
  if (d.triggerKind === 'interval') trigger.everyMinutes = Math.max(1, Number(d.everyMinutes) || 0);
  if (d.triggerKind === 'daily') {
    trigger.at = d.at;
    if (d.days.length > 0) trigger.days = d.days;
  }
  if (d.triggerKind === 'once') trigger.once = new Date(d.once).toISOString();
  const action: HubJob['action'] = { kind: d.actionKind };
  if (d.actionKind === 'spawn') {
    action.spawn = {
      cwd: d.cwd.trim(),
      prompt: d.prompt.trim(),
      provider: d.provider || undefined,
      model: d.model.trim() || undefined,
    };
  } else if (d.actionKind === 'call') {
    action.call = {
      method: d.method.trim(),
      params: d.params.trim() ? JSON.parse(d.params) : undefined,
    };
  } else {
    action.shell = { command: d.command.trim(), cwd: d.shellCwd.trim() || undefined };
  }
  const base = { name: d.name.trim() || 'Job', enabled, trigger, action };
  return d.id ? { id: d.id, ...base } : base;
}

const JobsSection: React.FC = () => {
  const [jobs, setJobs] = useState<HubJobView[]>([]);
  const [available, setAvailable] = useState(true);
  const [editing, setEditing] = useState<Draft | null>(null);
  const [expanded, setExpanded] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [historyKey, setHistoryKey] = useState(0);
  // Optimistic run-state: id → when Run-now was clicked. Cleared once the
  // polled truth reports running (or 15s pass — sub-tick runs finish fast).
  const ranAtRef = useRef<Map<string, number>>(new Map());

  const load = useCallback(() => {
    window.electronAPI
      .jobsList()
      .then((res) => {
        const list = res?.jobs ?? [];
        for (const j of list) {
          if (j.running) ranAtRef.current.delete(j.id);
        }
        setJobs(list);
        setHistoryKey((k) => k + 1);
      })
      // A hub without jobs (older build, or a view/triage web token) —
      // show the plain unavailable note rather than a broken editor.
      .catch(() => setAvailable(false));
  }, []);

  useEffect(() => {
    load();
    const t = setInterval(load, 10_000);
    return () => clearInterval(t);
  }, [load]);

  const save = async (d: Draft) => {
    setError(null);
    try {
      const existing = jobs.find((j) => j.id === d.id);
      await window.electronAPI.jobsUpsert(toJob(d, existing ? existing.enabled : true));
      setEditing(null);
      load();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  };

  const toggle = async (j: HubJobView) => {
    const { nextRunAt, lastRun, running, ...job } = j;
    void nextRunAt;
    void lastRun;
    void running;
    await window.electronAPI.jobsUpsert({ ...job, enabled: !j.enabled }).catch(() => {});
    load();
  };

  const runNow = async (id: string) => {
    ranAtRef.current.set(id, Date.now());
    setJobs((prev) => [...prev]); // repaint the chip immediately
    await window.electronAPI.jobsRun(id).catch(() => {});
    load();
  };

  const remove = async (id: string) => {
    await window.electronAPI.jobsRemove(id).catch(() => {});
    if (expanded === id) setExpanded(null);
    load();
  };

  const optimistic = (id: string): boolean => {
    const at = ranAtRef.current.get(id);
    return !!at && Date.now() - at < 15_000;
  };

  if (!available) {
    return (
      <Section title="Jobs">
        <div style={{ fontSize: '0.72rem', color: 'var(--wks-text-muted)', lineHeight: 1.5 }}>
          Jobs are managed by the hub and aren't available on this connection.
        </div>
      </Section>
    );
  }

  return (
    <Section title="Jobs">
      <div style={{ fontSize: '0.72rem', color: 'var(--wks-text-muted)', lineHeight: 1.5 }}>
        Recurring or one-off tasks the hub runs for you — spawn an agent with a prompt, run a
        shell command, or call a bus capability. Jobs keep running while this window is closed;
        click a row for its run history.
      </div>

      <div style={{ display: 'flex', flexDirection: 'column', gap: 4, marginTop: 8 }}>
        {jobs.map((j) =>
          editing?.id === j.id ? (
            <JobEditForm
              key={j.id}
              draft={editing}
              onChange={setEditing}
              onSave={() => void save(editing)}
              onCancel={() => {
                setEditing(null);
                setError(null);
              }}
            />
          ) : (
            <div key={j.id}>
              <div
                onClick={() => setExpanded(expanded === j.id ? null : j.id)}
                style={{
                  display: 'flex',
                  alignItems: 'center',
                  gap: 8,
                  padding: '4px 8px',
                  borderRadius: 'var(--wks-radius-sm)',
                  cursor: 'pointer',
                  opacity: j.enabled ? 1 : 0.55,
                }}
                onMouseEnter={(e) => {
                  (e.currentTarget as HTMLElement).style.backgroundColor = 'var(--wks-bg-hover)';
                }}
                onMouseLeave={(e) => {
                  (e.currentTarget as HTMLElement).style.backgroundColor = 'transparent';
                }}
              >
                <span
                  style={{
                    display: 'flex',
                    alignItems: 'center',
                    color: 'var(--wks-text-faint)',
                    flexShrink: 0,
                  }}
                >
                  {expanded === j.id ? <ChevronDown size={12} /> : <ChevronRight size={12} />}
                </span>
                <input
                  type="checkbox"
                  checked={j.enabled}
                  onClick={(e) => e.stopPropagation()}
                  onChange={() => void toggle(j)}
                  title={j.enabled ? 'Disable' : 'Enable'}
                  style={{ cursor: 'pointer', flexShrink: 0, accentColor: 'var(--wks-accent)' }}
                />
                <div style={{ flex: 1, minWidth: 0 }}>
                  <div
                    style={{
                      fontSize: '0.72rem',
                      fontWeight: 500,
                      color: 'var(--wks-text-secondary)',
                      display: 'flex',
                      gap: 8,
                      alignItems: 'baseline',
                      minWidth: 0,
                    }}
                  >
                    <span
                      style={{
                        overflow: 'hidden',
                        textOverflow: 'ellipsis',
                        whiteSpace: 'nowrap',
                      }}
                    >
                      {j.name}
                    </span>
                    <RunChip j={j} optimisticRunning={optimistic(j.id)} />
                  </div>
                  <div
                    title={
                      j.enabled && j.nextRunAt
                        ? `Next run ${new Date(j.nextRunAt).toLocaleString()}`
                        : undefined
                    }
                    style={{
                      fontSize: '0.66rem',
                      color: 'var(--wks-text-faint)',
                      fontFamily: 'var(--wks-font-mono)',
                      overflow: 'hidden',
                      textOverflow: 'ellipsis',
                      whiteSpace: 'nowrap',
                    }}
                  >
                    {triggerSummary(j)}
                    {j.enabled && j.nextRunAt ? ` (${inFuture(j.nextRunAt)})` : ''} ·{' '}
                    {actionSummary(j)}
                  </div>
                </div>
                <span onClick={(e) => e.stopPropagation()} style={{ display: 'flex', gap: 4 }}>
                  <SmallButton
                    label={
                      <span
                        style={{ display: 'inline-flex', alignItems: 'center', gap: 4 }}
                        title="Run now"
                      >
                        <Play size={11} strokeWidth={2} />
                      </span>
                    }
                    onClick={() => void runNow(j.id)}
                  />
                  <SmallButton label="Edit" onClick={() => setEditing(toDraft(j))} />
                  <SmallButton
                    label={<X size={11} strokeWidth={2} />}
                    onClick={() => void remove(j.id)}
                    danger
                  />
                </span>
              </div>
              {expanded === j.id && <RunHistory jobId={j.id} refreshKey={historyKey} />}
            </div>
          ),
        )}

        {error && (
          <div
            style={{
              fontSize: '0.66rem',
              color: 'var(--wks-error)',
              padding: '4px 8px',
              borderRadius: 'var(--wks-radius-sm)',
              background: 'color-mix(in srgb, var(--wks-error) 8%, transparent)',
            }}
          >
            {error}
          </div>
        )}

        {editing && !editing.id && (
          <JobEditForm
            draft={editing}
            onChange={setEditing}
            onSave={() => void save(editing)}
            onCancel={() => {
              setEditing(null);
              setError(null);
            }}
          />
        )}

        {!editing && jobs.length === 0 && (
          <div
            style={{
              display: 'flex',
              flexDirection: 'column',
              gap: 6,
              padding: '10px 12px',
              borderRadius: 'var(--wks-radius-md)',
              border: '1px dashed var(--wks-border-input)',
            }}
          >
            <div style={{ fontSize: '0.72rem', color: 'var(--wks-text-secondary)' }}>
              No jobs yet. Start from a template:
            </div>
            <div style={{ display: 'flex', gap: 6, flexWrap: 'wrap' }}>
              {TEMPLATES.map((tpl) => (
                <button
                  key={tpl.label}
                  title={tpl.hint}
                  onClick={() => {
                    setError(null);
                    setEditing({ ...EMPTY_DRAFT, ...tpl.draft });
                  }}
                  style={{
                    padding: '4px 10px',
                    fontSize: '0.66rem',
                    fontFamily: 'inherit',
                    fontWeight: 500,
                    background: 'var(--wks-bg-surface)',
                    color: 'var(--wks-text-secondary)',
                    border: '1px solid var(--wks-border-input)',
                    borderRadius: 'var(--wks-radius-pill)',
                    cursor: 'pointer',
                  }}
                >
                  {tpl.label}
                </button>
              ))}
            </div>
          </div>
        )}

        {!editing && (
          <button
            onClick={() => {
              setError(null);
              setEditing({ ...EMPTY_DRAFT });
            }}
            style={{
              padding: '6px 12px',
              fontSize: '0.66rem',
              fontFamily: 'inherit',
              fontWeight: 500,
              backgroundColor: 'transparent',
              color: 'var(--wks-text-muted)',
              border: '1px dashed var(--wks-border-input)',
              borderRadius: 'var(--wks-radius-sm)',
              cursor: 'pointer',
              lineHeight: 1.4,
              margin: '4px 0 0',
              width: '100%',
            }}
            onMouseEnter={(e) => {
              (e.currentTarget as HTMLElement).style.borderColor = 'var(--wks-accent)';
              (e.currentTarget as HTMLElement).style.color = 'var(--wks-text-secondary)';
            }}
            onMouseLeave={(e) => {
              (e.currentTarget as HTMLElement).style.borderColor = 'var(--wks-border-input)';
              (e.currentTarget as HTMLElement).style.color = 'var(--wks-text-muted)';
            }}
          >
            + Add Job
          </button>
        )}
      </div>
    </Section>
  );
};

const selectStyle: React.CSSProperties = {
  ...inputStyle,
  cursor: 'pointer',
};

/** Directory input with the native folder picker beside it (desktop only —
 *  pickFolder is host-only, so the button hides on the pure web client). */
const CwdInput: React.FC<{
  value: string;
  placeholder: string;
  onChange: (v: string) => void;
}> = ({ value, placeholder, onChange }) => (
  <div style={{ display: 'flex', gap: 4, flex: 1, minWidth: 0 }}>
    <input
      value={value}
      onChange={(e) => onChange(e.target.value)}
      placeholder={placeholder}
      style={{ ...inputStyle, flex: 1, fontFamily: 'var(--wks-font-mono)' }}
    />
    {typeof window.electronAPI.pickFolder === 'function' && (
      <SmallButton
        label={
          <span style={{ display: 'inline-flex', alignItems: 'center' }} title="Browse…">
            <FolderOpen size={12} strokeWidth={2} />
          </span>
        }
        onClick={() => {
          void window.electronAPI
            .pickFolder?.()
            .then((dir: string | null | undefined) => {
              if (dir) onChange(dir);
            })
            .catch(() => {});
        }}
      />
    )}
  </div>
);

const JobEditForm: React.FC<{
  draft: Draft;
  onChange: (d: Draft) => void;
  onSave: () => void;
  onCancel: () => void;
}> = ({ draft, onChange, onSave, onCancel }) => {
  const set = (patch: Partial<Draft>) => onChange({ ...draft, ...patch });
  const problem = draftProblem(draft);
  const submitKeys = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter' && !problem) onSave();
    if (e.key === 'Escape') onCancel();
  };
  return (
    <div
      style={{
        display: 'flex',
        flexDirection: 'column',
        gap: 4,
        padding: 8,
        backgroundColor: 'var(--wks-bg-surface)',
        borderRadius: 'var(--wks-radius-sm)',
        border: '1px solid var(--wks-border-input)',
      }}
    >
      <input
        value={draft.name}
        onChange={(e) => set({ name: e.target.value })}
        onKeyDown={submitKeys}
        placeholder="Job name"
        style={inputStyle}
        autoFocus
      />

      <div style={{ display: 'flex', gap: 4 }}>
        <select
          value={draft.triggerKind}
          onChange={(e) => set({ triggerKind: e.target.value as Draft['triggerKind'] })}
          style={{ ...selectStyle, flex: 1 }}
        >
          <option value="interval">Every N minutes</option>
          <option value="daily">Daily at a time</option>
          <option value="once">Once, at a time</option>
          <option value="manual">Manual only</option>
        </select>
        {draft.triggerKind === 'interval' && (
          <input
            value={draft.everyMinutes}
            onChange={(e) => set({ everyMinutes: e.target.value })}
            onKeyDown={submitKeys}
            placeholder="minutes"
            inputMode="numeric"
            style={{ ...inputStyle, width: 90 }}
          />
        )}
        {draft.triggerKind === 'daily' && (
          <input
            type="time"
            value={draft.at}
            onChange={(e) => set({ at: e.target.value })}
            style={{ ...inputStyle, width: 110 }}
          />
        )}
        {draft.triggerKind === 'once' && (
          <input
            type="datetime-local"
            value={draft.once}
            min={isoToLocalInput(new Date().toISOString())}
            onChange={(e) => set({ once: e.target.value })}
            style={{ ...inputStyle, width: 190 }}
          />
        )}
      </div>
      {draft.triggerKind === 'daily' && (
        <div style={{ display: 'flex', gap: 4, alignItems: 'center' }}>
          <div style={{ display: 'flex', gap: 2, flex: 1 }}>
            {DAY_LABELS.map((label, d) => {
              const on = draft.days.includes(d);
              return (
                <button
                  key={label}
                  onClick={() =>
                    set({
                      days: on ? draft.days.filter((x) => x !== d) : [...draft.days, d].sort(),
                    })
                  }
                  style={{
                    flex: 1,
                    padding: '3px 0',
                    fontSize: '0.6rem',
                    fontFamily: 'inherit',
                    borderRadius: 'var(--wks-radius-sm)',
                    cursor: 'pointer',
                    border: '1px solid var(--wks-border-input)',
                    background: on ? 'var(--wks-accent-bg)' : 'transparent',
                    color: on ? 'var(--wks-accent)' : 'var(--wks-text-muted)',
                  }}
                >
                  {label}
                </button>
              );
            })}
          </div>
          <span style={{ fontSize: '0.6rem', color: 'var(--wks-text-faint)', flexShrink: 0 }}>
            {draft.days.length === 0 ? 'every day' : ''}
          </span>
        </div>
      )}

      <select
        value={draft.actionKind}
        onChange={(e) => set({ actionKind: e.target.value as Draft['actionKind'] })}
        style={selectStyle}
      >
        <option value="spawn">Spawn an agent with a prompt</option>
        <option value="shell">Run a shell command</option>
        <option value="call">Call a bus capability</option>
      </select>
      {draft.actionKind === 'spawn' && (
        <>
          <div style={{ display: 'flex', gap: 4 }}>
            <CwdInput
              value={draft.cwd}
              placeholder="Working directory (absolute path)"
              onChange={(cwd) => set({ cwd })}
            />
            <select
              value={draft.provider}
              onChange={(e) => set({ provider: e.target.value })}
              style={{ ...selectStyle, width: 110 }}
            >
              <option value="claude">Claude</option>
              <option value="codex">Codex</option>
              <option value="opencode">OpenCode</option>
              <option value="pi">Pi</option>
            </select>
          </div>
          <textarea
            value={draft.prompt}
            onChange={(e) => set({ prompt: e.target.value })}
            placeholder="Prompt the agent starts with"
            rows={3}
            style={{ ...inputStyle, resize: 'vertical', height: 'auto', lineHeight: 1.4 }}
          />
          <input
            value={draft.model}
            onChange={(e) => set({ model: e.target.value })}
            onKeyDown={submitKeys}
            placeholder="Model (optional — blank = the provider's default)"
            style={inputStyle}
          />
        </>
      )}
      {draft.actionKind === 'shell' && (
        <>
          <input
            value={draft.command}
            onChange={(e) => set({ command: e.target.value })}
            onKeyDown={submitKeys}
            placeholder="Command (runs through the shell on the hub's machine)"
            style={{ ...inputStyle, fontFamily: 'var(--wks-font-mono)' }}
          />
          <CwdInput
            value={draft.shellCwd}
            placeholder="Working directory (optional)"
            onChange={(shellCwd) => set({ shellCwd })}
          />
        </>
      )}
      {draft.actionKind === 'call' && (
        <>
          <input
            value={draft.method}
            onChange={(e) => set({ method: e.target.value })}
            onKeyDown={submitKeys}
            placeholder="Method (e.g. notifications.post)"
            style={{ ...inputStyle, fontFamily: 'var(--wks-font-mono)' }}
          />
          <input
            value={draft.params}
            onChange={(e) => set({ params: e.target.value })}
            onKeyDown={submitKeys}
            placeholder='Params JSON (e.g. {"title":"ping"})'
            style={{ ...inputStyle, fontFamily: 'var(--wks-font-mono)' }}
          />
        </>
      )}

      <div style={{ display: 'flex', gap: 8, alignItems: 'center', justifyContent: 'flex-end' }}>
        {problem && (
          <span style={{ fontSize: '0.6rem', color: 'var(--wks-text-faint)', flex: 1 }}>
            {problem}
          </span>
        )}
        <SmallButton label="Cancel" onClick={onCancel} />
        <SmallButton
          label={draft.id ? 'Save' : 'Create'}
          onClick={() => {
            if (!problem) onSave();
          }}
          primary
          disabled={!!problem}
        />
      </div>
    </div>
  );
};

export default JobsSection;
