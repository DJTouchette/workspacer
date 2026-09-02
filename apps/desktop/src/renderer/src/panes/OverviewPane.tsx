import FleetManagerHero from '../components/FleetManagerHero';
import React, { useCallback, useEffect, useRef, useState } from 'react';
import { useConfig } from '../hooks/useConfig';
import { useAttention } from '../contexts/AttentionContext';
import { usePlugins } from '../hooks/usePlugins';
import { Home, Star, Plus, RefreshCw } from '../components/icons';
import { ProjectMark } from '../components/ProjectMark';
import { favouriteProjects, recentProjects, setFavourite } from '../lib/projectRegistry';
import { claudeAccountOf } from '../lib/claudeAccount';
import { usageWindows, fmtWindowLength } from '../lib/sessionStats';
import { useUsageReport } from '../hooks/useUsageReport';
import { reportAccountKeys, reportWindowsFor } from '../../../main/shared/usageReport';
import { useSessionAnalytics } from '../hooks/useSessionAnalytics';
import { useRecordedUsageMap } from '../contexts/RecordedUsageContext';
import { UsageDetailDialog } from '../components/claude/UsageDetailDialog';
import type { ProjectIdentity } from '../hooks/useConfig';
import { AgentLogo } from '../components/agentLogos';
import type { AgentProvider } from '../types/pane';
import type { UpdateStatus } from '../types/electron';

/** Latest supervisor state per plugin id, from `sidecar.*` bus events.
 *  (Mirrors PluginsManagerPane so the Overview grid shows the same status.) */
function usePluginStates(): Record<string, string> {
  const [states, setStates] = useState<Record<string, string>>({});
  useEffect(() => {
    const off = window.electronAPI.onHubEvent?.((ev) => {
      if (!ev.type?.startsWith('sidecar.')) return;
      const d = ev.data as { name?: string; state?: string } | undefined;
      if (d?.name && d?.state)
        setStates((prev) => ({ ...prev, [d.name as string]: d.state as string }));
    });
    return () => off?.();
  }, []);
  return states;
}

/** Live in-app update status ('unsupported' in dev/web hides the banner). */
function useUpdateStatus(): UpdateStatus | null {
  const [status, setStatus] = useState<UpdateStatus | null>(null);
  useEffect(() => {
    let cancelled = false;
    window.electronAPI
      .updatesGetStatus?.()
      .then((st) => {
        if (!cancelled && st) setStatus(st);
      })
      .catch(() => {});
    const off = window.electronAPI.onUpdateStatus?.((st) => setStatus(st));
    return () => {
      cancelled = true;
      off?.();
    };
  }, []);
  return status;
}

function pluginStateColor(s: string | undefined): string {
  switch (s) {
    case 'healthy':
    case 'running':
      return 'var(--wks-success)';
    case 'unhealthy':
      return 'var(--wks-warning)';
    case 'crashed':
      return 'var(--wks-error)';
    default:
      return 'var(--wks-text-faint)';
  }
}

interface Snap {
  sessionId: string;
  provider?: string;
  ambientState?: string;
  cwd?: string;
  transcriptPath?: string;
  usage?: { costUSD?: number; contextTokens?: number } | null;
  statusLine?: {
    costUSD?: number;
    fiveHourPct?: number;
    fiveHourResetsAt?: number;
    fiveHourWindowMins?: number;
    sevenDayPct?: number;
    sevenDayResetsAt?: number;
    sevenDayWindowMins?: number;
    monthlyPct?: number;
    monthlyResetsAt?: number;
    monthlyWindowMins?: number;
    receivedAt?: string;
  };
}

function basename(p: string): string {
  return (
    p
      .replace(/[/\\]+$/, '')
      .split(/[/\\]/)
      .pop() || p
  );
}
function fmtUSD(n: number): string {
  return n >= 0.01 ? `$${n.toFixed(2)}` : n > 0 ? '<$0.01' : '$0.00';
}
function limitColor(pct: number): string {
  if (pct >= 80) return 'var(--wks-error)';
  if (pct >= 50) return 'var(--wks-warning)';
  return 'var(--wks-success)';
}
function fmtReset(epochSecs: number | undefined): string {
  if (!epochSecs) return '';
  const mins = Math.round((epochSecs * 1000 - Date.now()) / 60000);
  if (mins <= 0) return 'resets soon';
  if (mins < 60) return `resets in ${mins}m`;
  const h = Math.round(mins / 60);
  if (h < 24) return `resets in ${h}h`;
  return `resets in ${Math.round(h / 24)}d`;
}

/** The account-window figures the card draws, cached INDEPENDENTLY of one
 *  another. A status line is not one reading: the daemon rebuilds a stream
 *  session's line from wire data that carries a reset time but no percentage,
 *  so a whole-line "newest wins" lets a newer, emptier line erase a good
 *  percentage — the card stays, the bar empties. A field the newer line does
 *  not mention says nothing about that field, so each is remembered by
 *  whichever line last actually carried it. */
