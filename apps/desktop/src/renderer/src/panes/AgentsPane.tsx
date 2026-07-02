import React, { useMemo, useState } from 'react';
import { useAttention } from '../contexts/AttentionContext';
import { AgentCard } from '../components/AgentCard';
import { agentAttentionScore } from '../lib/attentionRouter';
import { requestSessionWatch } from '../lib/watchBus';

// AgentCard's "blocked on you" pulse animation is normally injected by the
// FleetDeck; inject the same keyframes (same id → idempotent) so cards pulse
// here too even if the deck was never opened.
const STYLE_ID = 'fleet-deck-keyframes';
function ensureFleetKeyframes() {
  if (typeof document === 'undefined' || document.getElementById(STYLE_ID)) return;
  const s = document.createElement('style');
  s.id = STYLE_ID;
  s.textContent = '@keyframes fleetPulse { 0%,100% { box-shadow: 0 0 0 1px currentColor; } 50% { box-shadow: 0 0 0 3px currentColor, 0 0 18px currentColor; } }';
  document.head.appendChild(s);
}

/**
 * The Agents pane — the Fleet Deck's live cards as a regular PANE, so a
 * whole-fleet monitor can sit in a tab or split beside your work. Every agent
 * renders as its rich telemetry card (last message, tool activity, approvals,
 * compose box); clicking a card opens a GUI viewer pane attached to that
 * agent's session in the CURRENT workspace, so you can watch several agents
 * side by side without leaving where you are.
 */
const AgentsPane: React.FC<{ isActive?: boolean }> = () => {
  ensureFleetKeyframes();
  const { agents, snapshotBySession, counts, topByAgent, openAgent } = useAttention();
  const [query, setQuery] = useState('');

  const realAgents = useMemo(() => agents.filter((a) => !a.global), [agents]);

  // Needy-first, same rule as the SideBar / Fleet Deck.
  const sorted = useMemo(() => {
    const q = query.trim().toLowerCase();
    return realAgents
      .filter((a) => !q || a.name.toLowerCase().includes(q) || (a.provider ?? 'claude').toLowerCase().includes(q))
      .sort((a, b) => {
        const sa = agentAttentionScore(a.sessionId ? snapshotBySession[a.sessionId]?.ambientState : undefined, topByAgent.get(a.id)?.priority ?? 0);
        const sb = agentAttentionScore(b.sessionId ? snapshotBySession[b.sessionId]?.ambientState : undefined, topByAgent.get(b.id)?.priority ?? 0);
        return sb - sa;
      });
  }, [realAgents, snapshotBySession, topByAgent, query]);

  const working = realAgents.filter((a) => {
    const s = a.sessionId ? snapshotBySession[a.sessionId]?.ambientState : undefined;
    return s === 'thinking' || s === 'streaming';
  }).length;

  const watch = (agent: (typeof realAgents)[number]) => {
    // A stopped agent has no session to attach to — pilot into its workspace
    // instead, where the sidebar offers the respawn.
    if (!agent.sessionId) { openAgent(agent.id); return; }
    requestSessionWatch({
      sessionId: agent.sessionId,
      cwd: agent.cwd,
      title: `Watch: ${agent.name}`,
      provider: agent.provider,
    });
  };

  return (
    <div style={{ height: '100%', display: 'flex', flexDirection: 'column', background: 'var(--wks-bg-base)', color: 'var(--wks-text-primary)' }}>
      {/* Header */}
      <div style={{ display: 'flex', alignItems: 'center', gap: 12, padding: '12px 18px 8px', flexShrink: 0 }}>
        <div style={{ fontSize: '0.95rem', fontWeight: 800, letterSpacing: '-0.01em' }}>Agents</div>
        <div style={{ fontSize: '0.7rem', color: 'var(--wks-text-secondary)', fontVariantNumeric: 'tabular-nums', whiteSpace: 'nowrap' }}>
          {realAgents.length} agent{realAgents.length === 1 ? '' : 's'} · {working} working · {counts.needsYou} need{counts.needsYou === 1 ? 's' : ''} you
        </div>
        <div style={{ flex: 1 }} />
        <span style={{ fontSize: '0.62rem', color: 'var(--wks-text-faint)', whiteSpace: 'nowrap' }}>
          Click a card to watch it in a GUI pane
        </span>
        <input
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="Filter agents…"
          spellCheck={false}
          style={{
            width: 150, fontSize: '0.72rem', fontFamily: 'inherit', padding: '5px 9px',
            borderRadius: 8, border: '1px solid var(--wks-border-subtle)',
            background: 'var(--wks-bg-surface)', color: 'var(--wks-text-primary)',
          }}
        />
      </div>

      {/* Card grid */}
      <div style={{ flex: 1, overflowY: 'auto', padding: '6px 18px 24px' }}>
        {sorted.length === 0 ? (
          <div style={{ marginTop: 60, textAlign: 'center', color: 'var(--wks-text-faint)' }}>
            <div style={{ fontSize: '0.85rem', fontWeight: 600, color: 'var(--wks-text-secondary)' }}>
              {realAgents.length === 0 ? 'No agents running' : 'No agents match the filter'}
            </div>
            {realAgents.length === 0 && (
              <div style={{ fontSize: '0.74rem', marginTop: 6 }}>Spawn an agent and it'll appear here as a live card.</div>
            )}
          </div>
        ) : (
          <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(340px, 1fr))', gap: 16, alignItems: 'start' }}>
            {sorted.map((agent) => (
              <AgentCard
                key={agent.id}
                agent={agent}
                snapshot={agent.sessionId ? snapshotBySession[agent.sessionId] : undefined}
                onOpen={() => watch(agent)}
              />
            ))}
          </div>
        )}
      </div>
    </div>
  );
};

export default AgentsPane;
