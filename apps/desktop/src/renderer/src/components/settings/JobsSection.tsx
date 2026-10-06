import React, { useCallback, useEffect, useRef, useState } from 'react';
import { Check, ChevronDown, ChevronRight, Play, X } from 'lucide-react';
import { Section, SmallButton } from './primitives';
import DraftWithAgentButton from './DraftWithAgentButton';
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
 * validation and scheduling (services/hub-rs/src/services/jobs.rs).
 *
 * This is a VIEW, not an editor: agents write jobs (the scheduled-jobs skill
 * and propose_job), and here the user reads them, approves proposals, pauses,
 * runs and removes. Approving is the one write that turns an agent's spec into
 * a job, so the expanded row shows the whole spec — for a proposed change,
 * beside the job it replaces. The list polls while the section is open so
 * run-state chips stay roughly live without any new bus topic; run-now flips
 * the chip optimistically so the click lands instantly.
 */

const DAY_LABELS = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'];

function triggerSummary(j: HubJob): string {
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

function actionSummary(j: HubJob): string {
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

function stepSummary(step: HubJobContextStep): string {
  const run =
    step.kind === 'shell' ? `$ ${step.shell?.command ?? '?'}` : `call ${step.call?.method ?? '?'}`;
  const guards = [
    step.skipIfEmpty && 'skip if empty',
    step.skipUnlessMatch && `skip unless /${step.skipUnlessMatch}/`,
    step.ignoreExitCode && 'exit code ignored',
  ].filter(Boolean);
  return guards.length > 0 ? `${run}  (${guards.join(', ')})` : run;
}

/** Everything a job will do, field by field — what the user reads before
 *  approving, since there is no editor to open it in. */
function specRows(j: HubJob): Array<[string, string]> {
  const rows: Array<[string, string]> = [
    ['Name', j.name],
    ['When', triggerSummary(j)],
  ];
  const a = j.action;
  if (a.kind === 'spawn' && a.spawn) {
    const sp = a.spawn;
    rows.push(['Agent in', sp.cwd]);
    const who = [sp.provider, sp.model, sp.effort, sp.permissionMode].filter(Boolean).join(' · ');
    if (who) rows.push(['Using', who]);
    (sp.context ?? []).forEach((step, i) => rows.push([`Step ${i + 1}`, stepSummary(step)]));
    rows.push(['Prompt', sp.prompt]);
  } else if (a.kind === 'shell') {
    rows.push(['Runs', `$ ${a.shell?.command ?? '?'}`]);
    if (a.shell?.cwd) rows.push(['In', a.shell.cwd]);
  } else if (a.kind === 'call') {
    rows.push(['Calls', a.call?.method ?? '?']);
    if (a.call?.params !== undefined) rows.push(['Params', JSON.stringify(a.call.params, null, 2)]);
  }
  return rows;
}

const SpecDetail: React.FC<{ job: HubJob; title?: string; base?: HubJob }> = ({
  job,
  title,
  base,
}) => {
  const before = base ? new Map(specRows(base)) : null;
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 2, minWidth: 0 }}>
      {title && (
        <div
          style={{
            fontSize: '0.6rem',
            fontWeight: 600,
            textTransform: 'uppercase',
            letterSpacing: '0.04em',
            color: 'var(--wks-text-faint)',
          }}
        >
          {title}
        </div>
      )}
      {specRows(job).map(([label, value]) => {
        const changed = before !== null && before.get(label) !== value;
        return (
          <div key={label} style={{ display: 'flex', gap: 8, minWidth: 0 }}>
            <span
              style={{
                width: 64,
                flexShrink: 0,
                fontSize: '0.66rem',
                color: changed ? 'var(--wks-warning)' : 'var(--wks-text-faint)',
              }}
            >
              {label}
            </span>
            <span
              style={{
                flex: 1,
                minWidth: 0,
                fontSize: '0.66rem',
                color: 'var(--wks-text-secondary)',
                fontFamily: 'var(--wks-font-mono)',
                whiteSpace: 'pre-wrap',
                overflowWrap: 'anywhere',
              }}
            >
              {value}
            </span>
          </div>
        );
      })}
    </div>
  );
};

/** The wire spec of a listed row, without jobs.list's live fields. */
function specOf(j: HubJobView): HubJob {
  const { nextRunAt, lastRun, running, ...job } = j;
  void nextRunAt;
  void lastRun;
  void running;
  return job;
}