const RATE_LIMIT_FIELDS = [
  'fiveHourPct',
  'fiveHourResetsAt',
  'fiveHourWindowMins',
  'sevenDayPct',
  'sevenDayResetsAt',
  'sevenDayWindowMins',
  'monthlyPct',
  'monthlyResetsAt',
  'monthlyWindowMins',
] as const;
type RateLimitField = (typeof RATE_LIMIT_FIELDS)[number];

/** Freshest rate-limit statusLine seen this app run, per provider+account. The
 *  windows are account-global but ride on per-session statusLines, and the
 *  store evicts a session ~30s after it ends — so a refetch with no live
 *  session for that provider would blank the card even though the account
 *  data is still valid. Module-level so it also survives pane remounts.
 *
 *  `sl`/`ts` are the freshest whole line (everything the detail dialog reads
 *  that is not a window); `fields` is the per-field memory described above. */
const lastRateLimit: Record<
  string,
  {
    sl: NonNullable<Snap['statusLine']>;
    ts: number;
    fields: Partial<Record<RateLimitField, { value: number; ts: number }>>;
  }
> = {};

/** Providers whose sessions report account rate-limit windows, in display
 *  order. Each gets its own card — the windows are per-account, so a Claude
 *  reading and a Codex reading are different accounts and must never be
 *  collapsed into one "freshest wins" card. */
const RATE_LIMIT_PROVIDERS: Array<{ id: string; title: string }> = [
  { id: 'claude', title: 'Claude usage' },
  { id: 'codex', title: 'Codex usage' },
];

/**
 * The 5h/7d rate-limit windows are account-global (identical across every
 * session of one ACCOUNT — for Claude that's one login, i.e. one config
 * root, not one provider: a second-account profile's sessions report their
 * own windows and must get their own card). Pick the freshest statusLine
 * that carries them — newest `receivedAt` wins — and fall back to the last
 * reading seen when the current snapshots carry none (the reset countdowns
 * stay honest: they render from absolute epochs). Renders nothing until the
 * provider has ever reported a window.
 *
 * "Freshest" is decided PER FIELD, not per line (see [`RATE_LIMIT_FIELDS`]):
 * a newer line that omits a percentage is silent about it, not a report of
 * nothing, and a live line older than what is remembered loses to the memory.
 */
/** Exported for the surface test that pins which surfaces open the usage
 *  dialog — the Overview is the one a reader calls "the dashboard", and it
 *  spent a release drawing the windows without a way into their detail. */
