import React, { useEffect, useMemo, useRef, useState } from 'react';
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
  const { agents, snapshotBySession, feed, counts, setViewLevel } = useAttention();

  const realAgents = useMemo(() => agents.filter((a) => !a.global), [agents]);

  // Agent → most-urgent open-item priority (drives buoyancy above bare state).
  const topPriorityByAgent = useMemo(() => {
    const m = new Map<string, number>();
    for (const it of feed) m.set(it.agentId, Math.max(m.get(it.agentId) ?? 0, it.priority));
    return m;
  }, [feed]);

  const sorted = useMemo(() => {
    return [...realAgents].sort((a, b) => {
      const sa = agentAttentionScore(a.sessionId ? snapshotBySession[a.sessionId]?.ambientState : undefined, topPriorityByAgent.get(a.id) ?? 0);
      const sb = agentAttentionScore(b.sessionId ? snapshotBySession[b.sessionId]?.ambientState : undefined, topPriorityByAgent.get(b.id) ?? 0);
      return sb - sa;
    });
  }, [realAgents, snapshotBySession, topPriorityByAgent]);

  const working = realAgents.filter((a) => {
    const s = a.sessionId ? snapshotBySession[a.sessionId]?.ambientState : undefined;
    return s === 'thinking' || s === 'streaming';
  }).length;

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
        <button onClick={() => setViewLevel('piloting')} title="Back to agent (Esc)" style={{
          fontSize: '0.72rem', fontFamily: 'inherit', fontWeight: 600, cursor: 'pointer',
          border: '1px solid var(--wks-glass-border)', borderRadius: 6, padding: '5px 12px',
          background: 'var(--wks-bg-surface)', color: 'var(--wks-text-secondary)',
        }}>Exit fleet ⏎</button>
      </div>

      {/* Card grid (windowed by row) */}
      <div ref={scrollRef} style={{ flex: 1, overflowY: 'auto', padding: '6px 22px 28px' }}>
        {realAgents.length === 0 ? (
          <div style={{ marginTop: 80, textAlign: 'center', color: 'var(--wks-text-faint)' }}>
            <div style={{ fontSize: '0.9rem', fontWeight: 600, color: 'var(--wks-text-secondary)' }}>No agents in the fleet</div>
            <div style={{ fontSize: '0.78rem', marginTop: 6 }}>Spawn an agent and it'll appear here as a live card.</div>
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
                    <AgentCard key={agent.id} agent={agent} snapshot={agent.sessionId ? snapshotBySession[agent.sessionId] : undefined} />
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

const Stat: React.FC<{ label: string; value: number; color?: string }> = ({ label, value, color }) => (
  <span style={{ display: 'inline-flex', alignItems: 'baseline', gap: 4 }}>
    <span style={{ fontWeight: 700, color: color || 'var(--wks-text-primary)', fontVariantNumeric: 'tabular-nums' }}>{value}</span>
    <span style={{ color: 'var(--wks-text-faint)' }}>{label}</span>
  </span>
);

export default FleetDeck;
