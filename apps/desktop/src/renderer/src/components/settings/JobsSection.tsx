import React, { useCallback, useEffect, useRef, useState } from 'react';
import { Check, ChevronDown, ChevronRight, FolderOpen, Play, X } from 'lucide-react';
import { Section, SmallButton, inputStyle } from './primitives';
import type {
  HubJob,
  HubJobContextStep,
  HubJobRun,
  HubJobView,
} from '../../../../main/shared/ipcTypes';

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
        t.days && t.days.length > 0 ? ' · ' + t.days.map((d) => DAY_LABELS[d] ?? d).join(' ') : '';
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
    case 'spawn': {
      const steps = a.spawn?.context?.length ?? 0;
      const pre = steps > 0 ? `${steps} step${steps > 1 ? 's' : ''} → ` : '';
      return `${pre}agent in ${a.spawn?.cwd ?? '?'}`;
    }
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
    j.lastRun.status === 'error' ? 'failed' : j.lastRun.status === 'skipped' ? 'skipped' : 'ok';
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
      <div
        style={{ fontSize: '0.66rem', color: 'var(--wks-text-faint)', padding: '4px 0 4px 24px' }}
      >
        Loading runs…
      </div>
    );
  }
  if (runs.length === 0) {
    return (
      <div
        style={{ fontSize: '0.66rem', color: 'var(--wks-text-faint)', padding: '4px 0 4px 24px' }}
      >
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
/** The hub's cap (maxContextSteps in internal/jobs/jobs.go) — mirrored so the
 *  editor stops offering a step the save would be refused for. */
const MAX_CONTEXT_STEPS = 4;

/** One context step in editor form (every field a string/bool the inputs can
 *  hold; toJob narrows it back to the wire shape). */
interface DraftStep {
  kind: 'shell' | 'call';
  command: string;
  cwd: string;
  method: string;
  params: string;
  skipIfEmpty: boolean;
  skipUnlessMatch: string;
  ignoreExitCode: boolean;
}

const EMPTY_STEP: DraftStep = {
  kind: 'shell',
  command: '',
  cwd: '',
  method: '',
  params: '',
  // Guarding is what steps are FOR, so the empty-check starts on. The exit-code
  // forgiveness does not: a job that quietly swallows a broken command is the
  // failure mode people can't see, and the template below turns it on where
  // it's actually wanted.
  skipIfEmpty: true,
  skipUnlessMatch: '',
  ignoreExitCode: false,
};

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
  steps: DraftStep[];
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
  steps: [],
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
    label: 'Script, then an agent',
    hint: 'A command runs first — the agent is only woken if it found something',
    draft: {
      name: 'Failing tests',
      triggerKind: 'daily',
      at: '07:00',
      actionKind: 'spawn',
      steps: [
        {
          ...EMPTY_STEP,
          command: 'go test ./... 2>&1 | tail -60',
          // A failing suite exits nonzero — that's the interesting case here,
          // not an error, so the guard reads the OUTPUT instead of the code.
          ignoreExitCode: true,
          skipUnlessMatch: 'FAIL',
        },
      ],
      prompt:
        "Last night's test run:\n\n{{output}}\n\nTriage these failures and propose a fix for each.",
    },
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
    steps: (j.action.spawn?.context ?? []).map(toDraftStep),
    provider: j.action.spawn?.provider ?? 'claude',
    model: j.action.spawn?.model ?? '',
    method: j.action.call?.method ?? '',
    params: j.action.call?.params ? JSON.stringify(j.action.call.params) : '',
    command: j.action.shell?.command ?? '',
    shellCwd: j.action.shell?.cwd ?? '',
  };
}