export const RateLimitCard: React.FC<{
  snaps: Snap[];
  provider: string;
  title: string;
  /** Claude account group (lib/claudeAccount) this card covers; '' = default.
   *  Undefined = no account filtering (non-Claude providers). */
  account?: string;
}> = ({ snaps, provider, title, account }) => {
  const [detailOpen, setDetailOpen] = useState(false);
  const cacheKey = account === undefined ? provider : `${provider}:${account}`;
  // The session-free source. Only the windows nothing live has spoken for are
  // taken from it (below) — a status line is first-hand and the report is the
  // daemon's summary, so a live reading is never overwritten by it.
  const report = useUsageReport();
  let best: NonNullable<Snap['statusLine']> | null = null;
  let bestTs = -1;
  for (const s of snaps) {
    // Old snapshots may omit provider — those are Claude hook sessions.
    if ((s.provider ?? 'claude') !== provider) continue;
    if (account !== undefined && claudeAccountOf(s.transcriptPath) !== account) continue;
    const sl = s.statusLine;
    if (
      !sl ||
      (sl.fiveHourPct === undefined &&
        sl.sevenDayPct === undefined &&
        sl.monthlyPct === undefined &&
        sl.fiveHourResetsAt === undefined &&
        sl.sevenDayResetsAt === undefined &&
        sl.monthlyResetsAt === undefined)
    )
      continue;
    const ts = sl.receivedAt ? Date.parse(sl.receivedAt) : 0;
    if (ts >= bestTs) {
      bestTs = ts;
      best = sl;
    }
  }
  const cached = lastRateLimit[cacheKey];
  // Per field, the most recent line that ACTUALLY CARRIED it wins. A rebuilt
  // status line arrives newer than the reading it replaces while carrying only
  // a reset time, and taking it wholesale is what empties the meter.
  const fields = { ...(cached?.fields ?? {}) };
  if (best) {
    for (const f of RATE_LIMIT_FIELDS) {
      const v = best[f];
      if (v === undefined) continue;
      const prev = fields[f];
      if (!prev || bestTs >= prev.ts) fields[f] = { value: v, ts: bestTs };
    }
  }
  // The whole line only advances when the live one is at least as fresh as
  // what is cached. The old code updated the cache on that condition but had
  // no branch for "best exists and is OLDER", so an undated line (ts 0) was
  // rendered in preference to a fresher remembered one.
  const fresher = best !== null && bestTs >= (cached?.ts ?? -1);
  const whole = fresher ? best : (cached?.sl ?? best);
  const wholeTs = fresher ? bestTs : (cached?.ts ?? bestTs);
  // Only the windows the report says are RUNNING — a percentage against a
  // reset that has passed is real history and a false present, so
  // reportWindowsFor drops it rather than drawing it (the same currency test
  // the hub's limits.ReadWindow applies to this document).
  const fromReport = reportWindowsFor(report, provider, account, Date.now());
  const reportHasWindow = RATE_LIMIT_FIELDS.some((f) => fromReport[f] !== undefined);
  // A cold start has no line at all, live or remembered. The card used to
  // return null there and the daemon's perfectly good reading stayed invisible.
  if (!whole && !reportHasWindow) return null;
  if (whole) lastRateLimit[cacheKey] = { sl: whole, ts: wholeTs, fields };
  // Compose what the card renders: the freshest whole line for everything that
  // is not a window, each window figure from its own freshest carrier.
  best = { ...(whole ?? {}) };
  for (const f of RATE_LIMIT_FIELDS) {
    const c = fields[f];
    if (c) best[f] = c.value;
    else delete best[f];
  }
  // …and the report fills only what nothing live ever carried. Per field, like
  // the cache above: a session that reported a 5h percentage and never a 7d one
  // is silent about the 7d window, not a report that there isn't one.
  for (const f of RATE_LIMIT_FIELDS) {
    if (best[f] === undefined && fromReport[f] !== undefined) best[f] = fromReport[f];
  }

  // Render a window when Claude gives us a utilization % OR just a reset time.
  // Many accounts only report the reset while comfortably within a window, so a
  // pct-less row shows the label + reset countdown (an empty meter track).
  const Row: React.FC<{ label: string; pct?: number; reset?: number; title?: string }> = ({
    label,
    pct,
    reset,
    title: tip,
  }) =>
    pct === undefined && reset === undefined ? null : (
      <div title={tip} style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
        <span
          style={{ fontSize: '0.66rem', color: 'var(--wks-text-faint)', width: 22, flexShrink: 0 }}
        >
          {label}
        </span>
        <span
          style={{
            flex: 1,
            height: 5,
            borderRadius: 3,
            background: 'var(--wks-border-subtle)',
            overflow: 'hidden',
          }}
        >
          {pct !== undefined && (
            <span
              style={{
                display: 'block',
                height: '100%',
                width: `${Math.max(2, Math.min(100, pct))}%`,
                background: limitColor(pct),
              }}
            />
          )}
        </span>
        <span
          style={{
            fontSize: '0.69rem',
            fontWeight: 700,
            color: pct !== undefined ? limitColor(pct) : 'var(--wks-text-faint)',
            fontVariantNumeric: 'tabular-nums',
            flexShrink: 0,
          }}
        >
          {pct !== undefined ? `${Math.round(pct)}%` : reset ? `resets ${fmtReset(reset)}` : 'ok'}
        </span>
        {pct !== undefined && reset ? (
          <span style={{ fontSize: '0.6rem', color: 'var(--wks-text-faint)', flexShrink: 0 }}>
            {fmtReset(reset)}
          </span>
        ) : null}
      </div>
    );

  return (
    <>
      <div
        role="button"
        tabIndex={0}
        aria-label="Show usage detail"
        title="Usage and account limits. Click for detail."
        onClick={() => setDetailOpen(true)}
        onKeyDown={(e) => {
          if (e.key === 'Enter' || e.key === ' ') {
            e.preventDefault();
            setDetailOpen(true);
          }
        }}
        onMouseEnter={(e) => {
          (e.currentTarget as HTMLElement).style.borderColor = 'var(--wks-accent)';
        }}
        onMouseLeave={(e) => {
          (e.currentTarget as HTMLElement).style.borderColor = 'var(--wks-border-subtle)';
        }}
        style={{
          flex: 1,
          minWidth: 220,
          padding: '15px 16px',
          borderRadius: 'var(--wks-radius-md)',
          background: 'var(--wks-bg-raised)',
          border: '1px solid var(--wks-border-subtle)',
          display: 'flex',
          flexDirection: 'column',
          gap: 6,
          cursor: 'pointer',
          transition: 'border-color 0.12s',
        }}
      >
        <div
          style={{
            display: 'flex',
            alignItems: 'center',
            gap: 6,
            fontSize: '0.62rem',
            color: 'var(--wks-text-faint)',
            textTransform: 'uppercase',
            letterSpacing: '0.08em',
          }}
        >
          {/* Same provider mark vocabulary as the sidebar / nav / spawn dialog. */}
          <AgentLogo
            provider={provider as AgentProvider}
            size={13}
            style={{ color: 'var(--wks-text-secondary)', flexShrink: 0 }}
          />
          {title}
        </div>
        {/* One row per window the provider reported, labelled with the window's
            own length where it is known. Codex has no monthly window and Claude
            has one only while extra usage is enabled, so the rows come from the
            data, not from a fixed list of three. */}
        {usageWindows(best).map((w) => {
          const length = fmtWindowLength(w.windowMins);
          return (
            <Row
              key={w.key}
              label={w.short}
              pct={w.pct}
              reset={w.resetsAt}
              // The label column is 22px wide, so the window's full length rides
              // in the tooltip — and in the dialog this card now opens.
              title={[
                length ? `${w.label} (${length} window)` : w.label,
                w.pct !== undefined ? `${Math.round(w.pct)}% used` : undefined,
                w.resetsAt ? fmtReset(w.resetsAt) : undefined,
              ]
                .filter(Boolean)
                .join(' · ')}
            />
          );
        })}
      </div>
      {/* Outside the card on purpose: a portal still bubbles its events through
          the REACT tree, so a dialog nested inside the card would reopen itself
          the moment a backdrop click closed it. Account-scoped, because `best`
          is whichever session reported the account's windows most recently and
          its own tokens and cost are not this card's to show. */}
      {detailOpen && (
        <UsageDetailDialog
          snapshot={{ statusLine: best, provider }}
          scope="account"
          onClose={() => setDetailOpen(false)}
        />
      )}
    </>
  );
};

