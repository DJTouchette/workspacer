import React, { useCallback, useEffect, useRef, useState } from 'react';
import { useConfig } from '../hooks/useConfig';
import { useAttention } from '../contexts/AttentionContext';
import { usePlugins } from '../hooks/usePlugins';
import { Home, Star, Plus } from '../components/icons';

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

function pluginStateColor(s: string | undefined): string {
  switch (s) {
    case 'healthy':
    case 'running':
      return 'var(--wks-success, #3fb950)';
    case 'unhealthy':
      return 'var(--wks-warning, #e0a000)';
    case 'crashed':
      return 'var(--wks-danger, #e05555)';
    default:
      return 'var(--wks-text-faint, #666)';
  }
}

interface Snap {
  sessionId: string;
  ambientState?: string;
  cwd?: string;
  usage?: { costUSD?: number; contextTokens?: number } | null;
  statusLine?: {
    fiveHourPct?: number;
    fiveHourResetsAt?: number;
    sevenDayPct?: number;
    sevenDayResetsAt?: number;
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
  if (pct >= 80) return 'var(--wks-danger, #e05555)';
  if (pct >= 50) return 'var(--wks-warning, #e0a000)';
  return 'var(--wks-success, #3fb950)';
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

/**
 * The 5h/7d rate-limit windows are account-global (identical across every
 * session), so we surface them once. Pick the freshest statusLine that carries
 * them — newest `receivedAt` wins.
 */
const RateLimitCard: React.FC<{ snaps: Snap[] }> = ({ snaps }) => {
  let best: NonNullable<Snap['statusLine']> | null = null;
  let bestTs = -1;
  for (const s of snaps) {
    const sl = s.statusLine;
    if (!sl || (sl.fiveHourPct === undefined && sl.sevenDayPct === undefined)) continue;
    const ts = sl.receivedAt ? Date.parse(sl.receivedAt) : 0;
    if (ts >= bestTs) {
      bestTs = ts;
      best = sl;
    }
  }
  if (!best) return null;

  const Row: React.FC<{ label: string; pct?: number; reset?: number }> = ({ label, pct, reset }) =>
    pct === undefined ? null : (
      <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
        <span
          style={{ fontSize: '0.62rem', color: 'var(--wks-text-faint)', width: 22, flexShrink: 0 }}
        >
          {label}
        </span>
        <span
          style={{
            flex: 1,
            height: 5,
            borderRadius: 3,
            background: 'var(--wks-border-subtle, #2a2a2a)',
            overflow: 'hidden',
          }}
        >
          <span
            style={{
              display: 'block',
              height: '100%',
              width: `${Math.max(2, Math.min(100, pct))}%`,
              background: limitColor(pct),
            }}
          />
        </span>
        <span
          style={{
            fontSize: '0.66rem',
            fontWeight: 700,
            color: limitColor(pct),
            fontVariantNumeric: 'tabular-nums',
            flexShrink: 0,
          }}
        >
          {Math.round(pct)}%
        </span>
        {reset ? (
          <span style={{ fontSize: '0.55rem', color: 'var(--wks-text-faint)', flexShrink: 0 }}>
            {fmtReset(reset)}
          </span>
        ) : null}
      </div>
    );

  return (
    <div
      style={{
        flex: 1,
        minWidth: 220,
        padding: '15px 16px',
        borderRadius: 'var(--wks-radius-md, 13px)',
        background: 'var(--wks-bg-raised)',
        border: '1px solid var(--wks-border-subtle)',
        display: 'flex',
        flexDirection: 'column',
        gap: 6,
      }}
    >
      <div
        style={{
          fontSize: '0.6rem',
          color: 'var(--wks-text-faint)',
          textTransform: 'uppercase',
          letterSpacing: '0.08em',
        }}
      >
        Account usage
      </div>
      <Row label="5h" pct={best.fiveHourPct} reset={best.fiveHourResetsAt} />
      <Row label="7d" pct={best.sevenDayPct} reset={best.sevenDayResetsAt} />
    </div>
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
      borderRadius: 'var(--wks-radius-md, 13px)',
      background: 'var(--wks-bg-raised)',
      border: '1px solid var(--wks-border-subtle)',
      cursor: onClick ? 'pointer' : undefined,
      transition: 'border-color 0.12s',
    }}
    onMouseEnter={(e) => {
      if (onClick)
        (e.currentTarget as HTMLElement).style.borderColor = 'var(--wks-accent, #4a9eff)';
    }}
    onMouseLeave={(e) => {
      if (onClick) (e.currentTarget as HTMLElement).style.borderColor = 'var(--wks-border-subtle)';
    }}
  >
    <div
      style={{
        fontSize: '0.6rem',
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
          fontSize: '0.62rem',
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
  onSpawn: () => void;
  onToggleFav: () => void;
}> = ({ dir, fav, onSpawn, onToggleFav }) => (
  <div
    onClick={onSpawn}
    title={`Launch an agent in ${dir}`}
    style={{
      display: 'flex',
      alignItems: 'center',
      gap: 10,
      padding: '7px 10px',
      borderRadius: 7,
      cursor: 'pointer',
    }}
    onMouseEnter={(e) => {
      (e.currentTarget as HTMLElement).style.background = 'var(--wks-bg-selected)';
    }}
    onMouseLeave={(e) => {
      (e.currentTarget as HTMLElement).style.background = 'transparent';
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
        color: fav ? 'var(--wks-warning, #e0a000)' : 'var(--wks-text-faint)',
      }}
    >
      <Star size={14} strokeWidth={1.75} fill={fav ? 'currentColor' : 'none'} />
    </span>
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
        fontSize: '0.62rem',
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
        fontSize: '0.62rem',
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
  const { plugins } = usePlugins();
  const pluginStates = usePluginStates();
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
    return () => {
      off?.();
      if (pendingRef.current !== null) {
        clearTimeout(pendingRef.current);
        pendingRef.current = null;
      }
    };
  }, [refresh, throttledRefresh]);

  const recent = config.directories?.recent ?? [];
  const favourites = config.directories?.favourites ?? [];
  const favSet = new Set(favourites);
  const recentOnly = recent.filter((d) => !favSet.has(d));

  // Scope stats to workspacer's OWN agents — claudemon tracks every Claude
  // session on the machine (incl. ones run outside workspacer), so we filter
  // the snapshots down to the sessions our agents own.
  const ownSessionIds = new Set(
    workspaceAgents.map((a) => a.sessionId).filter(Boolean) as string[],
  );
  const own = snaps.filter((s) => ownSessionIds.has(s.sessionId));

  const agents = workspaceAgents.length;
  const working = own.filter(
    (s) => s.ambientState === 'thinking' || s.ambientState === 'streaming',
  ).length;
  const needsYou = counts.needsYou;
  const totalCost = own.reduce((n, s) => n + (s.usage?.costUSD ?? 0), 0);

  const spawnIn = (cwd: string) => {
    window.electronAPI.hubPublish?.({
      type: 'command.spawn_agent',
      source: 'workspacer.overview',
      data: { cwd },
    });
  };
  const browse = async () => {
    const dir = await window.electronAPI.pickFolder?.();
    if (dir) spawnIn(dir);
  };
  const toggleFav = (dir: string) => {
    const set = new Set(favourites);
    if (set.has(dir)) set.delete(dir);
    else set.add(dir);
    save({ directories: { recent, favourites: Array.from(set) } });
  };

  return (
    <div
      style={{
        height: '100%',
        overflow: 'auto',
        background: 'var(--wks-bg-base)',
        color: 'var(--wks-text-primary)',
        padding: 18,
      }}
    >
      <div style={{ display: 'flex', alignItems: 'baseline', gap: 11, marginBottom: 16 }}>
        <div
          style={{
            display: 'flex',
            alignItems: 'center',
            gap: 8,
            fontSize: '1.05rem',
            fontWeight: 700,
          }}
        >
          <Home size={18} strokeWidth={1.9} /> Workspace
        </div>
        <div
          style={{
            fontSize: '0.72rem',
            color: 'var(--wks-text-secondary)',
            fontVariantNumeric: 'tabular-nums',
          }}
        >
          {agents} agent{agents === 1 ? '' : 's'} · {working} working · {needsYou} need
          {needsYou === 1 ? 's' : ''} you
        </div>
        <button
          onClick={browse}
          style={{
            marginLeft: 'auto',
            fontSize: '0.72rem',
            fontFamily: 'inherit',
            cursor: 'pointer',
            background: 'var(--wks-accent)',
            color: 'var(--wks-text-on-accent, #fff)',
            border: 'none',
            borderRadius: 6,
            padding: '6px 12px',
            fontWeight: 600,
          }}
        >
          ＋ New agent…
        </button>
      </div>

      <div style={{ display: 'flex', gap: 12, flexWrap: 'wrap', marginBottom: 22 }}>
        <Stat
          label="Agents"
          value={String(agents)}
          sub={working ? `${working} working` : 'all idle'}
          onClick={() => setViewLevel('fleet')}
          clickTitle="Open the Fleet deck"
        />
        <Stat
          label="Working"
          value={String(working)}
          color={working ? 'var(--wks-busy, var(--wks-accent, #4a9eff))' : undefined}
          onClick={() => setViewLevel('fleet')}
          clickTitle="Open the Fleet deck"
        />
        <Stat
          label="Need you"
          value={String(needsYou)}
          color={needsYou ? 'var(--wks-warning, #e0a000)' : undefined}
          onClick={openInbox}
          clickTitle="Open the Inbox"
        />
        <Stat label="Total cost" value={fmtUSD(totalCost)} sub="this session" />
        {/* Account-wide 5h/7d rate-limit windows (scanned across all sessions,
            not just workspacer's — they're global to the account). */}
        <RateLimitCard snaps={snaps} />
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
                    borderRadius: 'var(--wks-radius-md, 13px)',
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
                        fontSize: '0.62rem',
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
                        borderRadius: 99,
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
              onSpawn={() => spawnIn(d)}
              onToggleFav={() => toggleFav(d)}
            />
          ))}
        </Section>
      )}

      <Section title="Recent directories">
        {recentOnly.length === 0 ? (
          <div style={{ padding: '10px', fontSize: '0.72rem', color: 'var(--wks-text-faint)' }}>
            No recent directories yet. Spawn an agent and it'll show up here for quick relaunch.
          </div>
        ) : (
          recentOnly.map((d) => (
            <DirRow
              key={d}
              dir={d}
              fav={false}
              onSpawn={() => spawnIn(d)}
              onToggleFav={() => toggleFav(d)}
            />
          ))
        )}
      </Section>
    </div>
  );
};

const Section: React.FC<{ title: string; children: React.ReactNode }> = ({ title, children }) => (
  <div style={{ marginBottom: 18 }}>
    <div
      style={{
        fontSize: '0.58rem',
        color: 'var(--wks-text-disabled)',
        fontWeight: 600,
        textTransform: 'uppercase',
        letterSpacing: '0.05em',
        padding: '0 10px 4px',
      }}
    >
      {title}
    </div>
    {children}
  </div>
);

export default OverviewPane;
