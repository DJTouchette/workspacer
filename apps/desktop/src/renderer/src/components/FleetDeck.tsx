import React, { useEffect, useMemo, useRef, useState } from 'react';
import { X } from 'lucide-react';
import { useVirtualizer } from '@tanstack/react-virtual';
import { useAttention } from '../contexts/AttentionContext';
import { AgentCard } from './AgentCard';
import { StatusGlyph } from './statusGlyph';
import { shortModelLabel } from '../lib/modelLabel';
import { agentAttentionScore } from '../lib/attentionRouter';
import { deriveSessionStats, fmtUSD, ctxColor, isSnapshotStale } from '../lib/sessionStats';
import { useConfig } from '../hooks/useConfig';
import { DEFAULT_SHORTCUTS } from '../hooks/configDefaults';
import { eventMatchesCombo, digitFromRangeEvent, formatBinding } from '../lib/shortcuts';

const CARD_MIN = 360; // matches the old minmax(360px) grid
const GRID_GAP = 18;
const GRID_PAD_X = 22; // horizontal padding each side of the scroll area

const STYLE_ID = 'fleet-deck-keyframes';
function ensureFleetKeyframes() {
  if (typeof document === 'undefined' || document.getElementById(STYLE_ID)) return;
  const s = document.createElement('style');
  s.id = STYLE_ID;
  s.textContent = '@keyframes fleetPulse { 0%,100% { box-shadow: 0 0 0 1px currentColor; } 50% { box-shadow: 0 0 0 3px currentColor, 0 0 18px currentColor; } }';
  document.head.appendChild(s);
}

/** Compact relative time for the list's "Active" column. */
function relTime(ts: number | undefined): string {
  if (!ts) return '—';
  const s = Math.max(0, Math.round((Date.now() - ts) / 1000));
  if (s < 60) return `${s}s`;
  const m = Math.round(s / 60);
  if (m < 60) return `${m}m`;
  const h = Math.round(m / 60);
  if (h < 24) return `${h}h`;
  return `${Math.round(h / 24)}d`;
}

interface Props {
  /** Inset so the deck sits inside the content area (right of sidebar, below navbar). */
  top: number;
  left: number;
}

/**
 * The Fleet Deck — a NORAD-style cross-agent radar. Every agent is a live
 * telemetry-face card, arranged by the Attention Router so the ones that need
 * you float to the front and pulse. Rendered as an overlay OVER the still-
 * mounted per-agent workspaces, so entering/leaving the deck never remounts a
 * pane: the agents keep running underneath, and clicking a card simply reveals
 * the one you picked (setActiveAgentId + viewLevel='piloting').
 */
