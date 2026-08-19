import React, { useCallback, useEffect, useState } from 'react';
import { Play, X } from 'lucide-react';
import { Section, SmallButton, inputStyle } from './primitives';
import type { HubJob, HubJobView } from '../../../../main/shared/ipcTypes';

/**
 * Jobs — recurring and one-off tasks the hub runs on your behalf: spawn an
 * agent with a prompt, call a bus capability, or run a shell command, on an
 * interval, at a daily time, once, or manually. The hub owns storage,
 * validation and scheduling (services/hub/internal/jobs); this section is a
 * thin editor over the trusted-only jobs.* RPCs. The list polls while the
 * section is open so run-state chips stay roughly live without any new bus
 * topic.
 */

const DAY_LABELS = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'];

function triggerSummary(j: HubJobView): string {
  const t = j.trigger;
  switch (t.kind) {
    case 'interval': {
      const m = t.everyMinutes ?? 0;
      return m % 60 === 0 ? `every ${m / 60}h` : `every ${m}m`;
    }
    case 'daily': {
      const days =
        t.days && t.days.length > 0 ? ' ' + t.days.map((d) => DAY_LABELS[d] ?? d).join(',') : '';
      return `daily ${t.at}${days}`;
    }
    case 'once':
      return t.once ? `once ${new Date(t.once).toLocaleString()}` : 'once';
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

/** Last-run status chip: running > error > skipped > ok. */
const RunChip: React.FC<{ j: HubJobView }> = ({ j }) => {
  let text: string | null = null;
  let color = 'var(--wks-text-faint)';
  if (j.running) {
    text = 'running';
    color = 'var(--wks-accent)';
  } else if (j.lastRun) {
    const when = ago(j.lastRun.finishedAt ?? j.lastRun.startedAt);
    if (j.lastRun.status === 'error') {
      text = `failed ${when}`;
      color = 'var(--wks-error)';
    } else if (j.lastRun.status === 'skipped') {
      text = `skipped ${when}`;
    } else {
      text = `ok ${when}`;
      color = 'var(--wks-success)';
    }
  }
  if (!text) return null;
  return (
    <span title={j.lastRun?.detail} style={{ fontSize: '0.62rem', color, flexShrink: 0 }}>
      {text}
    </span>
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

/** Draft → wire job. Throws with a user-facing message on bad input; the hub
 *  re-validates authoritatively. */
function toJob(d: Draft, enabled: boolean): HubJob | Omit<HubJob, 'id'> {
  const trigger: HubJob['trigger'] = { kind: d.triggerKind };
  if (d.triggerKind === 'interval') trigger.everyMinutes = Math.max(1, Number(d.everyMinutes) || 0);
  if (d.triggerKind === 'daily') {
    trigger.at = d.at;
    if (d.days.length > 0) trigger.days = d.days;
  }
  if (d.triggerKind === 'once') {
    const t = new Date(d.once);
    if (isNaN(t.getTime())) throw new Error('Pick a date/time for the one-off run.');
    trigger.once = t.toISOString();
  }
  const action: HubJob['action'] = { kind: d.actionKind };
  if (d.actionKind === 'spawn') {
    action.spawn = {
      cwd: d.cwd.trim(),
      prompt: d.prompt.trim(),
      provider: d.provider || undefined,
      model: d.model.trim() || undefined,
    };
  } else if (d.actionKind === 'call') {
    let params: unknown;
    if (d.params.trim()) {
      try {
        params = JSON.parse(d.params);
      } catch {
        throw new Error('Params must be valid JSON.');
      }
    }
    action.call = { method: d.method.trim(), params };
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
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(() => {
    window.electronAPI
      .jobsList()
      .then((res) => setJobs(res?.jobs ?? []))
      // A hub without jobs (older build, or a view/triage web token) —
      // show the empty affordance-less state rather than a broken editor.
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
    await window.electronAPI.jobsRun(id).catch(() => {});
    load();
  };

  const remove = async (id: string) => {
    await window.electronAPI.jobsRemove(id).catch(() => {});
    load();
  };

  if (!available) {
    return (
      <Section title="Jobs">
        <div style={{ fontSize: '0.7rem', color: 'var(--wks-text-muted)', lineHeight: 1.5 }}>
          Jobs are managed by the hub and aren't available on this connection.
        </div>
      </Section>
    );
  }

  return (
    <Section title="Jobs">
      <div style={{ fontSize: '0.68rem', color: 'var(--wks-text-muted)', lineHeight: 1.5 }}>
        Recurring or one-off tasks the hub runs for you — spawn an agent with a prompt, call a bus
        capability, or run a shell command. Jobs run even while this window is closed, as long as
        the hub is up.
      </div>
      <div style={{ display: 'flex', flexDirection: 'column', gap: 4, marginTop: 8 }}>
        {jobs.map((j) =>
          editing?.id === j.id ? (
            <JobEditForm
              key={j.id}
              draft={editing}
              onChange={setEditing}
              onSave={() => void save(editing)}
              onCancel={() => setEditing(null)}
            />
          ) : (
            <div
              key={j.id}
              style={{ display: 'flex', alignItems: 'center', gap: 8, padding: '4px 8px' }}
            >
              <input
                type="checkbox"
                checked={j.enabled}
                onChange={() => void toggle(j)}
                title={j.enabled ? 'Disable' : 'Enable'}
                style={{ cursor: 'pointer', flexShrink: 0 }}
              />
              <div style={{ flex: 1, minWidth: 0 }}>
                <div
                  style={{
                    fontSize: '0.7rem',
                    fontWeight: 500,
                    color: 'var(--wks-text-secondary)',
                    display: 'flex',
                    gap: 8,
                    alignItems: 'baseline',
                  }}
                >
                  <span style={{ overflow: 'hidden', textOverflow: 'ellipsis' }}>{j.name}</span>
                  <RunChip j={j} />
                </div>
                <div
                  style={{
                    fontSize: '0.62rem',
                    color: 'var(--wks-text-faint)',
                    fontFamily: 'var(--wks-font-mono)',
                    overflow: 'hidden',
                    textOverflow: 'ellipsis',
                    whiteSpace: 'nowrap',
                  }}
                >
                  {triggerSummary(j)}
                  {j.enabled && j.nextRunAt ? ` (${inFuture(j.nextRunAt)})` : ''} · {actionSummary(j)}
                </div>
              </div>
              <SmallButton
                label={<Play size={11} strokeWidth={2.25} />}
                onClick={() => void runNow(j.id)}
              />
              <SmallButton label="Edit" onClick={() => setEditing(toDraft(j))} />
              <SmallButton
                label={<X size={11} strokeWidth={2.25} />}
                onClick={() => void remove(j.id)}
                danger
              />
            </div>
          ),
        )}

        {error && (
          <div style={{ fontSize: '0.66rem', color: 'var(--wks-error)' }}>{error}</div>
        )}

        {editing && !editing.id ? (
          <JobEditForm
            draft={editing}
            onChange={setEditing}
            onSave={() => void save(editing)}
            onCancel={() => setEditing(null)}
          />
        ) : (
          !editing && (
            <button
              onClick={() => {
                setError(null);
                setEditing({ ...EMPTY_DRAFT });
              }}
              style={{
                padding: '6px 12px',
                fontSize: '0.68rem',
                fontFamily: 'inherit',
                fontWeight: 500,
                backgroundColor: 'transparent',
                color: 'var(--wks-text-muted)',
                border: '1px dashed var(--wks-border-input)',
                borderRadius: '4px',
                cursor: 'pointer',
                lineHeight: 1.4,
                margin: '4px 0 0',
                width: '100%',
              }}
            >
              + Add Job
            </button>
          )
        )}
      </div>
    </Section>
  );
};

const selectStyle: React.CSSProperties = {
  ...inputStyle,
  cursor: 'pointer',
};

const JobEditForm: React.FC<{
  draft: Draft;
  onChange: (d: Draft) => void;
  onSave: () => void;
  onCancel: () => void;
}> = ({ draft, onChange, onSave, onCancel }) => {
  const set = (patch: Partial<Draft>) => onChange({ ...draft, ...patch });
  return (
    <div
      style={{
        display: 'flex',
        flexDirection: 'column',
        gap: 4,
        padding: 8,
        backgroundColor: 'var(--wks-bg-surface)',
        borderRadius: 4,
        border: '1px solid var(--wks-border-input)',
      }}
    >
      <input
        value={draft.name}
        onChange={(e) => set({ name: e.target.value })}
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
            placeholder="minutes"
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
            onChange={(e) => set({ once: e.target.value })}
            style={{ ...inputStyle, width: 190 }}
          />
        )}
      </div>
      {draft.triggerKind === 'daily' && (
        <div style={{ display: 'flex', gap: 2 }}>
          {DAY_LABELS.map((label, d) => {
            const on = draft.days.includes(d);
            return (
              <button
                key={label}
                onClick={() =>
                  set({ days: on ? draft.days.filter((x) => x !== d) : [...draft.days, d].sort() })
                }
                title={draft.days.length === 0 ? 'Every day (pick days to restrict)' : label}
                style={{
                  flex: 1,
                  padding: '3px 0',
                  fontSize: '0.62rem',
                  fontFamily: 'inherit',
                  borderRadius: 3,
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
            <input
              value={draft.cwd}
              onChange={(e) => set({ cwd: e.target.value })}
              placeholder="Working directory (absolute path)"
              style={{ ...inputStyle, flex: 1, fontFamily: 'var(--wks-font-mono)' }}
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
            style={{ ...inputStyle, resize: 'vertical', height: 'auto' }}
          />
          <input
            value={draft.model}
            onChange={(e) => set({ model: e.target.value })}
            placeholder="Model (optional, e.g. haiku)"
            style={inputStyle}
          />
        </>
      )}
      {draft.actionKind === 'shell' && (
        <>
          <input
            value={draft.command}
            onChange={(e) => set({ command: e.target.value })}
            placeholder="Command (runs through the shell on the hub's machine)"
            style={{ ...inputStyle, fontFamily: 'var(--wks-font-mono)' }}
          />
          <input
            value={draft.shellCwd}
            onChange={(e) => set({ shellCwd: e.target.value })}
            placeholder="Working directory (optional)"
            style={{ ...inputStyle, fontFamily: 'var(--wks-font-mono)' }}
          />
        </>
      )}
      {draft.actionKind === 'call' && (
        <>
          <input
            value={draft.method}
            onChange={(e) => set({ method: e.target.value })}
            placeholder="Method (e.g. notifications.post)"
            style={{ ...inputStyle, fontFamily: 'var(--wks-font-mono)' }}
          />
          <input
            value={draft.params}
            onChange={(e) => set({ params: e.target.value })}
            placeholder='Params JSON (e.g. {"title":"ping"})'
            style={{ ...inputStyle, fontFamily: 'var(--wks-font-mono)' }}
          />
        </>
      )}

      <div style={{ display: 'flex', gap: 4, justifyContent: 'flex-end' }}>
        <SmallButton label="Cancel" onClick={onCancel} />
        <SmallButton label="Save" onClick={onSave} primary />
      </div>
    </div>
  );
};

export default JobsSection;