/** Stat tile — mockup "Overview" card: uppercase mono label on top, large
 *  mono value, optional sub-line. Matches the Usage pane's Stat exactly.
 *  With `onClick` the tile becomes a navigation shortcut (hover ring). */
const Stat: React.FC<{
  label: string;
  value: string;
  sub?: string;
  color?: string;
  onClick?: () => void;
  clickTitle?: string;
}> = ({ label, value, sub, color, onClick, clickTitle }) => (
  <div
    onClick={onClick}
    title={onClick ? clickTitle : undefined}
    style={{
      flex: 1,
      minWidth: 130,
      padding: '15px 16px',
      borderRadius: 'var(--wks-radius-md)',
      background: 'var(--wks-bg-raised)',
      border: '1px solid var(--wks-border-subtle)',
      cursor: onClick ? 'pointer' : undefined,
      transition: 'border-color 0.12s',
    }}
    onMouseEnter={(e) => {
      if (onClick) (e.currentTarget as HTMLElement).style.borderColor = 'var(--wks-accent)';
    }}
    onMouseLeave={(e) => {
      if (onClick) (e.currentTarget as HTMLElement).style.borderColor = 'var(--wks-border-subtle)';
    }}
  >
    <div
      style={{
        fontSize: '0.62rem',
        color: 'var(--wks-text-faint)',
        textTransform: 'uppercase',
        letterSpacing: '0.08em',
      }}
    >
      {label}
    </div>
    <div
      style={{
        fontSize: '1.5rem',
        fontWeight: 700,
        color: color || 'var(--wks-text-primary)',
        fontVariantNumeric: 'tabular-nums',
        marginTop: 8,
      }}
    >
      {value}
    </div>
    {sub && (
      <div
        style={{
          fontSize: '0.66rem',
          color: 'var(--wks-text-secondary)',
          marginTop: 3,
          fontVariantNumeric: 'tabular-nums',
        }}
      >
        {sub}
      </div>
    )}
  </div>
);

const DirRow: React.FC<{
  dir: string;
  fav: boolean;
  projects?: Record<string, ProjectIdentity>;
  onSpawn: () => void;
  onToggleFav: () => void;
}> = ({ dir, fav, projects, onSpawn, onToggleFav }) => (
  <div
    onClick={onSpawn}
    title={`Dispatch agent in ${dir}`}
    style={{
      display: 'flex',
      alignItems: 'center',
      gap: 10,
      padding: '7px 10px',
      borderRadius: 9,
      border: '1px solid transparent',
      cursor: 'pointer',
      transition: 'background 0.12s, border-color 0.12s',
    }}
    onMouseEnter={(e) => {
      (e.currentTarget as HTMLElement).style.background = 'var(--wks-bg-selected)';
      (e.currentTarget as HTMLElement).style.borderColor = 'var(--wks-border-subtle)';
    }}
    onMouseLeave={(e) => {
      (e.currentTarget as HTMLElement).style.background = 'transparent';
      (e.currentTarget as HTMLElement).style.borderColor = 'transparent';
    }}
  >
    <span
      onClick={(e) => {
        e.stopPropagation();
        onToggleFav();
      }}
      title={fav ? 'Unfavourite' : 'Favourite'}
      style={{
        display: 'flex',
        alignItems: 'center',
        flexShrink: 0,
        cursor: 'pointer',
        color: fav ? 'var(--wks-warning)' : 'var(--wks-text-faint)',
      }}
    >
      <Star size={14} strokeWidth={1.75} fill={fav ? 'currentColor' : 'none'} />
    </span>
    {/* The same mark the sidebar draws, so a directory here and the agents it
        spawns read as one project rather than two unrelated lists. */}
    <ProjectMark cwd={dir} projects={projects} size={15} />
    <span
      style={{
        fontSize: '0.8rem',
        fontWeight: 600,
        color: 'var(--wks-text-primary)',
        flexShrink: 0,
      }}
    >
      {basename(dir)}
    </span>
    <span
      style={{
        fontSize: '0.66rem',
        color: 'var(--wks-text-faint)',
        overflow: 'hidden',
        textOverflow: 'ellipsis',
        whiteSpace: 'nowrap',
      }}
    >
      {dir}
    </span>
    <span
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: 3,
        marginLeft: 'auto',
        flexShrink: 0,
        fontSize: '0.66rem',
        color: 'var(--wks-accent)',
      }}
    >
      <Plus size={11} strokeWidth={2.2} /> launch
    </span>
  </div>
);