const FleetDeck: React.FC<Props> = ({ top, left }) => {
  ensureFleetKeyframes();
  const {
    agents, snapshotBySession, counts, setViewLevel, topByAgent,
    spawnAgent, approve, answer, openAgent,
  } = useAttention();

  const realAgents = useMemo(() => agents.filter((a) => !a.global), [agents]);

  // Deck-scoped keybindings (fleet-*), remappable in Settings → Keybindings.
  // Defaults merged under user overrides so a partial saved map still binds.
  const { config } = useConfig();
  const sc = useMemo(
    () => ({ ...DEFAULT_SHORTCUTS, ...(config.keybindings?.shortcuts ?? {}) }),
    [config.keybindings?.shortcuts],
  );

  // Cards (default) vs a dense List table — mirrors the mockup's Fleet toggle.
  // Persisted so the deck reopens in the layout you last used.
  const [fleetView, setFleetView] = useState<'cards' | 'list'>(() => {
    try { return localStorage.getItem('wks-fleet-view') === 'list' ? 'list' : 'cards'; } catch { return 'cards'; }
  });
  const pickView = (v: 'cards' | 'list') => {
    setFleetView(v);
    try { localStorage.setItem('wks-fleet-view', v); } catch { /* private mode */ }
  };
  const listScrollRef = useRef<HTMLDivElement>(null);

  // Type-to-filter by name or provider. Applied before sort, so cards, list, and
  // keyboard nav all operate on the filtered set; header counts stay whole-fleet.
  const [query, setQuery] = useState('');

  // Staleness needs a clock even when no snapshots arrive (that IS the stale
  // case) — a slow tick re-evaluates the list rows' warning tint.
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const t = setInterval(() => setNow(Date.now()), 30_000);
    return () => clearInterval(t);
  }, []);

  // Agent → most-urgent open item, shared with the SideBar via the attention
  // feed (topByAgent) so both surfaces buoy cards by the same rule.
  const sorted = useMemo(() => {
    const q = query.trim().toLowerCase();
    const matches = (a: typeof realAgents[number]) =>
      !q || a.name.toLowerCase().includes(q) || (a.provider ?? 'claude').toLowerCase().includes(q);
    return realAgents.filter(matches).sort((a, b) => {
      const sa = agentAttentionScore(a.sessionId ? snapshotBySession[a.sessionId]?.ambientState : undefined, topByAgent.get(a.id)?.priority ?? 0);
      const sb = agentAttentionScore(b.sessionId ? snapshotBySession[b.sessionId]?.ambientState : undefined, topByAgent.get(b.id)?.priority ?? 0);
      return sb - sa;
    });
  }, [realAgents, snapshotBySession, topByAgent, query]);

  // List-view column sort. 'attn' keeps the needy-first order (the default); the
  // other keys sort by live stats. Cards always use the attention order.
  const [listSort, setListSort] = useState<{ key: 'attn' | 'name' | 'ctx' | 'cost' | 'act'; dir: 1 | -1 }>({ key: 'attn', dir: -1 });
  const toggleSort = (key: typeof listSort.key) =>
    setListSort((s) => (s.key === key ? { key, dir: (s.dir === 1 ? -1 : 1) } : { key, dir: key === 'name' ? 1 : -1 }));
  const listRows = useMemo(() => {
    if (listSort.key === 'attn') return sorted;
    const keyOf = (a: typeof sorted[number]): number | string => {
      const snap = a.sessionId ? snapshotBySession[a.sessionId] : undefined;
      const st = deriveSessionStats(snap);
      switch (listSort.key) {
        case 'name': return a.name.toLowerCase();
        case 'ctx': return st.ctxPct ?? -1;
        case 'cost': return st.costUSD ?? -1;
        case 'act': return snap?.lastActivity ?? 0;
        default: return 0;
      }
    };
    return [...sorted].sort((a, b) => {
      const ka = keyOf(a);
      const kb = keyOf(b);
      const c = typeof ka === 'string' ? ka.localeCompare(kb as string) : (ka as number) - (kb as number);
      return c * listSort.dir;
    });
  }, [sorted, listSort, snapshotBySession]);
  // The order the user is actually looking at — keyboard nav + selection follow it.
  const displayOrder = fleetView === 'list' ? listRows : sorted;

  // Clickable, sortable list-column header. Click toggles direction; the active
  // column shows a caret.
  const SortBtn: React.FC<{ k: typeof listSort.key; label: string }> = ({ k, label }) => {
    const active = listSort.key === k;
    return (
      <button
        onClick={(e) => { e.stopPropagation(); toggleSort(k); }}
        style={{
          background: 'none', border: 'none', padding: 0, font: 'inherit', cursor: 'pointer',
          color: active ? 'var(--wks-text-secondary)' : 'inherit', fontWeight: active ? 700 : 'inherit',
        }}
      >
        {label}{active ? (listSort.dir === 1 ? ' ▲' : ' ▼') : ''}
      </button>
    );
  };

  const working = realAgents.filter((a) => {
    const s = a.sessionId ? snapshotBySession[a.sessionId]?.ambientState : undefined;
    return s === 'thinking' || s === 'streaming';
  }).length;

  // Windowed grid measurement (also feeds keyboard grid-nav): track the content
  // width so we can pack cards into rows of `cols` and move selection by row.
  const scrollRef = useRef<HTMLDivElement>(null);
  const [avail, setAvail] = useState(0);
  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    const measure = () => setAvail(Math.max(0, el.clientWidth - GRID_PAD_X * 2));
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);
  const cols = Math.max(1, Math.floor((avail + GRID_GAP) / (CARD_MIN + GRID_GAP)));

  // Card selection (needy-first order == `displayOrder`), with approve/answer
  // acting on the selected agent's top attention item — kept entirely within
  // the deck.
  const [selectedId, setSelectedId] = useState<string | null>(null);
  // Keep selection valid as the fleet re-sorts / agents come and go.
  useEffect(() => {
    if (displayOrder.length === 0) { if (selectedId !== null) setSelectedId(null); return; }
    if (!selectedId || !displayOrder.some((a) => a.id === selectedId)) setSelectedId(displayOrder[0].id);
  }, [displayOrder, selectedId]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const t = e.target as HTMLElement | null;
      if (t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.isContentEditable)) return;
      if (displayOrder.length === 0) return;
      const stop = () => { e.preventDefault(); e.stopPropagation(); };
      const idx = selectedId ? displayOrder.findIndex((a) => a.id === selectedId) : -1;

      // Movement adapts to the active fleet view: the Cards grid navigates
      // spatially (down = the card BELOW, one row of `cols` later), the List
      // linearly. Each view has its own bindings; arrows are fixed fallbacks.
      const select = (n: number) => setSelectedId(displayOrder[Math.max(0, Math.min(displayOrder.length - 1, n))].id);
      if (fleetView === 'cards') {
        if (idx < 0) {
          // Nothing selected yet: any movement key just lands on the first card.
          if (['ArrowDown', 'ArrowUp', 'ArrowLeft', 'ArrowRight'].includes(e.key) ||
              eventMatchesCombo(e, sc['fleet-cards-down']) || eventMatchesCombo(e, sc['fleet-cards-up']) ||
              eventMatchesCombo(e, sc['fleet-cards-left']) || eventMatchesCombo(e, sc['fleet-cards-right'])) { stop(); select(0); return; }
        } else {
          // Row moves are true vertical moves: below the grid they clamp into
          // the (possibly partial) last row; at the edge rows they no-op rather
          // than sliding sideways.
          const lastRowStart = Math.floor((displayOrder.length - 1) / cols) * cols;
          if (eventMatchesCombo(e, sc['fleet-cards-down']) || e.key === 'ArrowDown') { stop(); if (idx < lastRowStart) select(idx + cols); return; }
          if (eventMatchesCombo(e, sc['fleet-cards-up']) || e.key === 'ArrowUp') { stop(); if (idx >= cols) select(idx - cols); return; }
          if (eventMatchesCombo(e, sc['fleet-cards-left']) || e.key === 'ArrowLeft') { stop(); select(idx - 1); return; }
          if (eventMatchesCombo(e, sc['fleet-cards-right']) || e.key === 'ArrowRight') { stop(); select(idx + 1); return; }
        }
      } else {
        if (eventMatchesCombo(e, sc['fleet-list-down']) || e.key === 'ArrowDown') { stop(); select((idx < 0 ? -1 : idx) + 1); return; }
        if (eventMatchesCombo(e, sc['fleet-list-up']) || e.key === 'ArrowUp') { stop(); select((idx < 0 ? 1 : idx) - 1); return; }
      }

      if (idx < 0) return;
      const top = topByAgent.get(displayOrder[idx].id);
      if (!top) {
        if (eventMatchesCombo(e, sc['fleet-open'])) { stop(); openAgent(displayOrder[idx].id); }
        return;
      }
      if (eventMatchesCombo(e, sc['fleet-open'])) { stop(); openAgent(top.agentId); return; }
      if (top.payload.type === 'approval') {
        if (eventMatchesCombo(e, sc['fleet-approve-yes'])) { stop(); approve(top, 'yes'); return; }
        if (eventMatchesCombo(e, sc['fleet-approve-no'])) { stop(); approve(top, 'no'); return; }
      }
      if (top.payload.type === 'question') {
        const n = digitFromRangeEvent(e, sc['fleet-answer']);
        if (n !== null && n <= (top.payload.questions[0]?.options.length ?? 0)) { stop(); answer(top, { option: n }); return; }
      }
    };
    window.addEventListener('keydown', onKey, true);
    return () => window.removeEventListener('keydown', onKey, true);
  }, [displayOrder, selectedId, topByAgent, approve, answer, openAgent, sc, fleetView, cols]);

  // In list mode, keep the j/k-selected row visible as it moves.
  useEffect(() => {
    if (fleetView !== 'list' || !selectedId) return;
    listScrollRef.current?.querySelector(`[data-fleet-row="${selectedId}"]`)?.scrollIntoView({ block: 'nearest' });
  }, [selectedId, fleetView]);

  // Windowed grid: virtualize the packed rows — only on-screen cards (plus
  // overscan) are in the DOM, so a 50+-agent fleet stays smooth.
  const rowVirtualizer = useVirtualizer({
    count: Math.ceil(displayOrder.length / cols),
    getScrollElement: () => scrollRef.current,
    estimateSize: () => 290, // ~260 card min-height (chips + files line) + row gap; rows self-measure
    overscan: 2,
  });

  // In card mode, keep the j/k-selected card visible by scrolling its row into
  // view (the list mode has its own scroll effect above).
  useEffect(() => {
    if (fleetView !== 'cards' || !selectedId) return;
    const i = displayOrder.findIndex((a) => a.id === selectedId);
    if (i >= 0) rowVirtualizer.scrollToIndex(Math.floor(i / cols), { align: 'auto' });
  }, [selectedId, fleetView, displayOrder, cols, rowVirtualizer]);

  return (
    <div style={{ position: 'fixed', top, left, right: 0, bottom: 0, zIndex: 150, background: 'var(--wks-bg-base)', display: 'flex', flexDirection: 'column' }}>
      {/* Deck header */}
      <div style={{ display: 'flex', alignItems: 'center', gap: 12, padding: '14px 22px 10px' }}>
        <div style={{ display: 'flex', alignItems: 'baseline', gap: 11, minWidth: 0 }}>
          <div style={{ fontSize: '1.1rem', fontWeight: 800, letterSpacing: '-0.01em', color: 'var(--wks-text-primary)' }}>Fleet</div>
          <div style={{ fontSize: '0.72rem', color: 'var(--wks-text-secondary)', fontVariantNumeric: 'tabular-nums', whiteSpace: 'nowrap' }}>
            {realAgents.length} agent{realAgents.length === 1 ? '' : 's'} · {working} working · {counts.needsYou} need{counts.needsYou === 1 ? 's' : ''} you
          </div>
        </div>
        <div style={{ flex: 1 }} />
        <input
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="Filter agents…"
          spellCheck={false}
          style={{
            width: 160, fontSize: '0.72rem', fontFamily: 'inherit', padding: '5px 9px',
            borderRadius: 8, border: '1px solid var(--wks-border-subtle)',
            background: 'var(--wks-bg-surface)', color: 'var(--wks-text-primary)',
          }}
        />
        <span style={{ fontSize: '0.62rem', color: 'var(--wks-text-faint)', display: 'inline-flex', gap: 6 }}>
          {fleetView === 'cards' ? (
            <span>
              <kbd style={kbdStyle}>{formatBinding(sc['fleet-cards-left'] ?? '')}</kbd>/<kbd style={kbdStyle}>{formatBinding(sc['fleet-cards-down'] ?? '')}</kbd>/<kbd style={kbdStyle}>{formatBinding(sc['fleet-cards-up'] ?? '')}</kbd>/<kbd style={kbdStyle}>{formatBinding(sc['fleet-cards-right'] ?? '')}</kbd> move
            </span>
          ) : (
            <span><kbd style={kbdStyle}>{formatBinding(sc['fleet-list-down'] ?? '')}</kbd>/<kbd style={kbdStyle}>{formatBinding(sc['fleet-list-up'] ?? '')}</kbd> move</span>
          )}
          <span><kbd style={kbdStyle}>{formatBinding(sc['fleet-approve-yes'] ?? '')}</kbd>/<kbd style={kbdStyle}>{formatBinding(sc['fleet-approve-no'] ?? '')}</kbd> approve</span>
          <span><kbd style={kbdStyle}>{formatBinding(sc['fleet-answer'] ?? '')}</kbd> answer</span>
        </span>
        {/* Cards / List toggle */}
        <div style={{ display: 'flex', background: 'var(--wks-bg-surface)', border: '1px solid var(--wks-border-subtle)', borderRadius: 8, padding: 3 }}>
          <SegBtn active={fleetView === 'cards'} onClick={() => pickView('cards')}>Cards</SegBtn>
          <SegBtn active={fleetView === 'list'} onClick={() => pickView('list')}>List</SegBtn>
        </div>
        <button onClick={spawnAgent} title="Spawn a new agent" style={{
          display: 'inline-flex', alignItems: 'center', gap: 6,
          fontSize: '0.72rem', fontFamily: 'inherit', fontWeight: 700, cursor: 'pointer',
          border: 'none', borderRadius: 8, padding: '6px 13px',
          background: 'var(--wks-accent)', color: 'var(--wks-text-on-accent, #fff)',
        }}>
          <span style={{ fontSize: '0.95rem', lineHeight: 1 }}>+</span> Spawn agent
        </button>
        <button onClick={() => setViewLevel('piloting')} title="Back to agent (Esc)" style={{
          display: 'inline-flex', alignItems: 'center', gap: 6,
          fontSize: '0.72rem', fontFamily: 'inherit', fontWeight: 600, cursor: 'pointer',
          border: '1px solid var(--wks-glass-border)', borderRadius: 6, padding: '5px 12px',
          background: 'var(--wks-bg-surface)', color: 'var(--wks-text-secondary)',
        }}>
          <X size={13} strokeWidth={2} /> Exit fleet <kbd style={kbdStyle}>Esc</kbd>
        </button>
      </div>

      {/* Content: empty state · dense list · windowed card grid */}
      {realAgents.length === 0 ? (
        <div style={CONTENT_SCROLL}>
          <div style={{ marginTop: 80, textAlign: 'center', color: 'var(--wks-text-faint)' }}>
            <div style={{ fontSize: '0.9rem', fontWeight: 600, color: 'var(--wks-text-secondary)' }}>No agents in the fleet</div>
            <div style={{ fontSize: '0.78rem', marginTop: 6 }}>Spawn an agent and it'll appear here as a live card.</div>
            <button
              onClick={spawnAgent}
              style={{
                marginTop: 16, fontSize: '0.8rem', fontFamily: 'inherit', fontWeight: 700, cursor: 'pointer',
                background: 'var(--wks-accent)', color: 'var(--wks-text-on-accent, #fff)',
                border: 'none', borderRadius: 6, padding: '8px 16px',
              }}
            >
              + Spawn agent
            </button>
          </div>
        </div>
      ) : fleetView === 'list' ? (
        <div ref={listScrollRef} style={CONTENT_SCROLL}>
          <table style={{ width: '100%', borderCollapse: 'collapse', fontSize: '0.74rem' }}>
            <thead>
              <tr style={{ color: 'var(--wks-text-faint)', textAlign: 'left' }}>
                <th style={lth}><SortBtn k="name" label="Agent" /></th>
                <th style={lth}>Status</th>
                <th style={lth}>Model</th>
                <th style={lthNum}><SortBtn k="ctx" label="Context" /></th>
                <th style={lthNum}><SortBtn k="cost" label="Cost" /></th>
                <th style={lthNum}><SortBtn k="act" label="Active" /></th>
              </tr>
            </thead>
            <tbody>
              {displayOrder.map((agent) => {
                const snap = agent.sessionId ? snapshotBySession[agent.sessionId] : undefined;
                const vis = listStateVisual(agent.sessionId ? snap?.ambientState : undefined);
                const stats = deriveSessionStats(snap);
                const sel = selectedId === agent.id;
                return (
                  <tr
                    key={agent.id}
                    data-fleet-row={agent.id}
                    onMouseDown={() => setSelectedId(agent.id)}
                    onClick={() => openAgent(agent.id)}
                    title={`${agent.name} — ${vis.label}\n${agent.cwd ?? ''}`}
                    style={{ cursor: 'pointer', borderTop: '1px solid var(--wks-border-subtle)', background: sel ? 'var(--wks-bg-selected)' : 'transparent' }}
                    onMouseEnter={(e) => { if (!sel) (e.currentTarget as HTMLElement).style.background = 'var(--wks-bg-hover)'; }}
                    onMouseLeave={(e) => { if (!sel) (e.currentTarget as HTMLElement).style.background = 'transparent'; }}
                  >
                    <td style={ltd}>
                      <span style={{ display: 'inline-flex', alignItems: 'center', gap: 8, minWidth: 0 }}>
                        <span style={{ width: 8, height: 8, borderRadius: '50%', flexShrink: 0, background: vis.color, boxShadow: vis.glow ? `0 0 8px ${vis.color}` : 'none' }} />
                        <span style={{ fontWeight: 600, color: 'var(--wks-text-primary)', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                          {agent.kind === 'supervisor' ? '🧭 ' : ''}{agent.name}
                        </span>
                      </span>
                    </td>
                    <td style={ltd}>
                      <span style={{ display: 'inline-flex', alignItems: 'center', gap: 4, fontSize: '0.62rem', fontWeight: 600, color: vis.color, border: `1px solid ${vis.color}`, borderRadius: 99, padding: '1px 9px', whiteSpace: 'nowrap' }}>
                        <StatusGlyph state={agent.sessionId ? snap?.ambientState : undefined} size={12} strokeWidth={2.2} accent="currentColor" />
                        {vis.label}
                      </span>
                    </td>
                    <td style={{ ...ltd, color: 'var(--wks-text-secondary)' }}>{stats.model ? shortModelLabel(stats.model) : '—'}</td>
                    <td style={ltdNum}>{stats.ctxPct !== undefined ? <span style={{ color: ctxColor(stats.ctxPct), fontWeight: 600 }}>{Math.round(stats.ctxPct)}%</span> : '—'}</td>
                    <td style={{ ...ltdNum, color: 'var(--wks-accent)' }}>{stats.costUSD !== undefined ? fmtUSD(stats.costUSD) : '—'}</td>
                    {isSnapshotStale(snap?.ambientState, snap?.lastActivity, now) ? (
                      <td style={{ ...ltdNum, color: 'var(--wks-warning, #e0a000)', fontWeight: 700 }} title="Says it's working but nothing has arrived — the stream may have stalled.">
                        ⚠ {relTime(snap?.lastActivity)}
                      </td>
                    ) : (
                      <td style={ltdNum}>{relTime(snap?.lastActivity)}</td>
                    )}
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      ) : (
        <div ref={scrollRef} style={CONTENT_SCROLL}>
          <div style={{ position: 'relative', width: '100%', height: rowVirtualizer.getTotalSize() }}>
            {rowVirtualizer.getVirtualItems().map((vr) => {
              const start = vr.index * cols;
              const rowAgents = displayOrder.slice(start, start + cols);
              return (
                <div
                  key={vr.key}
                  data-index={vr.index}
                  ref={rowVirtualizer.measureElement}
                  style={{
                    position: 'absolute', top: 0, left: 0, width: '100%',
                    transform: `translateY(${vr.start}px)`,
                    display: 'grid', gridTemplateColumns: `repeat(${cols}, minmax(0, 1fr))`,
                    gap: GRID_GAP, paddingBottom: GRID_GAP, alignItems: 'start',
                  }}
                >
                  {rowAgents.map((agent) => (
                    <div
                      key={agent.id}
                      onMouseDown={() => setSelectedId(agent.id)}
                      style={{
                        borderRadius: 'var(--wks-radius-lg, 12px)',
                        outline: selectedId === agent.id ? '2px solid var(--wks-accent, #4a9eff)' : '2px solid transparent',
                        outlineOffset: 2,
                        transition: 'outline-color 0.12s',
                      }}
                    >
                      <AgentCard agent={agent} snapshot={agent.sessionId ? snapshotBySession[agent.sessionId] : undefined} />
                    </div>
                  ))}
                </div>
              );
            })}
          </div>
        </div>
      )}
    </div>
  );
};

const kbdStyle: React.CSSProperties = {
  fontSize: '0.58rem', color: 'var(--wks-text-secondary)', border: '1px solid var(--wks-glass-border)',
  borderRadius: 3, padding: '0 3px', fontFamily: 'monospace',
};

const CONTENT_SCROLL: React.CSSProperties = { flex: 1, overflowY: 'auto', padding: '6px 22px 28px' };

/** Segmented Cards/List toggle button (mockup style). */
const SegBtn: React.FC<{ active: boolean; onClick: () => void; children: React.ReactNode }> = ({ active, onClick, children }) => (
  <button
    onClick={onClick}
    style={{
      border: 'none', borderRadius: 6, padding: '5px 13px', cursor: 'pointer',
      fontFamily: 'inherit', fontSize: '0.72rem', fontWeight: 600,
      background: active ? 'var(--wks-bg-selected)' : 'transparent',
      color: active ? 'var(--wks-text-primary)' : 'var(--wks-text-faint)',
    }}
  >{children}</button>
);


/** Status dot/pill colour + label for the list row. `busy` (thinking/streaming)
 *  uses the dedicated busy token so it matches the cards and sidebar. */
function listStateVisual(s: string | undefined): { color: string; label: string; glow: boolean } {
  switch (s) {
    case 'waiting_approval': return { color: 'var(--wks-warning, #e0a000)', label: 'Needs approval', glow: true };
    case 'waiting_input':    return { color: 'var(--wks-warning, #e0a000)', label: 'Waiting', glow: true };
    case 'thinking':         return { color: 'var(--wks-busy, var(--wks-accent, #4a9eff))', label: 'Thinking', glow: true };
    case 'streaming':        return { color: 'var(--wks-busy, var(--wks-accent, #4a9eff))', label: 'Working', glow: true };
    case 'idle':             return { color: 'var(--wks-success, #3fb950)', label: 'Idle', glow: false };
    default:                 return { color: 'var(--wks-text-faint, #666)', label: 'Stopped', glow: false };
  }
}

const lth: React.CSSProperties = { padding: '6px 10px', fontWeight: 600, textTransform: 'uppercase', letterSpacing: '0.05em', fontSize: '0.56rem' };
const lthNum: React.CSSProperties = { ...lth, textAlign: 'right' };
const ltd: React.CSSProperties = { padding: '8px 10px', whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis', maxWidth: 280 };
const ltdNum: React.CSSProperties = { ...ltd, textAlign: 'right', fontVariantNumeric: 'tabular-nums', color: 'var(--wks-text-secondary)' };

export default FleetDeck;
