import React, { useCallback, useEffect, useState } from 'react';
import { useConfig } from '../hooks/useConfig';
import { Home, Star, Plus } from '../components/icons';

interface Snap {
  sessionId: string;
  ambientState?: string;
  cwd?: string;
  usage?: { costUSD?: number; contextTokens?: number } | null;
}

function basename(p: string): string {
  return p.replace(/[/\\]+$/, '').split(/[/\\]/).pop() || p;
}
function fmtUSD(n: number): string {
  return n >= 0.01 ? `$${n.toFixed(2)}` : n > 0 ? '<$0.01' : '$0.00';
}

const Stat: React.FC<{ label: string; value: string; color?: string }> = ({ label, value, color }) => (
  <div style={{
    flex: 1, minWidth: 110, padding: '12px 14px', borderRadius: 10,
    background: 'var(--wks-bg-surface)', border: '1px solid var(--wks-border-subtle)',
  }}>
    <div style={{ fontSize: '1.5rem', fontWeight: 700, color: color || 'var(--wks-text-primary)', fontVariantNumeric: 'tabular-nums' }}>{value}</div>
    <div style={{ fontSize: '0.6rem', color: 'var(--wks-text-faint)', textTransform: 'uppercase', letterSpacing: '0.05em', marginTop: 2 }}>{label}</div>
  </div>
);

const DirRow: React.FC<{ dir: string; fav: boolean; onSpawn: () => void; onToggleFav: () => void }> = ({ dir, fav, onSpawn, onToggleFav }) => (
  <div
    onClick={onSpawn}
    title={`Launch an agent in ${dir}`}
    style={{
      display: 'flex', alignItems: 'center', gap: 10, padding: '7px 10px', borderRadius: 7,
      cursor: 'pointer',
    }}
    onMouseEnter={(e) => { (e.currentTarget as HTMLElement).style.background = 'var(--wks-bg-selected)'; }}
    onMouseLeave={(e) => { (e.currentTarget as HTMLElement).style.background = 'transparent'; }}
  >
    <span
      onClick={(e) => { e.stopPropagation(); onToggleFav(); }}
      title={fav ? 'Unfavourite' : 'Favourite'}
      style={{ display: 'flex', alignItems: 'center', flexShrink: 0, cursor: 'pointer', color: fav ? 'var(--wks-warning, #e0a000)' : 'var(--wks-text-faint)' }}
    ><Star size={14} strokeWidth={1.75} fill={fav ? 'currentColor' : 'none'} /></span>
    <span style={{ fontSize: '0.8rem', fontWeight: 600, color: 'var(--wks-text-primary)', flexShrink: 0 }}>{basename(dir)}</span>
    <span style={{ fontSize: '0.62rem', color: 'var(--wks-text-faint)', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{dir}</span>
    <span style={{ display: 'flex', alignItems: 'center', gap: 3, marginLeft: 'auto', flexShrink: 0, fontSize: '0.62rem', color: 'var(--wks-accent)' }}><Plus size={11} strokeWidth={2.2} /> launch</span>
  </div>
);

const OverviewPane: React.FC<{ title?: string; agents?: { sessionId?: string }[] }> = ({ agents: workspaceAgents = [] }) => {
  const { config, save } = useConfig();
  const [snaps, setSnaps] = useState<Snap[]>([]);

  const refresh = useCallback(() => {
    window.electronAPI.getAllClaudeSessions?.()
      .then((s: any[]) => setSnaps(Array.isArray(s) ? s : []))
      .catch(() => {});
  }, []);
  useEffect(() => {
    refresh();
    const off = window.electronAPI.onClaudeSessionUpdate?.(() => refresh());
    return () => off?.();
  }, [refresh]);

  const recent = config.directories?.recent ?? [];
  const favourites = config.directories?.favourites ?? [];
  const favSet = new Set(favourites);
  const recentOnly = recent.filter((d) => !favSet.has(d));

  // Scope stats to workspacer's OWN agents — claudemon tracks every Claude
  // session on the machine (incl. ones run outside workspacer), so we filter
  // the snapshots down to the sessions our agents own.
  const ownSessionIds = new Set(workspaceAgents.map((a) => a.sessionId).filter(Boolean) as string[]);
  const own = snaps.filter((s) => ownSessionIds.has(s.sessionId));

  const agents = workspaceAgents.length;
  const working = own.filter((s) => s.ambientState === 'thinking' || s.ambientState === 'streaming').length;
  const needsYou = own.filter((s) => s.ambientState === 'waiting_approval' || s.ambientState === 'waiting_input').length;
  const totalCost = own.reduce((n, s) => n + (s.usage?.costUSD ?? 0), 0);

  const spawnIn = (cwd: string) => {
    window.electronAPI.hubPublish?.({ type: 'command.spawn_agent', source: 'workspacer.overview', data: { cwd } });
  };
  const browse = async () => {
    const dir = await window.electronAPI.pickFolder?.();
    if (dir) spawnIn(dir);
  };
  const toggleFav = (dir: string) => {
    const set = new Set(favourites);
    if (set.has(dir)) set.delete(dir); else set.add(dir);
    save({ directories: { recent, favourites: Array.from(set) } });
  };

  return (
    <div style={{ height: '100%', overflow: 'auto', background: 'var(--wks-bg-base)', color: 'var(--wks-text-primary)', padding: 18 }}>
      <div style={{ display: 'flex', alignItems: 'center', gap: 10, marginBottom: 16 }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 8, fontSize: '1.05rem', fontWeight: 700 }}>
          <Home size={18} strokeWidth={1.9} /> Workspace
        </div>
        <button
          onClick={browse}
          style={{
            marginLeft: 'auto', fontSize: '0.72rem', fontFamily: 'inherit', cursor: 'pointer',
            background: 'var(--wks-accent)', color: 'var(--wks-text-on-accent, #fff)',
            border: 'none', borderRadius: 6, padding: '6px 12px', fontWeight: 600,
          }}
        >＋ New agent…</button>
      </div>

      <div style={{ display: 'flex', gap: 10, flexWrap: 'wrap', marginBottom: 22 }}>
        <Stat label="Agents" value={String(agents)} />
        <Stat label="Working" value={String(working)} color={working ? 'var(--wks-accent, #4a9eff)' : undefined} />
        <Stat label="Need you" value={String(needsYou)} color={needsYou ? 'var(--wks-warning, #e0a000)' : undefined} />
        <Stat label="Total cost" value={fmtUSD(totalCost)} />
      </div>

      {favourites.length > 0 && (
        <Section title="Favourites">
          {favourites.map((d) => (
            <DirRow key={d} dir={d} fav onSpawn={() => spawnIn(d)} onToggleFav={() => toggleFav(d)} />
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
            <DirRow key={d} dir={d} fav={false} onSpawn={() => spawnIn(d)} onToggleFav={() => toggleFav(d)} />
          ))
        )}
      </Section>
    </div>
  );
};

const Section: React.FC<{ title: string; children: React.ReactNode }> = ({ title, children }) => (
  <div style={{ marginBottom: 18 }}>
    <div style={{ fontSize: '0.58rem', color: 'var(--wks-text-disabled)', fontWeight: 600, textTransform: 'uppercase', letterSpacing: '0.05em', padding: '0 10px 4px' }}>{title}</div>
    {children}
  </div>
);

export default OverviewPane;
