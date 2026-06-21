import React, { useEffect, useMemo, useRef, useState } from 'react';
import { X } from 'lucide-react';
import { useVirtualizer } from '@tanstack/react-virtual';
import { useAttention } from '../contexts/AttentionContext';
import { AgentCard } from './AgentCard';
import { agentAttentionScore } from '../lib/attentionRouter';

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

  // Agent → most-urgent open item, shared with the SideBar via the attention
  // feed (topByAgent) so both surfaces buoy cards by the same rule.
  const sorted = useMemo(() => {
    return [...realAgents].sort((a, b) => {
      const sa = agentAttentionScore(a.sessionId ? snapshotBySession[a.sessionId]?.ambientState : undefined, topByAgent.get(a.id)?.priority ?? 0);
      const sb = agentAttentionScore(b.sessionId ? snapshotBySession[b.sessionId]?.ambientState : undefined, topByAgent.get(b.id)?.priority ?? 0);
      return sb - sa;
    });
  }, [realAgents, snapshotBySession, topByAgent]);

  const working = realAgents.filter((a) => {
    const s = a.sessionId ? snapshotBySession[a.sessionId]?.ambientState : undefined;
    return s === 'thinking' || s === 'streaming';
  }).length;

  // j/k card selection (needy-first order == `sorted`), with y/n/1-9 acting on the
  // selected agent's top attention item — kept entirely within the deck.
  const [selectedId, setSelectedId] = useState<string | null>(null);
  // Keep selection valid as the fleet re-sorts / agents come and go.
  useEffect(() => {
    if (sorted.length === 0) { if (selectedId !== null) setSelectedId(null); return; }
    if (!selectedId || !sorted.some((a) => a.id === selectedId)) setSelectedId(sorted[0].id);
  }, [sorted, selectedId]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const t = e.target as HTMLElement | null;
      if (t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.isContentEditable)) return;
      if (sorted.length === 0) return;
      const stop = () => { e.preventDefault(); e.stopPropagation(); };
      const idx = selectedId ? sorted.findIndex((a) => a.id === selectedId) : -1;

      if (e.key === 'j' || e.key === 'ArrowDown') { stop(); const n = Math.min(sorted.length - 1, (idx < 0 ? -1 : idx) + 1); setSelectedId(sorted[n].id); return; }
      if (e.key === 'k' || e.key === 'ArrowUp') { stop(); const n = Math.max(0, (idx < 0 ? 0 : idx) - 1); setSelectedId(sorted[n].id); return; }

      if (idx < 0) return;
      const top = topByAgent.get(sorted[idx].id);
      if (!top) {
        if (e.key === 'Enter') { stop(); openAgent(sorted[idx].id); }
        return;
      }
      if (e.key === 'Enter') { stop(); openAgent(top.agentId); return; }
      if (top.payload.type === 'approval') {
        if (e.key === 'y') { stop(); approve(top, 'yes'); return; }
        if (e.key === 'n') { stop(); approve(top, 'no'); return; }
      }
      if (top.payload.type === 'question') {
        const n = parseInt(e.key, 10);
        if (n >= 1 && n <= 9 && n <= (top.payload.questions[0]?.options.length ?? 0)) { stop(); answer(top, { option: n }); return; }
      }
    };
    window.addEventListener('keydown', onKey, true);
    return () => window.removeEventListener('keydown', onKey, true);
  }, [sorted, selectedId, topByAgent, approve, answer, openAgent]);

  // In list mode, keep the j/k-selected row visible as it moves.
  useEffect(() => {
    if (fleetView !== 'list' || !selectedId) return;
    listScrollRef.current?.querySelector(`[data-fleet-row="${selectedId}"]`)?.scrollIntoView({ block: 'nearest' });
  }, [selectedId, fleetView]);

  // Windowed grid: track the content width so we can pack cards into rows, then
  // virtualize the rows — only on-screen cards (plus overscan) are in the DOM,
  // so a 50+-agent fleet stays smooth.
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
  const rowVirtualizer = useVirtualizer({
    count: Math.ceil(sorted.length / cols),
    getScrollElement: () => scrollRef.current,
    estimateSize: () => 248, // ~230 card min-height + row gap
    overscan: 2,
  });

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
        <span style={{ fontSize: '0.62rem', color: 'var(--wks-text-faint)', display: 'inline-flex', gap: 6 }}>
          <span><kbd style={kbdStyle}>j</kbd>/<kbd style={kbdStyle}>k</kbd> move</span>
          <span><kbd style={kbdStyle}>y</kbd>/<kbd style={kbdStyle}>n</kbd> approve</span>
          <span><kbd style={kbdStyle}>1-9</kbd> answer</span>
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
                <th style={lth}>Agent</th>
                <th style={lth}>Status</th>
                <th style={lth}>Model</th>
                <th style={lthNum}>Context</th>
                <th style={lthNum}>Cost</th>
              </tr>
            </thead>
            <tbody>
              {sorted.map((agent) => {
                const snap = agent.sessionId ? snapshotBySession[agent.sessionId] : undefined;
                const vis = listStateVisual(agent.sessionId ? snap?.ambientState : undefined);
                const usage = snap?.usage;
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
                      <span style={{ fontSize: '0.62rem', fontWeight: 600, color: vis.color, border: `1px solid ${vis.color}`, borderRadius: 99, padding: '1px 9px', whiteSpace: 'nowrap' }}>{vis.label}</span>
                    </td>
                    <td style={{ ...ltd, color: 'var(--wks-text-secondary)' }}>{usage?.model ? usage.model.replace(/^claude-/, '') : '—'}</td>
                    <td style={ltdNum}>{usage && usage.contextTokens > 0 ? fmtTokens(usage.contextTokens) : '—'}</td>
                    <td style={{ ...ltdNum, color: 'var(--wks-accent)' }}>{usage ? fmtUSD(usage.costUSD) : '—'}</td>
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
              const rowAgents = sorted.slice(start, start + cols);
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

function fmtTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${Math.round(n / 1_000)}k`;
  return `${n}`;
}
function fmtUSD(n: number): string {
  return n >= 0.01 ? `$${n.toFixed(2)}` : n > 0 ? '<$0.01' : '$0.00';
}

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
