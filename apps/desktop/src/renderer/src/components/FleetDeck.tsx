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
      <div style={{ display: 'flex', alignItems: 'center', gap: 14, padding: '14px 22px 10px' }}>
        <div style={{ fontSize: '1.1rem', fontWeight: 800, letterSpacing: '-0.01em', color: 'var(--wks-text-primary)' }}>Fleet</div>
        <div style={{ display: 'flex', gap: 12, fontSize: '0.72rem' }}>
          <Stat label="agents" value={realAgents.length} />
          {counts.needsYou > 0 && <Stat label="need you" value={counts.needsYou} color="var(--wks-warning, #e0a000)" />}
          {working > 0 && <Stat label="working" value={working} color="var(--wks-accent, #4a9eff)" />}
        </div>
        <div style={{ flex: 1 }} />
        <span style={{ fontSize: '0.62rem', color: 'var(--wks-text-faint)', display: 'inline-flex', gap: 6 }}>
          <span><kbd style={kbdStyle}>j</kbd>/<kbd style={kbdStyle}>k</kbd> move</span>
          <span><kbd style={kbdStyle}>y</kbd>/<kbd style={kbdStyle}>n</kbd> approve</span>
          <span><kbd style={kbdStyle}>1-9</kbd> answer</span>
        </span>
        <button onClick={() => setViewLevel('piloting')} title="Back to agent (Esc)" style={{
          display: 'inline-flex', alignItems: 'center', gap: 6,
          fontSize: '0.72rem', fontFamily: 'inherit', fontWeight: 600, cursor: 'pointer',
          border: '1px solid var(--wks-glass-border)', borderRadius: 6, padding: '5px 12px',
          background: 'var(--wks-bg-surface)', color: 'var(--wks-text-secondary)',
        }}>
          <X size={13} strokeWidth={2} /> Exit fleet <kbd style={kbdStyle}>Esc</kbd>
        </button>
      </div>

      {/* Card grid (windowed by row) */}
      <div ref={scrollRef} style={{ flex: 1, overflowY: 'auto', padding: '6px 22px 28px' }}>
        {realAgents.length === 0 ? (
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
        ) : (
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
        )}
      </div>
    </div>
  );
};

const kbdStyle: React.CSSProperties = {
  fontSize: '0.58rem', color: 'var(--wks-text-secondary)', border: '1px solid var(--wks-glass-border)',
  borderRadius: 3, padding: '0 3px', fontFamily: 'monospace',
};

const Stat: React.FC<{ label: string; value: number; color?: string }> = ({ label, value, color }) => (
  <span style={{ display: 'inline-flex', alignItems: 'baseline', gap: 4 }}>
    <span style={{ fontWeight: 700, color: color || 'var(--wks-text-primary)', fontVariantNumeric: 'tabular-nums' }}>{value}</span>
    <span style={{ color: 'var(--wks-text-faint)' }}>{label}</span>
  </span>
);

export default FleetDeck;