const JobsSection: React.FC = () => {
  const [jobs, setJobs] = useState<HubJobView[]>([]);
  const [available, setAvailable] = useState(true);
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
      // show the plain unavailable note rather than a broken list.
      .catch(() => setAvailable(false));
  }, []);

  useEffect(() => {
    load();
    const t = setInterval(load, 10_000);
    return () => clearInterval(t);
  }, [load]);

  const toggle = async (j: HubJobView) => {
    setError(null);
    await window.electronAPI.jobsUpsert({ ...specOf(j), enabled: !j.enabled }).catch(() => {});
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
  // write is the whole difference between an agent's suggestion and a job. A
  // proposal that `replaces` a job is applied to that job in place by the hub,
  // which keeps the job's own on/off state.
  const approve = async (j: HubJobView) => {
    setError(null);
    try {
      await window.electronAPI.jobsUpsert({ ...specOf(j), proposedBy: undefined, enabled: true });
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

  const byId = new Map(jobs.map((j) => [j.id, j]));

  return (
    <Section title="Jobs">
      <div style={{ fontSize: '0.72rem', color: 'var(--wks-text-muted)', lineHeight: 1.5 }}>
        Recurring or one-off tasks the hub runs for you — spawn an agent with a prompt, run a shell
        command, or call a bus capability. They keep running while this window is closed. To add or
        change one, ask any agent; what it proposes waits here until you approve it. Click a row for
        its full spec and run history.
      </div>

      <div style={{ display: 'flex', flexDirection: 'column', gap: 4, marginTop: 8 }}>
        {[...jobs]
          .sort((a, b) => Number(!!b.proposedBy) - Number(!!a.proposedBy))
          .map((j) => {
            const target = j.proposedBy && j.replaces ? byId.get(j.replaces) : undefined;
            return (
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
                    opacity: j.enabled || j.proposedBy ? 1 : 0.55,
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
                          ? 'Pause'
                          : 'Resume'
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
                          title={`${j.proposedBy} proposed this. It does nothing until you approve it — expand the row and read it first.`}
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
                          {j.replaces
                            ? `change to ${target?.name ?? 'a removed job'} · by ${j.proposedBy}`
                            : `proposed by ${j.proposedBy}`}
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
                            title={
                              j.replaces
                                ? 'Apply this change to the job'
                                : 'Approve this proposal and enable it'
                            }
                          >
                            <Check size={11} strokeWidth={2} /> Approve
                          </span>
                        }
                        onClick={() => void approve(j)}
                        disabled={!!j.replaces && !target}
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
                    <SmallButton
                      label={
                        <span title={j.proposedBy ? 'Reject this proposal' : 'Remove this job'}>
                          <X size={11} strokeWidth={2} />
                        </span>
                      }
                      onClick={() => void remove(j.id)}
                      danger
                    />
                  </span>
                </div>
                {expanded === j.id && (
                  <div
                    style={{
                      display: 'flex',
                      flexDirection: 'column',
                      gap: 8,
                      padding: '4px 8px 8px 24px',
                    }}
                  >
                    {target ? (
                      <>
                        <SpecDetail job={target} title="Now" />
                        <SpecDetail job={j} title="Proposed" base={target} />
                      </>
                    ) : (
                      <SpecDetail job={j} />
                    )}
                  </div>
                )}
                {expanded === j.id && !j.proposedBy && (
                  <RunHistory jobId={j.id} refreshKey={historyKey} />
                )}
              </div>
            );
          })}

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

        <div
          style={
            jobs.length === 0
              ? {
                  display: 'flex',
                  flexDirection: 'column',
                  gap: 6,
                  padding: '10px 12px',
                  borderRadius: 'var(--wks-radius-md)',
                  border: '1px dashed var(--wks-border-input)',
                }
              : { display: 'flex', flexDirection: 'column', gap: 6, marginTop: 6 }
          }
        >
          {jobs.length === 0 && (
            <div style={{ fontSize: '0.72rem', color: 'var(--wks-text-secondary)' }}>
              No jobs yet. Ask any agent for one, for example “every weekday at 7, run the tests and
              wake an agent if anything fails.”
            </div>
          )}
          <DraftWithAgentButton briefId="jobs" />
        </div>
      </div>
    </Section>
  );
};

export default JobsSection;
