import React from 'react';
import type { AgentWorkspace } from '../types/pane';
import type { ClaudeSessionSnapshot, SessionAmbientState } from '../types/claudeSession';
import { formatToolSummary } from './claude-shared';
import { useAttention } from '../contexts/AttentionContext';

function fmtTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${Math.round(n / 1_000)}k`;
  return `${n}`;
}
function fmtUSD(n: number): string {
  return n >= 0.01 ? `$${n.toFixed(2)}` : n > 0 ? '<$0.01' : '$0.00';
}
function ctxColor(frac: number): string {
  if (frac >= 0.9) return 'var(--wks-danger, #e05555)';
  if (frac >= 0.7) return 'var(--wks-warning, #e0a000)';
  return 'var(--wks-success, #3fb950)';
}

interface StateVisual { color: string; label: string; pulse: boolean }
function stateVisual(s: SessionAmbientState | undefined): StateVisual {
  switch (s) {
    case 'waiting_approval': return { color: 'var(--wks-warning, #e0a000)', label: 'Needs approval', pulse: true };
    case 'waiting_input':    return { color: 'var(--wks-warning, #e0a000)', label: 'Waiting for input', pulse: true };
    case 'thinking':         return { color: 'var(--wks-accent, #4a9eff)', label: 'Thinking', pulse: false };
    case 'streaming':        return { color: 'var(--wks-accent, #4a9eff)', label: 'Working', pulse: false };
    case 'idle':             return { color: 'var(--wks-success, #3fb950)', label: 'Idle', pulse: false };
    default:                 return { color: 'var(--wks-text-faint, #666)', label: 'Stopped', pulse: false };
  }
}

function lastAssistant(snap: ClaudeSessionSnapshot | undefined): string {
  const turns = snap?.conversation ?? [];
  for (let i = turns.length - 1; i >= 0; i--) {
    if (turns[i].role === 'assistant' && turns[i].content?.trim()) return turns[i].content.trim();
  }
  return '';
}

interface Props {
  agent: AgentWorkspace;
  snapshot?: ClaudeSessionSnapshot;
  /** Score-derived buoyancy badge ("needs you" etc.), already computed by the deck. */
  rank?: number;
}

/**
 * A live, glanceable Fleet Deck tile — the "telemetry face" of one agent,
 * rendered purely from its snapshot (no live terminal at deck scale, so N
 * agents stay cheap). Border tints by ambient state and pulses when the agent
 * is blocked on you. Click pilots into the agent; quick-actions resolve the
 * common blocked case without leaving the deck.
 */
export const AgentCard: React.FC<Props> = ({ agent, snapshot }) => {
  const { openAgent, approve, feed } = useAttention();
  const state = snapshot?.ambientState;
  const v = stateVisual(agent.sessionId ? state : undefined);
  const usage = snapshot?.usage;
  const ctxFrac = usage && usage.contextLimit > 0 ? Math.min(1, usage.contextTokens / usage.contextLimit) : 0;

  const activeTool = snapshot?.activeToolCalls?.[snapshot.activeToolCalls.length - 1];
  const runningSubs = (snapshot?.subagents ?? []).filter((s) => s.status === 'running').length;
  const runningWf = (snapshot?.workflows ?? []).filter((w) => w.status === 'running');
  const approvalItem = feed.find((it) => it.agentId === agent.id && it.kind === 'approval');

  const working = state === 'thinking' || state === 'streaming';
  const body = working && activeTool
    ? formatToolSummary(activeTool).call
    : lastAssistant(snapshot) || (agent.sessionId ? 'No activity yet' : 'Stopped — click to respawn');

  return (
    <div
      onClick={() => openAgent(agent.id)}
      title={`${agent.name} — ${v.label}\n${agent.cwd}`}
      style={{
        display: 'flex', flexDirection: 'column', minHeight: 150, cursor: 'pointer',
        borderRadius: 'var(--wks-radius-lg)', overflow: 'hidden',
        background: 'var(--wks-bg-surface)',
        border: `1.5px solid ${v.color}`,
        boxShadow: v.pulse ? `0 0 0 1px ${v.color}` : '0 4px 16px var(--wks-shadow)',
        animation: v.pulse ? 'fleetPulse 1.8s ease-in-out infinite' : undefined,
        transition: 'transform 0.1s ease, box-shadow 0.12s ease',
      }}
      onMouseEnter={(e) => { (e.currentTarget as HTMLElement).style.transform = 'translateY(-2px)'; }}
      onMouseLeave={(e) => { (e.currentTarget as HTMLElement).style.transform = ''; }}
    >
      {/* Header */}
      <div style={{ display: 'flex', alignItems: 'center', gap: 8, padding: '10px 12px' }}>
        <span style={{ width: 9, height: 9, borderRadius: '50%', background: v.color, flexShrink: 0, boxShadow: state && state !== 'idle' ? `0 0 7px ${v.color}` : 'none' }} />
        <span style={{ fontSize: '0.85rem', fontWeight: 700, color: 'var(--wks-text-primary)', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{agent.name}</span>
        <span style={{ marginLeft: 'auto', fontSize: '0.64rem', fontWeight: 600, color: v.color, flexShrink: 0 }}>{v.label}</span>
      </div>

      {/* Body: current tool or last message */}
      <div style={{ flex: 1, padding: '0 12px 8px', fontSize: '0.74rem', color: 'var(--wks-text-secondary)', lineHeight: 1.45, overflow: 'hidden', display: '-webkit-box', WebkitLineClamp: 4, WebkitBoxOrient: 'vertical', fontFamily: working && activeTool ? 'var(--claude-mono-font, monospace)' : 'inherit' }}>
        {body}
      </div>

      {/* Orchestration mini-progress */}
      {(runningSubs > 0 || runningWf.length > 0) && (
        <div style={{ padding: '0 12px 8px', display: 'flex', gap: 8, fontSize: '0.64rem', color: 'var(--wks-accent)', fontWeight: 600 }}>
          {runningWf.length > 0 && <span>⚙ {runningWf[0].name || 'workflow'}</span>}
          {runningSubs > 0 && <span>◇ {runningSubs} subagent{runningSubs > 1 ? 's' : ''}</span>}
        </div>
      )}

      {/* Footer: context bar + cost, or quick-approve when blocked */}
      <div style={{ padding: '8px 12px', borderTop: '1px solid var(--wks-glass-border)', background: 'var(--wks-glass-strong)' }}>
        {approvalItem ? (
          <div style={{ display: 'flex', gap: 6 }} onClick={(e) => e.stopPropagation()}>
            <button onClick={() => approve(approvalItem, 'yes')} style={qa('var(--wks-success, #3fb950)')}>Allow</button>
            <button onClick={() => approve(approvalItem, 'no')} style={qa('var(--wks-error, #f87171)')}>Deny</button>
            <button onClick={() => openAgent(agent.id)} style={{ ...qa('var(--wks-text-secondary)'), marginLeft: 'auto' }}>Open</button>
          </div>
        ) : (
          <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
            {usage && usage.contextTokens > 0 ? (
              <>
                <span style={{ flex: 1, height: 4, borderRadius: 2, background: 'var(--wks-border-subtle, #2a2a2a)', overflow: 'hidden' }}>
                  <span style={{ display: 'block', height: '100%', width: `${Math.max(2, ctxFrac * 100)}%`, background: ctxColor(ctxFrac) }} />
                </span>
                <span style={{ fontSize: '0.62rem', color: ctxColor(ctxFrac), fontVariantNumeric: 'tabular-nums', flexShrink: 0 }}>{Math.round(ctxFrac * 100)}%</span>
                <span style={{ fontSize: '0.62rem', color: 'var(--wks-text-faint)', flexShrink: 0 }}>{fmtUSD(usage.costUSD)}</span>
              </>
            ) : (
              <span style={{ fontSize: '0.62rem', color: 'var(--wks-text-faint)' }}>{usage?.model?.replace(/^claude-/, '') || ''}</span>
            )}
          </div>
        )}
      </div>
    </div>
  );
};

function qa(color: string): React.CSSProperties {
  return {
    fontSize: '0.66rem', fontWeight: 700, fontFamily: 'inherit', padding: '3px 12px',
    borderRadius: 5, border: `1px solid ${color}`, background: 'transparent', color, cursor: 'pointer',
  };
}