function toDraftStep(st: HubJobContextStep): DraftStep {
  return {
    kind: st.kind,
    command: st.shell?.command ?? '',
    cwd: st.shell?.cwd ?? '',
    method: st.call?.method ?? '',
    params: st.call?.params ? JSON.stringify(st.call.params) : '',
    skipIfEmpty: st.skipIfEmpty ?? false,
    skipUnlessMatch: st.skipUnlessMatch ?? '',
    ignoreExitCode: st.ignoreExitCode ?? false,
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
    for (let i = 0; i < d.steps.length; i++) {
      const st = d.steps[i];
      const n = i + 1;
      if (st.kind === 'shell' && !st.command.trim()) return `Step ${n} needs a command.`;
      if (st.kind === 'call') {
        if (!st.method.trim()) return `Step ${n} needs a capability method.`;
        if (st.params.trim()) {
          try {
            JSON.parse(st.params);
          } catch {
            return `Step ${n} params must be valid JSON.`;
          }
        }
      }
      if (st.skipUnlessMatch.trim()) {
        try {
          new RegExp(st.skipUnlessMatch);
        } catch {
          return `Step ${n} has an invalid "only if it matches" pattern.`;
        }
      }
    }
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
      context: d.steps.length ? d.steps.map(toWireStep) : undefined,
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

function toWireStep(st: DraftStep): HubJobContextStep {
  const out: HubJobContextStep = { kind: st.kind };
  if (st.kind === 'shell') {
    out.shell = { command: st.command.trim(), cwd: st.cwd.trim() || undefined };
  } else {
    out.call = {
      method: st.method.trim(),
      params: st.params.trim() ? JSON.parse(st.params) : undefined,
    };
  }
  if (st.skipIfEmpty) out.skipIfEmpty = true;
  if (st.skipUnlessMatch.trim()) out.skipUnlessMatch = st.skipUnlessMatch.trim();
  if (st.ignoreExitCode) out.ignoreExitCode = true;
  return out;
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

  // Approve = clear the proposal stamp and arm it. The stamp is what the hub
  // checks (a stamped row never schedules and jobs.run refuses it), so this one
  // write is the whole difference between an agent's suggestion and a job.
  const approve = async (j: HubJobView) => {
    const { nextRunAt, lastRun, running, ...job } = j;
    void nextRunAt;
    void lastRun;
    void running;
    setError(null);
    try {
      await window.electronAPI.jobsUpsert({ ...job, proposedBy: undefined, enabled: true });
      load();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
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
        Recurring or one-off tasks the hub runs for you — spawn an agent with a prompt, run a shell
        command, or call a bus capability. Jobs keep running while this window is closed; click a
        row for its run history.
      </div>

      <div style={{ display: 'flex', flexDirection: 'column', gap: 4, marginTop: 8 }}>
        {[...jobs]
          .sort((a, b) => Number(!!b.proposedBy) - Number(!!a.proposedBy))
          .map((j) =>
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
                    disabled={!!j.proposedBy}
                    onClick={(e) => e.stopPropagation()}
                    onChange={() => void toggle(j)}
                    title={
                      j.proposedBy
                        ? 'Proposed by an agent — approve it to arm it'
                        : j.enabled
                          ? 'Disable'
                          : 'Enable'
                    }
                    style={{
                      cursor: j.proposedBy ? 'not-allowed' : 'pointer',
                      flexShrink: 0,
                      accentColor: 'var(--wks-accent)',
                    }}
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
                      {j.proposedBy && (
                        <span
                          title={`${j.proposedBy} proposed this job. It will not run until you approve it — read the trigger and action first.`}
                          style={{
                            fontSize: '0.58rem',
                            fontWeight: 500,
                            padding: '1px 6px',
                            borderRadius: 'var(--wks-radius-pill)',
                            border: '1px solid var(--wks-warning)',
                            color: 'var(--wks-warning)',
                            whiteSpace: 'nowrap',
                            flexShrink: 0,
                          }}
                        >
                          proposed by {j.proposedBy}
                        </span>
                      )}
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
                    {j.proposedBy ? (
                      <SmallButton
                        label={
                          <span
                            style={{ display: 'inline-flex', alignItems: 'center', gap: 4 }}
                            title="Approve this proposal and enable it"
                          >
                            <Check size={11} strokeWidth={2} /> Approve
                          </span>
                        }
                        onClick={() => void approve(j)}
                        primary
                      />
                    ) : (
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
                    )}
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

/** Pre-spawn steps: run something, feed it to the prompt, and let it cancel
 *  the agent entirely. Rendered under the spawn prompt because that's what it
 *  fills in — the placeholder hint sits right beside the textarea it names. */
const ContextSteps: React.FC<{
  steps: DraftStep[];
  onChange: (steps: DraftStep[]) => void;
}> = ({ steps, onChange }) => {
  const patch = (i: number, p: Partial<DraftStep>) =>
    onChange(steps.map((st, n) => (n === i ? { ...st, ...p } : st)));
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
      <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
        <span style={{ fontSize: '0.6rem', color: 'var(--wks-text-faint)', flex: 1 }}>
          {steps.length === 0
            ? 'Optional: run something first and hand the result to the agent'
            : 'Runs on this machine before the agent exists — use {{output}} in the prompt'}
        </span>
        {steps.length < MAX_CONTEXT_STEPS && (
          <SmallButton
            label={steps.length === 0 ? '+ Run something first' : '+ Step'}
            onClick={() => onChange([...steps, { ...EMPTY_STEP }])}
          />
        )}
      </div>
      {steps.map((st, i) => (
        <div
          key={i}
          style={{
            display: 'flex',
            flexDirection: 'column',
            gap: 4,
            padding: 6,
            borderRadius: 'var(--wks-radius-sm)',
            border: '1px solid var(--wks-border-subtle)',
          }}
        >
          <div style={{ display: 'flex', gap: 4, alignItems: 'center' }}>
            <span
              style={{
                fontSize: '0.58rem',
                color: 'var(--wks-text-faint)',
                fontFamily: 'var(--wks-font-mono)',
                flexShrink: 0,
              }}
              title={`Substituted at {{output.${i + 1}}}`}
            >
              {`{{output.${i + 1}}}`}
            </span>
            <select
              value={st.kind}
              onChange={(e) => patch(i, { kind: e.target.value as DraftStep['kind'] })}
              style={{ ...selectStyle, width: 90, flexShrink: 0 }}
            >
              <option value="shell">Shell</option>
              <option value="call">Call</option>
            </select>
            {st.kind === 'shell' ? (
              <input
                value={st.command}
                onChange={(e) => patch(i, { command: e.target.value })}
                placeholder="Command — its output becomes the context"
                style={{ ...inputStyle, flex: 1, fontFamily: 'var(--wks-font-mono)' }}
              />
            ) : (
              <input
                value={st.method}
                onChange={(e) => patch(i, { method: e.target.value })}
                placeholder="Method (e.g. sessions.list)"
                style={{ ...inputStyle, flex: 1, fontFamily: 'var(--wks-font-mono)' }}
              />
            )}
            <SmallButton
              label={
                <span style={{ display: 'inline-flex', alignItems: 'center' }} title="Remove step">
                  <X size={12} strokeWidth={2} />
                </span>
              }
              onClick={() => onChange(steps.filter((_, n) => n !== i))}
            />
          </div>
          {st.kind === 'shell' ? (
            <CwdInput
              value={st.cwd}
              placeholder="Working directory (optional)"
              onChange={(cwd) => patch(i, { cwd })}
            />
          ) : (
            <input
              value={st.params}
              onChange={(e) => patch(i, { params: e.target.value })}
              placeholder="Params JSON (optional)"
              style={{ ...inputStyle, fontFamily: 'var(--wks-font-mono)' }}
            />
          )}
          <div style={{ display: 'flex', gap: 10, alignItems: 'center', flexWrap: 'wrap' }}>
            <CheckLabel
              checked={st.skipIfEmpty}
              onChange={(skipIfEmpty) => patch(i, { skipIfEmpty })}
              label="Skip if no output"
              title="Nothing came back, so nothing is worth an agent — the run records as skipped and no model is called."
            />
            {st.kind === 'shell' && (
              <CheckLabel
                checked={st.ignoreExitCode}
                onChange={(ignoreExitCode) => patch(i, { ignoreExitCode })}
                label="Nonzero exit is OK"
                title="For guards that signal 'nothing found' with an exit code (grep, test runners). Timeouts still fail the job either way."
              />
            )}
            <input
              value={st.skipUnlessMatch}
              onChange={(e) => patch(i, { skipUnlessMatch: e.target.value })}
              placeholder="Only if output matches… (regex, optional)"
              style={{
                ...inputStyle,
                flex: 1,
                minWidth: 140,
                fontFamily: 'var(--wks-font-mono)',
              }}
            />
          </div>
        </div>
      ))}
    </div>
  );
};

const CheckLabel: React.FC<{
  checked: boolean;
  onChange: (v: boolean) => void;
  label: string;
  title: string;
}> = ({ checked, onChange, label, title }) => (
  <label
    title={title}
    style={{
      display: 'inline-flex',
      alignItems: 'center',
      gap: 4,
      fontSize: '0.6rem',
      color: 'var(--wks-text-muted)',
      cursor: 'pointer',
      flexShrink: 0,
    }}
  >
    <input
      type="checkbox"
      checked={checked}
      onChange={(e) => onChange(e.target.checked)}
      style={{ cursor: 'pointer', accentColor: 'var(--wks-accent)' }}
    />
    {label}
  </label>
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
          <ContextSteps steps={draft.steps} onChange={(steps) => set({ steps })} />
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
