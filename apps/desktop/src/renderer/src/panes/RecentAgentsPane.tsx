import FleetWorkflowTask from '../components/FleetWorkflowTask';
import type { AgentProvider } from '../types/pane';
import React, { useEffect, useRef, useState } from 'react';
import { RefreshCw } from 'lucide-react';
import type {
  DispatchAttempt,
  DispatchHistoryResponse,
  DispatchTask,
} from '../../../main/shared/dispatchHistory';
import { Surface } from '../components/Surface';
import { SmallButton } from '../components/settings/primitives';
import { FleetReview } from '../components/claude/FleetReview';
import { HtmlCardHostContext } from '../components/claude/HtmlResponseCard';
import { requestInspector, requestSessionWatch } from '../lib/watchBus';
import './RecentAgentsPane.css';

const unknown = 'Not reported';
const number = (n?: number) => (n === undefined ? unknown : n.toLocaleString());
const wall = (ms?: number) => (ms === undefined ? unknown : `${Math.round(ms / 1000)}s`);
export function total(
  attempts: DispatchAttempt[],
  key: 'inputTokens' | 'outputTokens' | 'costUSD' | 'wallMs',
): string {
  const unique = [...new Map(attempts.map((a) => [a.dispatchId, a])).values()];
  const values = unique.map((a) => a.metrics[key]).filter((v): v is number => v !== undefined);
  if (!values.length) return `${unknown} (0/${unique.length})`;
  const n = values.reduce((a, b) => a + b, 0);
  return `${key === 'costUSD' ? `$${n.toFixed(4)}` : key === 'wallMs' ? wall(n) : number(n)} · ${values.length === unique.length ? '' : 'partial '}${values.length}/${unique.length} reported`;
}
export default function RecentAgentsPane(): React.ReactElement {
  const [data, setData] = useState<DispatchHistoryResponse>();
  const [error, setError] = useState('');
  const [scope, setScope] = useState('current');
  const [project, setProject] = useState('all');
  const [status, setStatus] = useState('all');
  const [days, setDays] = useState('all');
  const [busy, setBusy] = useState(false);
  const generation = useRef(0);
  async function refresh() {
    const g = ++generation.current;
    setBusy(true);
    try {
      const next = (await window.electronAPI?.dispatchHistoryRead?.()) ?? {
        available: false as const,
        reason: 'Recent agents history is unavailable on this backend or preload version.',
      };
      if (g === generation.current) {
        setData(next);
        setError('');
      }
    } catch {
      if (g === generation.current) setError('Could not read local dispatch history.');
    } finally {
      if (g === generation.current) setBusy(false);
    }
  }
  useEffect(() => {
    void refresh();
    const timer = setInterval(() => void refresh(), 3000);
    return () => {
      clearInterval(timer);
      generation.current++;
    };
  }, []);
  const owner = data?.available ? data.currentOwnerSessionId : undefined;
  const currentOwner = useRef(owner);
  currentOwner.current = owner;
  const tasks = data?.available ? data.tasks : [];
  const filtered = tasks.filter(
    (t) =>
      (scope === 'all' || t.ownerSessionId === owner) &&
      (project === 'all' || t.projectCwd === project) &&
      (status === 'all' ||
        t.attempts.some((a) => (status === 'stale' ? a.stale : a.lifecycle === status))) &&
      (days === 'all' || Date.parse(t.createdAt) >= Date.now() - Number(days) * 86400000),
  );
  return (
    <section className="recent-agents" aria-label="Recent agents">
      <header>
        <div>
          <h2>Recent agents</h2>
          <p>Direct agents and inspect actual task attempts.</p>
        </div>
        <SmallButton
          onClick={() => void refresh()}
          disabled={busy}
          label={
            <>
              <RefreshCw size={12} /> Refresh
            </>
          }
        />
      </header>
      {!data && !error && <p role="status">Loading local dispatch history…</p>}
      {error && <p role="alert">{error}</p>}
      {data && !data.available && (
        <Surface elevation="flat" pad="lg">
          <h3>History unavailable</h3>
          <p>{data.reason}</p>
        </Surface>
      )}
      {data?.available && (
        <>
          <div className="recent-filters">
            <label>
              Manager
              <select aria-label="Manager" value={scope} onChange={(e) => setScope(e.target.value)}>
                <option value="current">Current manager</option>
                <option value="all">All locally recorded managers</option>
              </select>
            </label>
            <label>
              Project
              <select
                aria-label="Project"
                value={project}
                onChange={(e) => setProject(e.target.value)}
              >
                <option value="all">All recorded projects</option>
                {[...new Set(tasks.map((t) => t.projectCwd))].sort().map((p) => (
                  <option key={p}>{p}</option>
                ))}
              </select>
            </label>
            <label>
              Status
              <select
                aria-label="Status"
                value={status}
                onChange={(e) => setStatus(e.target.value)}
              >
                {['all', 'starting', 'running', 'idle', 'needs-decision', 'ended', 'stale'].map(
                  (s) => (
                    <option key={s} value={s}>
                      {s === 'all' ? 'All statuses' : s}
                    </option>
                  ),
                )}
              </select>
            </label>
            <label>
              Time
              <select aria-label="Time" value={days} onChange={(e) => setDays(e.target.value)}>
                <option value="all">All retained time</option>
                <option value="1">Last day</option>
                <option value="7">Last 7 days</option>
                <option value="30">Last 30 days</option>
              </select>
            </label>
          </div>
          <p className="recent-note">
            Local bounded history · {filtered.length} tasks · only accepted dispatches. Idle/ended
            and a valid result contract do not mean passing work. Input includes cache tokens.
            Reported model is the latest observation, not a historic token split.
          </p>
          {scope === 'current' && !owner ? (
            <p>
              No live local manager. Select all locally recorded managers to inspect retained
              history.
            </p>
          ) : !filtered.length ? (
            <p>No recorded dispatches match these filters.</p>
          ) : (
            filtered.map((t) => (
              <Task
                key={t.taskId}
                task={t}
                owner={owner}
                isCurrent={() => currentOwner.current === t.ownerSessionId}
              />
            ))
          )}
        </>
      )}
    </section>
  );
}
function Task({
  task,
  owner,
  isCurrent,
}: {
  task: DispatchTask;
  owner?: string;
  isCurrent: () => boolean;
}) {
  return (
    <Surface elevation="raised" pad="md" className="recent-task">
      <details>
        <summary>
          <strong>{task.title}</strong>
          <span>
            {task.attempts.length} attempts · {new Date(task.createdAt).toLocaleString()}
          </span>
          <span>
            {task.projectCwd} · {task.ownerLabel}
          </span>
          <span>
            Input {total(task.attempts, 'inputTokens')} · Output{' '}
            {total(task.attempts, 'outputTokens')} · Estimated API cost{' '}
            {total(task.attempts, 'costUSD')}
          </span>
        </summary>
        <p className="recent-note">
          Task {task.taskId} · Ordered by accepted dispatch. Only declared stages appear.
        </p>
        <SmallButton
          label="Inspect task"
          onClick={() => requestInspector({ taskId: task.taskId, agentName: task.title })}
        />
        <FleetWorkflowTask task={task} />
        <ol>
          {task.attempts.map((a) => (
            <Attempt
              key={a.dispatchId}
              attempt={a}
              owner={task.ownerSessionId === owner ? owner : undefined}
              isCurrent={isCurrent}
            />
          ))}
        </ol>
      </details>
    </Surface>
  );
}
function Attempt({
  attempt: a,
  owner,
  isCurrent,
}: {
  attempt: DispatchAttempt;
  owner?: string;
  isCurrent: () => boolean;
}) {
  const [error, setError] = useState('');
  async function open() {
    try {
      const s = await window.electronAPI.getClaudeSession(a.sessionId);
      if (!s || s.sessionId !== a.sessionId || s.status === 'ended' || s.hub) {
        setError('This agent is no longer available locally.');
        return;
      }
      requestSessionWatch({
        sessionId: s.sessionId,
        cwd: s.cwd,
        title: a.stage ?? 'Agent',
        provider: a.provider as AgentProvider | undefined,
      });
    } catch {
      setError('This agent is unavailable.');
    }
  }
  return (
    <li className="recent-attempt">
      <div className="recent-step-heading">
        <strong>{a.stage ?? 'Unclassified'}</strong>
        <span>
          {a.kind} · {a.lifecycle}
          {a.stale ? ' · stale / last observed' : ''}
        </span>
      </div>
      <div className="recent-metrics">
        <span>Wall time {wall(a.metrics.wallMs)}</span>
        <span>Input {number(a.metrics.inputTokens)}</span>
        <span>Output {number(a.metrics.outputTokens)}</span>
        <span>
          Estimated API cost{' '}
          {a.metrics.costUSD === undefined ? unknown : `$${a.metrics.costUSD.toFixed(4)}`}
        </span>
      </div>
      <p>
        Provider requested {a.requestedProvider ?? 'default'}; resolved {a.provider ?? unknown}.
        Model requested {a.requestedModel ?? 'default'}; reported {a.reportedModel ?? unknown}.{' '}
        {a.role && `Role: ${a.role}.`} Result contract: {a.resultContract}.
      </p>
      <p>
        Cache{' '}
        {a.metrics.cache
          ? `fresh ${number(a.metrics.cache.fresh)} / write ${number(a.metrics.cache.write)} / read ${number(a.metrics.cache.read)}`
          : a.metrics.cachedInputTokens !== undefined
            ? `read ${number(a.metrics.cachedInputTokens)} (other tiers not reported)`
            : unknown}
        . Context snapshot{' '}
        {a.metrics.context
          ? `${number(a.metrics.context.used)} / ${number(a.metrics.context.limit)} at ${a.metrics.context.observedAt}${a.stale ? ' (stale)' : ''}`
          : unknown}
        .
      </p>
      <p>
        {a.executionCwd} ·{' '}
        {a.worktree?.allocated
          ? `Worktree ${a.worktree.branch ?? ''}`
          : a.worktree?.fallback
            ? `Worktree fallback: ${a.worktree.error ?? 'allocation unavailable'}`
            : 'In place'}
      </p>
      <p className="recent-note">
        Dispatch {a.dispatchId} ·{' '}
        {a.afterDispatchId ? `After ${a.afterDispatchId}` : 'No declared predecessor'}
        {a.retryOfDispatchId && ` · Retry of ${a.retryOfDispatchId}`} · Observed {a.observedAt}
      </p>
      <div className="recent-actions">
        <span
          title={!a.live || a.stale ? 'No exact live session available' : 'Open this live agent'}
        >
          <SmallButton
            onClick={() => void open()}
            disabled={!a.live || a.stale}
            label="Open agent"
          />
        </span>
        <HtmlCardHostContext.Provider value={owner ? { sessionId: owner, isCurrent } : null}>
          <FleetReview
            requireLiveOwner
            evidenceId={owner ? a.reviewEvidenceId : undefined}
            workerSessionId={a.sessionId}
          />
        </HtmlCardHostContext.Provider>
      </div>
      {error && <p role="alert">{error}</p>}
    </li>
  );
}
