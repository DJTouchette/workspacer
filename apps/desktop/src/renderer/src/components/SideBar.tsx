import React, { useState } from 'react';
import { AgentWorkspace } from '../types/pane';
import type { SessionAmbientState, ClaudeSessionSnapshot } from '../types/claudeSession';
import { deriveSessionStats } from '../lib/sessionStats';
import HubStatus from './HubStatus';
import { ContextMenu, ContextMenuItem } from './ContextMenu';

export const SIDEBAR_WIDTH = 196;

/** 142345 → "142k", 1_200_000 → "1.2M". */
function fmtTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(n >= 10_000_000 ? 0 : 1)}M`;
  if (n >= 1_000) return `${Math.round(n / 1_000)}k`;
  return `${n}`;
}

function fmtUSD(n: number): string {
  if (n >= 10) return `$${n.toFixed(2)}`;
  if (n >= 0.01) return `$${n.toFixed(2)}`;
  return n > 0 ? '<$0.01' : '$0.00';
}

/** Green → amber → red as the context window fills. */
function contextColor(frac: number): string {
  if (frac >= 0.9) return 'var(--wks-danger, #e05555)';
  if (frac >= 0.7) return 'var(--wks-warning, #e0a000)';
  return 'var(--wks-success, #3fb950)';
}

/** Ambient state (or `undefined` = stopped) → status dot color + label. */
function statusVisual(state: SessionAmbientState | undefined): { color: string; label: string } {
  switch (state) {
    case 'waiting_approval': return { color: 'var(--wks-warning, #e0a000)', label: 'Needs approval' };
    case 'waiting_input': return { color: 'var(--wks-warning, #e0a000)', label: 'Waiting for input' };
    case 'thinking': return { color: 'var(--wks-accent, #4a9eff)', label: 'Thinking' };
    case 'streaming': return { color: 'var(--wks-accent, #4a9eff)', label: 'Working' };
    case 'idle': return { color: 'var(--wks-success, #3fb950)', label: 'Idle' };
    default: return { color: 'var(--wks-text-faint, #666)', label: 'Stopped' };
  }
}

interface SideBarProps {
  agents: AgentWorkspace[];
  activeAgentId: string;
  /** sessionId → live ambient state, from claudeSessionStore. */
  statusBySession: Record<string, SessionAmbientState>;
  /**
   * sessionId → full live snapshot. The per-agent context bar derives its
   * numbers from this via `deriveSessionStats`, the same source the agent
   * pane's status bar uses, so the two can never disagree.
   */
  snapshotBySession: Record<string, ClaudeSessionSnapshot>;
  onSelectAgent: (id: string) => void;
  onSpawnAgent: () => void;
  onTerminateAgent: (id: string) => void;
  onRenameAgent: (id: string, name: string) => void;
  /** Jump to the next agent blocked on the user (approval / input). */
  onJumpToAttention?: () => void;
  /** Open the Triage Inbox drawer. */
  onOpenInbox?: () => void;
  /** Toggle the Fleet Deck (cross-agent radar). */
  onToggleFleet?: () => void;
  /** Current altitude — highlights the Fleet button when active. */
  viewLevel?: 'fleet' | 'piloting';
  /** Open the remote-control (phone sharing) panel. */
  onOpenRemote?: () => void;
  /** Collapse the sidebar (so panes take the full width). */
  onToggleCollapse?: () => void;
}

const SideBar: React.FC<SideBarProps> = ({
  agents,
  activeAgentId,
  statusBySession,
  snapshotBySession,
  onSelectAgent,
  onSpawnAgent,
  onTerminateAgent,
  onRenameAgent,
  onJumpToAttention,
  onOpenInbox,
  onToggleFleet,
  viewLevel,
  onOpenRemote,
  onToggleCollapse,
}) => {
  // Aggregate live counts for the header summary.
  const needYouCount = agents.reduce((n, a) => {
    const s = a.sessionId ? statusBySession[a.sessionId] : undefined;
    return n + (s === 'waiting_approval' || s === 'waiting_input' ? 1 : 0);
  }, 0);
  const workingCount = agents.reduce((n, a) => {
    const s = a.sessionId ? statusBySession[a.sessionId] : undefined;
    return n + (s === 'thinking' || s === 'streaming' ? 1 : 0);
  }, 0);
  const [contextMenu, setContextMenu] = useState<{ agentId: string; x: number; y: number } | null>(null);
  const [renamingId, setRenamingId] = useState<string | null>(null);
  const [renameValue, setRenameValue] = useState('');

  const commitRename = () => {
    if (renamingId && renameValue.trim()) onRenameAgent(renamingId, renameValue.trim());
    setRenamingId(null);
  };

  return (
    <div style={{
      position: 'absolute',
      top: 0, left: 0, bottom: 0,
      width: `${SIDEBAR_WIDTH}px`,
      display: 'flex',
      flexDirection: 'column',
      paddingTop: '8px',
      gap: '2px',
      backgroundColor: 'var(--wks-glass-strong)',
      backdropFilter: 'blur(var(--wks-glass-blur)) saturate(160%)',
      WebkitBackdropFilter: 'blur(var(--wks-glass-blur)) saturate(160%)',
      borderRight: '1px solid var(--wks-glass-border)',
      boxShadow: 'inset -1px 0 0 var(--wks-glass-highlight)',
      // Round the inner (right) corners with the active corner style — the left
      // edge stays flush to the window (rounded by the app shell). Square corner
      // style resolves these to 0.
      borderTopRightRadius: 'var(--wks-radius-lg)',
      borderBottomRightRadius: 'var(--wks-radius-lg)',
      // Clip children (e.g. the HubStatus footer's solid background) to the
      // rounded corners. The context menu is position:fixed so it still escapes.
      overflow: 'hidden',
      zIndex: 100,
      userSelect: 'none',
      boxSizing: 'border-box',
    }}>
      <div style={{
        padding: '4px 12px 8px',
        fontSize: '0.6rem',
        fontWeight: 700,
        letterSpacing: '0.08em',
        textTransform: 'uppercase',
        color: 'var(--wks-text-faint)',
        display: 'flex',
        alignItems: 'center',
        gap: 6,
      }}>
        <span>Agents</span>
        {needYouCount > 0 && (
          <span
            onClick={onOpenInbox ?? onJumpToAttention}
            title="Open the Triage Inbox"
            style={{
              color: 'var(--wks-warning, #e0a000)',
              cursor: (onOpenInbox ?? onJumpToAttention) ? 'pointer' : 'default',
              letterSpacing: 0,
              textTransform: 'none',
              fontWeight: 600,
            }}
          >
            {needYouCount} need you
          </span>
        )}
        {needYouCount === 0 && workingCount > 0 && (
          <span style={{
            color: 'var(--wks-accent, #4a9eff)',
            letterSpacing: 0,
            textTransform: 'none',
            fontWeight: 600,
          }}>
            {workingCount} working
          </span>
        )}
        <div style={{ flex: 1 }} />
        {onToggleCollapse && (
          <button
            onClick={onToggleCollapse}
            title="Collapse sidebar (Ctrl+B)"
            style={{
              width: 20, height: 20, padding: 0, flexShrink: 0,
              display: 'flex', alignItems: 'center', justifyContent: 'center',
              border: 'none', borderRadius: 5, cursor: 'pointer',
              background: 'transparent', color: 'var(--wks-text-faint)', fontSize: '0.9rem', lineHeight: 1,
            }}
          >«</button>
        )}
      </div>

      {/* Mission Control: the cross-agent surfaces — always reachable. */}
      <div style={{ display: 'flex', gap: 6, padding: '2px 6px 6px' }}>
        <button
          onClick={onOpenInbox}
          title="Triage Inbox (Ctrl+Shift+A)"
          style={mcBtnStyle(false)}
        >
          <span>Inbox</span>
          {needYouCount > 0 && (
            <span style={{
              marginLeft: 'auto', minWidth: 16, height: 16, padding: '0 4px', borderRadius: 8,
              display: 'inline-flex', alignItems: 'center', justifyContent: 'center',
              background: 'var(--wks-warning, #e0a000)', color: '#1a1a1a', fontSize: '0.6rem', fontWeight: 700,
            }}>{needYouCount}</span>
          )}
        </button>
        <button
          onClick={onToggleFleet}
          title="Fleet Deck (Ctrl+Shift+F)"
          style={mcBtnStyle(viewLevel === 'fleet')}
        >
          Fleet
        </button>
      </div>

      <div style={{ flex: 1, overflowY: 'auto', display: 'flex', flexDirection: 'column', gap: '6px', padding: '2px 0' }}>
        {agents.length === 0 && (
          <div style={{ padding: '8px 12px', fontSize: '0.7rem', color: 'var(--wks-text-faint)', lineHeight: 1.5 }}>
            No agents yet. Spawn one to start a Claude Code session.
          </div>
        )}

        {/* Render agents with nested children beneath their parent.
            Strategy: any agent with a parentId that resolves to a known agent's
            id is rendered indented below that parent. Top-level agents are those
            with no parentId, or whose parentId doesn't resolve (fallback so
            nothing disappears). Children are NOT rendered again at top level. */}
        {(() => {
          // Build a set of all known agent ids for fast parent-resolution checks.
          const agentIds = new Set(agents.map((a) => a.id));
          // Build a lookup: parentId → child agents (any kind with a resolvable parentId).
          const childrenByParent = new Map<string, typeof agents>();
          const topLevel: typeof agents = [];
          for (const agent of agents) {
            if (agent.parentId && agentIds.has(agent.parentId)) {
              const bucket = childrenByParent.get(agent.parentId) ?? [];
              bucket.push(agent);
              childrenByParent.set(agent.parentId, bucket);
            } else {
              topLevel.push(agent);
            }
          }

          const renderAgentRow = (agent: typeof agents[0], indent?: boolean) => {
            const isActive = agent.id === activeAgentId;
            const isGlobal = !!agent.global;
            const isSupervisor = agent.kind === 'supervisor';
            const state = agent.sessionId ? statusBySession[agent.sessionId] : undefined;
            const { color, label } = statusVisual(state);
            const isRenaming = renamingId === agent.id;
            const snap = agent.sessionId ? snapshotBySession[agent.sessionId] : undefined;
            const stats = deriveSessionStats(snap);
            const hasCtx = stats.ctxPct !== undefined;
            const ctxFrac = hasCtx ? Math.min(1, stats.ctxPct! / 100) : 0;
            const usageTip = hasCtx
              ? `\n${Math.round(stats.ctxPct!)}% context${stats.tokens !== undefined ? ` · ${fmtTokens(stats.tokens)} tok` : ''}${stats.costUSD !== undefined ? ` · ${fmtUSD(stats.costUSD)}` : ''}${stats.model ? ` · ${stats.model}` : ''}`
              : '';

            return (
              <button
                key={agent.id}
                onClick={() => onSelectAgent(agent.id)}
                onContextMenu={(e) => {
                  e.preventDefault();
                  if (isGlobal) return; // Overview can't be renamed/terminated
                  setContextMenu({ agentId: agent.id, x: e.clientX, y: e.clientY });
                }}
                style={{
                  width: indent ? 'calc(100% - 24px)' : 'calc(100% - 12px)',
                  margin: indent ? '0 6px 0 18px' : '0 6px',
                  padding: '9px 11px',
                  display: 'flex',
                  alignItems: 'center',
                  gap: '9px',
                  borderRadius: 'var(--wks-radius-md)',
                  cursor: 'pointer',
                  fontSize: indent ? '0.78rem' : '0.85rem',
                  fontFamily: 'inherit',
                  fontWeight: isActive ? 600 : 500,
                  backgroundColor: isActive ? 'var(--wks-bg-selected)' : 'var(--wks-bg-surface)',
                  color: isActive ? 'var(--wks-text-primary)' : 'var(--wks-text-secondary)',
                  border: `1px solid ${isActive ? 'var(--wks-accent)' : 'var(--wks-glass-border)'}`,
                  textAlign: 'left',
                  boxSizing: 'border-box',
                  transition: 'border-color 0.12s, background-color 0.12s',
                  opacity: indent ? 0.9 : 1,
                }}
                title={isGlobal ? 'Overview — cross-agent dashboards & plugin panes' : `${agent.name} — ${label}\n${agent.cwd}${usageTip}`}
              >
                {isGlobal ? (
                  <span style={{ width: 8, flexShrink: 0, textAlign: 'center', fontSize: '0.7rem', lineHeight: 1 }}>▦</span>
                ) : isSupervisor ? (
                  <span style={{ width: 8, flexShrink: 0, textAlign: 'center', fontSize: '0.65rem', lineHeight: 1 }}>🧭</span>
                ) : (
                  <span
                    style={{
                      width: 8, height: 8, borderRadius: '50%', flexShrink: 0,
                      backgroundColor: color,
                      boxShadow: state && state !== 'idle' ? `0 0 6px ${color}` : 'none',
                    }}
                  />
                )}
                <span style={{ flex: 1, minWidth: 0, display: 'flex', flexDirection: 'column', gap: '1px' }}>
                  {isRenaming ? (
                    <input
                      autoFocus
                      value={renameValue}
                      onChange={(e) => setRenameValue(e.target.value)}
                      onBlur={commitRename}
                      onKeyDown={(e) => {
                        if (e.key === 'Enter') commitRename();
                        if (e.key === 'Escape') setRenamingId(null);
                        e.stopPropagation();
                      }}
                      onClick={(e) => e.stopPropagation()}
                      style={{
                        width: '100%', fontSize: '0.75rem', fontFamily: 'inherit',
                        background: 'var(--wks-bg-base)', color: 'var(--wks-text-primary)',
                        border: '1px solid var(--wks-accent)', borderRadius: 3, padding: '1px 4px',
                        boxSizing: 'border-box',
                      }}
                    />
                  ) : (
                    <span style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                      {agent.name}
                    </span>
                  )}
                  <span style={{
                    fontSize: '0.67rem', color: 'var(--wks-text-faint)',
                    display: 'flex', alignItems: 'baseline', gap: 4,
                    overflow: 'hidden', whiteSpace: 'nowrap',
                  }}>
                    <span style={{ overflow: 'hidden', textOverflow: 'ellipsis' }}>
                      {isGlobal ? 'Dashboards & plugins' : agent.sessionId ? label : 'Stopped — click to respawn'}
                    </span>
                    {hasCtx && (
                      <span style={{ marginLeft: 'auto', flexShrink: 0, color: contextColor(ctxFrac), fontVariantNumeric: 'tabular-nums' }}>
                        {Math.round(ctxFrac * 100)}%
                      </span>
                    )}
                  </span>
                  {hasCtx && (
                    <span style={{
                      height: 3, borderRadius: 2, marginTop: 1,
                      backgroundColor: 'var(--wks-border-subtle, #2a2a2a)',
                      overflow: 'hidden', display: 'block',
                    }}>
                      <span style={{
                        display: 'block', height: '100%',
                        width: `${Math.max(2, ctxFrac * 100)}%`,
                        backgroundColor: contextColor(ctxFrac),
                      }} />
                    </span>
                  )}
                </span>
              </button>
            );
          };

          const rows: React.ReactNode[] = [];
          for (const agent of topLevel) {
            rows.push(renderAgentRow(agent, false));
            // Render children indented directly after their parent.
            const children = childrenByParent.get(agent.id) ?? [];
            for (const child of children) {
              rows.push(renderAgentRow(child, true));
            }
          }
          return rows;
        })()}
      </div>

      {/* Spawn button */}
      <button
        onClick={onSpawnAgent}
        style={{
          width: 'calc(100% - 8px)',
          margin: '4px 4px 8px',
          padding: '8px',
          display: 'flex',
          alignItems: 'center',
          gap: '8px',
          border: '1px dashed var(--wks-border-input)',
          borderRadius: '6px',
          cursor: 'pointer',
          fontSize: '0.75rem',
          fontFamily: 'inherit',
          backgroundColor: 'transparent',
          color: 'var(--wks-text-muted)',
          textAlign: 'left',
          boxSizing: 'border-box',
        }}
        title="Spawn a new agent"
      >
        <span style={{ width: 8, display: 'inline-flex', justifyContent: 'center', fontSize: '0.95rem' }}>+</span>
        <span>Spawn agent</span>
      </button>

      {/* Hub bus status — sits in-flow at the bottom of the sidebar */}
      <HubStatus onOpenRemote={onOpenRemote} />

      {/* Context menu */}
      {contextMenu && (
        <ContextMenu x={contextMenu.x} y={contextMenu.y} minWidth={140} onClose={() => setContextMenu(null)}>
          <ContextMenuItem
            label="Rename"
            onClick={() => {
              const agent = agents.find((a) => a.id === contextMenu.agentId);
              setRenameValue(agent?.name ?? '');
              setRenamingId(contextMenu.agentId);
              setContextMenu(null);
            }}
          />
          <ContextMenuItem
            label="Terminate"
            danger
            onClick={() => {
              const id = contextMenu.agentId;
              setContextMenu(null);
              onTerminateAgent(id);
            }}
          />
        </ContextMenu>
      )}
    </div>
  );
};

function mcBtnStyle(active: boolean): React.CSSProperties {
  return {
    flex: 1, display: 'flex', alignItems: 'center', gap: 6,
    padding: '6px 10px', borderRadius: 'var(--wks-radius-md)', cursor: 'pointer',
    fontSize: '0.72rem', fontFamily: 'inherit', fontWeight: 600,
    border: `1px solid ${active ? 'var(--wks-accent)' : 'var(--wks-glass-border)'}`,
    background: active ? 'var(--wks-bg-selected)' : 'var(--wks-bg-surface)',
    color: active ? 'var(--wks-text-primary)' : 'var(--wks-text-secondary)',
    boxSizing: 'border-box',
  };
}

export default SideBar;