const OverviewPane: React.FC<{ title?: string; agents?: { sessionId?: string }[] }> = ({
  agents: workspaceAgents = [],
}) => {
  const { config, save } = useConfig();
  // "Need you" comes from the single attention feed (the spine), not a parallel
  // ambient-state count, so this stat matches the SideBar / Inbox / Fleet exactly.
  const { counts, setViewLevel, openInbox } = useAttention();
  // The desktop's own session-history store — the five figures of recorded
  // spend this pane used to render as "$0.00 this session".
  const analytics = useSessionAnalytics();
  // Per-session recorded cost, for workspace agents with no live snapshot.
  const recordedUsage = useRecordedUsageMap();
  const { plugins } = usePlugins();
  const pluginStates = usePluginStates();
  const updateStatus = useUpdateStatus();
  // Shared with every RateLimitCard below — one fetch, many readers. Read here
  // so a second Claude login gets its card on a COLD start too: the account
  // list is otherwise discovered only from live sessions and from cards drawn
  // earlier this run, neither of which exists a second after launch.
  const usageReport = useUsageReport();
  const [snaps, setSnaps] = useState<Snap[]>([]);

  const pendingRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const refresh = useCallback(() => {
    window.electronAPI
      .getAllClaudeSessions?.()
      .then((s: any[]) => setSnaps(Array.isArray(s) ? s : []))
      .catch(() => {});
  }, []);
  // Throttle the per-update refresh to at most once per 1000 ms (trailing) so
  // streaming agents (~9 updates/s) don't trigger a fetch storm.
  const throttledRefresh = useCallback(() => {
    if (pendingRef.current !== null) return;
    pendingRef.current = setTimeout(() => {
      pendingRef.current = null;
      refresh();
    }, 1000);
  }, [refresh]);
  useEffect(() => {
    refresh();
    const off = window.electronAPI.onClaudeSessionUpdate?.(() => throttledRefresh());
    // Idle heartbeat: the update feed goes quiet when no agent is emitting
    // events, freezing everything time- or externally-driven — the "resets in
    // Xm" countdown, rate-limit bars fed by sessions running outside
    // workspacer, and ended sessions the store evicts silently. A slow clock
    // keeps the dashboard honest between events.
    const tick = setInterval(refresh, 30_000);
    return () => {
      off?.();
      clearInterval(tick);
      if (pendingRef.current !== null) {
        clearTimeout(pendingRef.current);
        pendingRef.current = null;
      }
    };
  }, [refresh, throttledRefresh]);

  // One source of truth for "what projects exist", reading both the projects map
  // and the legacy directories arrays — see lib/projectRegistry.
  const favourites = favouriteProjects(config).map((p) => p.dir);
  const recentOnly = recentProjects(config).map((p) => p.dir);

  // Scope stats to workspacer's OWN agents — claudemon tracks every Claude
  // session on the machine (incl. ones run outside workspacer), so we filter
  // the snapshots down to the sessions our agents own.
  const ownSessionIds = new Set(
    workspaceAgents.map((a) => a.sessionId).filter(Boolean) as string[],
  );
  const own = snaps.filter((s) => ownSessionIds.has(s.sessionId));

  const agents = workspaceAgents.length;
  const working = own.filter(
    (s) =>
      s.ambientState === 'thinking' ||
      s.ambientState === 'streaming' ||
      s.ambientState === 'background',
  ).length;
  const needsYou = counts.needsYou;
  // Same source-of-truth order as the per-card path (deriveSessionStats):
  // Claude's own statusLine cost is authoritative, transcript-derived usage
  // is the fallback — summing usage alone reads $0.00 while every card
  // shows a statusLine cost.
  //
  // The reduce below used to `?? 0` its way to a confident "$0.00 this session"
  // at every cold start: a restored agent's session is a stopped daemon row,
  // `promoteSessionSnapshots` drops it, so `own` is EMPTY and the sum of an
  // empty list is zero — displayed beside a database holding five figures.
  //
  // Now each agent contributes its live figure, or the history DB's last
  // recorded one, or NOTHING. `contributing` counts the agents that actually
  // had a figure, so the tile can say what the total covers, and a total
  // covering nothing renders as a dash rather than as a measured zero.
  const costBySession = new Map(
    own.map((s) => [s.sessionId, s.statusLine?.costUSD ?? s.usage?.costUSD]),
  );
  let totalCost = 0;
  let contributing = 0;
  for (const sid of ownSessionIds) {
    const cost = costBySession.get(sid) ?? recordedUsage[sid]?.costUSD;
    if (cost === undefined) continue;
    totalCost += cost;
    contributing++;
  }
  // Lifetime spend across every session the desktop ever recorded — the answer
  // to "the tile says $0.00 and the database says otherwise". Deliberately a
  // SECOND tile: the workspace figure above is about what is running now, and
  // silently swapping it for a lifetime number would answer a question nobody
  // asked while losing the one they did.
  const lifetime = analytics.summary?.totals;

  // Directory rows open the new-agent view pre-filled with this cwd (and the
  // last harness/provider used, restored from config in the dialog) rather than
  // spawning straight away — so you can tweak the model/provider before launch.
  const spawnIn = (cwd: string) => {
    window.electronAPI.hubPublish?.({
      type: 'command.open_spawn_dialog',
      source: 'workspacer.overview',
      data: { cwd },
    });
  };
  // Open the new-agent view at the default directory, same as every other spawn
  // entry point — the working directory is picked/changed inside the dialog, so
  // no OS folder prompt up front.
  const newAgent = () => {
    window.electronAPI.hubPublish?.({
      type: 'command.open_spawn_dialog',
      source: 'workspacer.overview',
      data: {},
    });
  };
  const toggleFav = (dir: string) => {
    save(setFavourite(config, dir, !favourites.includes(dir)));
  };

  return (
    <div
      style={{
        position: 'relative',
        height: '100%',
        overflow: 'hidden',
        background: 'var(--wks-bg-base)',
        color: 'var(--wks-text-primary)',
      }}
    >
      {/* Soft accent glow behind the hero — same decoration as the spawn
          dialog, fixed while the content scrolls beneath it. */}
      <div
        aria-hidden
        style={{
          position: 'absolute',
          top: '-22%',
          left: '50%',
          transform: 'translateX(-50%)',
          width: 720,
          height: 720,
          borderRadius: '50%',
          background:
            'radial-gradient(circle, color-mix(in srgb, var(--wks-accent) 8%, transparent) 0%, transparent 65%)',
          pointerEvents: 'none',
        }}
      />

      <div style={{ position: 'relative', height: '100%', overflowY: 'auto' }}>
        <div
          style={{
            maxWidth: 860,
            margin: '0 auto',
            padding: '44px 28px 40px',
            boxSizing: 'border-box',
            animation: 'wks-fade-in 0.25s ease-out',
          }}
        >
          {/* ── Pending update banner ────────────────────────────────────── */}
          {updateStatus &&
            (updateStatus.state === 'downloaded' || updateStatus.state === 'downloading') && (
              <div
                style={{
                  display: 'flex',
                  alignItems: 'center',
                  gap: 12,
                  marginBottom: 22,
                  padding: '11px 16px',
                  borderRadius: 'var(--wks-radius-md)',
                  border: '1px solid color-mix(in srgb, var(--wks-accent) 35%, transparent)',
                  background: 'color-mix(in srgb, var(--wks-accent) 9%, transparent)',
                  animation: 'wks-fade-in 0.25s ease-out',
                }}
              >
                <RefreshCw
                  size={16}
                  strokeWidth={1.75}
                  style={{
                    color: 'var(--wks-accent-text)',
                    flexShrink: 0,
                    animation:
                      updateStatus.state === 'downloading'
                        ? 'wks-spin 1.2s linear infinite'
                        : 'none',
                  }}
                />
                <div style={{ flex: 1, minWidth: 0 }}>
                  <div style={{ fontSize: '0.8rem', fontWeight: 650 }}>
                    {updateStatus.state === 'downloaded'
                      ? `Update ready — Workspacer v${updateStatus.version ?? ''}`
                      : `Downloading update${updateStatus.version ? ` v${updateStatus.version}` : ''}…${
                          updateStatus.percent != null ? ` ${updateStatus.percent}%` : ''
                        }`}
                  </div>
                  <div style={{ fontSize: '0.7rem', color: 'var(--wks-text-muted)', marginTop: 2 }}>
                    {updateStatus.state === 'downloaded'
                      ? 'Restart to apply — your session is saved on quit.'
                      : 'You can keep working; it installs on restart once ready.'}
                  </div>
                </div>
                {updateStatus.state === 'downloaded' && (
                  <button
                    onClick={() => window.electronAPI.updatesInstall?.().catch(() => {})}
                    style={{
                      flexShrink: 0,
                      fontSize: '0.74rem',
                      fontFamily: 'inherit',
                      fontWeight: 700,
                      cursor: 'pointer',
                      background: 'var(--wks-accent)',
                      color: 'var(--wks-text-on-accent)',
                      border: 'none',
                      borderRadius: 6,
                      padding: '7px 14px',
                    }}
                  >
                    Restart now
                  </button>
                )}
              </div>
            )}

          {/* ── Hero: the workspace at a glance ─────────────────────────── */}
          <div style={{ textAlign: 'center', marginBottom: 30 }}>
            <div
              style={{
                width: 64,
                height: 64,
                margin: '0 auto',
                borderRadius: '50%',
                display: 'flex',
                alignItems: 'center',
                justifyContent: 'center',
                border: '1px solid var(--wks-border-input)',
                background: 'color-mix(in srgb, var(--wks-accent) 5%, transparent)',
                color: 'var(--wks-accent-text)',
              }}
            >
              <Home size={26} strokeWidth={1.7} />
            </div>
            <div
              style={{
                marginTop: 16,
                fontSize: '1.05rem',
                fontWeight: 650,
                letterSpacing: '-0.01em',
                color: 'var(--wks-text-primary)',
              }}
            >
              Workspace
            </div>
            <div
              style={{
                marginTop: 6,
                fontSize: '0.72rem',
                color: 'var(--wks-text-muted)',
                fontVariantNumeric: 'tabular-nums',
                display: 'flex',
                justifyContent: 'center',
                alignItems: 'center',
                gap: 7,
                flexWrap: 'wrap',
              }}
            >
              <span>
                {agents} agent{agents === 1 ? '' : 's'}
              </span>
              <span style={{ color: 'var(--wks-text-disabled)' }}>·</span>
              <span>{working} working</span>
              <span style={{ color: 'var(--wks-text-disabled)' }}>·</span>
              <span>
                {needsYou} need{needsYou === 1 ? 's' : ''} you
              </span>
            </div>
            <button
              onClick={newAgent}
              style={{
                marginTop: 20,
                fontSize: '0.75rem',
                fontFamily: 'inherit',
                cursor: 'pointer',
                background: 'var(--wks-accent)',
                color: 'var(--wks-text-on-accent)',
                border: 'none',
                borderRadius: 8,
                padding: '7px 20px',
                fontWeight: 600,
              }}
            >
              ＋ Dispatch agent…
            </button>
          </div>

          {/* ── Fleet Manager — cross-project delegation, below Dispatch agent ── */}
          <FleetManagerHero />

          <div style={{ display: 'flex', gap: 12, flexWrap: 'wrap', marginBottom: 26 }}>
            <Stat
              label="Agents"
              value={String(agents)}
              sub={working ? `${working} in flight` : 'all standing by'}
              onClick={() => setViewLevel('fleet')}
              clickTitle="Open the Fleet"
            />
            <Stat
              label="In flight"
              value={String(working)}
              color={working ? 'var(--wks-busy)' : undefined}
              onClick={() => setViewLevel('fleet')}
              clickTitle="Open the Fleet"
            />
            <Stat
              label="Waiting"
              value={String(needsYou)}
              color={needsYou ? 'var(--wks-warning)' : undefined}
              onClick={openInbox}
              clickTitle="Open the Inbox"
            />
            <Stat
              label="Workspace cost"
              // A dash, not $0.00: no agent in this workspace has a figure —
              // live or recorded — so nothing was measured to be zero.
              value={contributing > 0 ? fmtUSD(totalCost) : '—'}
              sub={
                contributing === 0
                  ? agents === 0
                    ? 'no agents'
                    : 'no usage recorded'
                  : contributing === agents
                    ? 'this workspace'
                    : `${contributing} of ${agents} agents`
              }
            />
            <Stat
              label="All time"
              value={lifetime ? fmtUSD(lifetime.costUSD) : '—'}
              sub={
                analytics.loading
                  ? 'reading history…'
                  : analytics.unavailable
                    ? // Unreachable is not empty. A headless hub answers these
                      // methods with an all-zero stub, and rendering that as
                      // "$0.00 across 0 sessions" would invent a measurement.
                      'history unavailable'
                    : lifetime
                      ? `${lifetime.sessions} session${lifetime.sessions === 1 ? '' : 's'}${
                          analytics.unrecordedSessions > 0
                            ? ` · ${analytics.unrecordedComplete ? '' : '≥'}${
                                analytics.unrecordedSessions
                              } un-costed`
                            : ''
                        }`
                      : 'no sessions recorded'
              }
            />
            {/* Account-wide 5h/7d rate-limit windows, one card per ACCOUNT
                (scanned across all sessions, not just workspacer's — they're
                global to each account). For Claude that means one card per
                login: a second-account profile's sessions group by config
                root, each with its own windows. */}
            {RATE_LIMIT_PROVIDERS.map((p) => {
              if (p.id !== 'claude') {
                return <RateLimitCard key={p.id} snaps={snaps} provider={p.id} title={p.title} />;
              }
              // Cached groups keep their card after the last session of an
              // account ends — same eviction-survival as the provider cache.
              const accounts = new Set<string>(['']);
              for (const s of snaps) {
                if ((s.provider ?? 'claude') === 'claude') {
                  accounts.add(claudeAccountOf(s.transcriptPath));
                }
              }
              for (const k of Object.keys(lastRateLimit)) {
                if (k.startsWith('claude:')) accounts.add(k.slice('claude:'.length));
              }
              // …and the accounts the daemon has a RUNNING window for, which is
              // the only source that speaks before the first session starts.
              for (const k of reportAccountKeys(usageReport, 'claude', Date.now())) {
                accounts.add(k);
              }
              return [...accounts]
                .sort()
                .map((a) => (
                  <RateLimitCard
                    key={`claude:${a}`}
                    snaps={snaps}
                    provider="claude"
                    title={a ? `Claude usage — ${a}` : p.title}
                    account={a}
                  />
                ));
            })}
          </div>

          {plugins.length > 0 && (
            <Section title="Plugins">
              <div
                style={{
                  display: 'grid',
                  gridTemplateColumns: 'repeat(auto-fill, minmax(220px, 1fr))',
                  gap: 12,
                  padding: '0 2px',
                }}
              >
                {plugins.map((p) => {
                  const hasServer = !!p.server;
                  const state = hasServer ? (pluginStates[p.id] ?? 'starting') : 'no server';
                  const color = pluginStateColor(hasServer ? pluginStates[p.id] : undefined);
                  const glyph = p.panes?.[0]?.icon || (p.name || p.id).charAt(0).toUpperCase();
                  return (
                    <div
                      key={p.id}
                      title={`${p.name || p.id}${hasServer ? ` — ${state}` : ''}`}
                      style={{
                        display: 'flex',
                        alignItems: 'center',
                        gap: 12,
                        padding: '13px 14px',
                        background: 'var(--wks-bg-raised)',
                        border: '1px solid var(--wks-border-subtle)',
                        borderRadius: 'var(--wks-radius-md)',
                      }}
                    >
                      <span
                        style={{
                          width: 34,
                          height: 34,
                          borderRadius: 9,
                          flexShrink: 0,
                          display: 'flex',
                          alignItems: 'center',
                          justifyContent: 'center',
                          background: 'var(--wks-bg-base)',
                          border: '1px solid var(--wks-border-subtle)',
                          fontSize: '0.95rem',
                          color: 'var(--wks-accent)',
                        }}
                      >
                        {glyph}
                      </span>
                      <div style={{ flex: 1, minWidth: 0 }}>
                        <div
                          style={{
                            fontSize: '0.82rem',
                            fontWeight: 600,
                            color: 'var(--wks-text-primary)',
                            overflow: 'hidden',
                            textOverflow: 'ellipsis',
                            whiteSpace: 'nowrap',
                          }}
                        >
                          {p.name || p.id}
                        </div>
                        <div
                          style={{
                            fontSize: '0.66rem',
                            color: 'var(--wks-text-faint)',
                            marginTop: 1,
                            overflow: 'hidden',
                            textOverflow: 'ellipsis',
                            whiteSpace: 'nowrap',
                          }}
                        >
                          {hasServer
                            ? state
                            : `${p.panes?.length ?? 0} pane${(p.panes?.length ?? 0) === 1 ? '' : 's'}`}
                        </div>
                      </div>
                      {hasServer && (
                        <span
                          style={{
                            width: 8,
                            height: 8,
                            borderRadius: 'var(--wks-radius-pill)',
                            flexShrink: 0,
                            background: color,
                          }}
                        />
                      )}
                    </div>
                  );
                })}
              </div>
            </Section>
          )}

          {favourites.length > 0 && (
            <Section title="Favourites">
              {favourites.map((d) => (
                <DirRow
                  key={d}
                  dir={d}
                  fav
                  projects={config.projects}
                  onSpawn={() => spawnIn(d)}
                  onToggleFav={() => toggleFav(d)}
                />
              ))}
            </Section>
          )}

          <Section title="Recent directories">
            {recentOnly.length === 0 ? (
              <div style={{ padding: '10px', fontSize: '0.72rem', color: 'var(--wks-text-faint)' }}>
                No recent directories yet. Dispatch an agent and it'll show up here for quick
                relaunch.
              </div>
            ) : (
              recentOnly.map((d) => (
                <DirRow
                  key={d}
                  dir={d}
                  fav={false}
                  projects={config.projects}
                  onSpawn={() => spawnIn(d)}
                  onToggleFav={() => toggleFav(d)}
                />
              ))
            )}
          </Section>
        </div>
      </div>
    </div>
  );
};

const Section: React.FC<{ title: string; children: React.ReactNode }> = ({ title, children }) => (
  <div style={{ marginBottom: 24 }}>
    <div
      style={{
        fontSize: '0.62rem',
        color: 'var(--wks-text-faint)',
        fontWeight: 600,
        textTransform: 'uppercase',
        letterSpacing: '0.08em',
        padding: '0 10px 6px',
      }}
    >
      {title}
    </div>
    {children}
  </div>
);

export default OverviewPane;
