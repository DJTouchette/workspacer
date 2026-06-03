import React, { useState, useRef, useEffect } from 'react';
import { AgentWorkspace } from '../types/pane';
import type { SessionAmbientState, SessionUsage } from '../types/claudeSession';
import HubStatus from './HubStatus';

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
  /** sessionId → token / cost / context usage, parsed from the transcript. */
  usageBySession: Record<string, SessionUsage>;
  onSelectAgent: (id: string) => void;
  onSpawnAgent: () => void;
  onTerminateAgent: (id: string) => void;
  onRenameAgent: (id: string, name: string) => void;
  /** Jump to the next agent blocked on the user (approval / input). */
  onJumpToAttention?: () => void;
}

const SideBar: React.FC<SideBarProps> = ({
  agents,
  activeAgentId,
  statusBySession,
  usageBySession,
  onSelectAgent,
  onSpawnAgent,
  onTerminateAgent,
  onRenameAgent,
  onJumpToAttention,
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
  const [contextMenu, setContextMenu] = useState<{ agentId: string; y: number } | null>(null);
  const [renamingId, setRenamingId] = useState<string | null>(null);
  const [renameValue, setRenameValue] = useState('');
  const cmRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!contextMenu) return;
    const handler = (e: MouseEvent) => {
      if (cmRef.current && !cmRef.current.contains(e.target as Node)) setContextMenu(null);
    };
    document.addEventListener('mousedown', handler);
    return () => document.removeEventListener('mousedown', handler);
  }, [contextMenu]);

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
        alignItems: 'baseline',
        gap: 6,
      }}>
        <span>Agents</span>
        {needYouCount > 0 && (
          <span
            onClick={onJumpToAttention}
            title="Jump to the next agent that needs you"
            style={{
              color: 'var(--wks-warning, #e0a000)',
              cursor: onJumpToAttention ? 'pointer' : 'default',
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
      </div>

      <div style={{ flex: 1, overflowY: 'auto', display: 'flex', flexDirection: 'column', gap: '2px' }}>
        {agents.length === 0 && (
          <div style={{ padding: '8px 12px', fontSize: '0.7rem', color: 'var(--wks-text-faint)', lineHeight: 1.5 }}>
            No agents yet. Spawn one to start a Claude Code session.
          </div>
        )}

        {agents.map((agent) => {
          const isActive = agent.id === activeAgentId;
          const isGlobal = !!agent.global;
          const state = agent.sessionId ? statusBySession[agent.sessionId] : undefined;
          const { color, label } = statusVisual(state);
          const isRenaming = renamingId === agent.id;
          const usage = agent.sessionId ? usageBySession[agent.sessionId] : undefined;
          const ctxFrac = usage && usage.contextLimit > 0
            ? Math.min(1, usage.contextTokens / usage.contextLimit)
            : 0;
          const usageTip = usage
            ? `\n${fmtTokens(usage.contextTokens)} / ${fmtTokens(usage.contextLimit)} context · ${fmtUSD(usage.costUSD)}${usage.model ? ` · ${usage.model.replace(/^claude-/, '')}` : ''}`
            : '';

          return (
            <button
              key={agent.id}
              onClick={() => onSelectAgent(agent.id)}
              onContextMenu={(e) => {
                e.preventDefault();
                if (isGlobal) return; // Overview can't be renamed/terminated
                setContextMenu({ agentId: agent.id, y: e.clientY });
              }}
              style={{
                width: 'calc(100% - 8px)',
                margin: '0 4px',
                padding: '6px 8px',
                display: 'flex',
                alignItems: 'center',
                gap: '8px',
                border: 'none',
                borderRadius: '6px',
                cursor: 'pointer',
                fontSize: '0.75rem',
                fontFamily: 'inherit',
                fontWeight: isActive ? 600 : 400,
                backgroundColor: isActive ? 'var(--wks-bg-selected)' : 'transparent',
                color: isActive ? 'var(--wks-text-primary)' : 'var(--wks-text-muted)',
                borderLeft: isActive ? '2px solid var(--wks-accent)' : '2px solid transparent',
                textAlign: 'left',
                boxSizing: 'border-box',
              }}
              title={isGlobal ? 'Overview — cross-agent dashboards & plugin panes' : `${agent.name} — ${label}\n${agent.cwd}${usageTip}`}
            >
              {isGlobal ? (
                <span style={{ width: 8, flexShrink: 0, textAlign: 'center', fontSize: '0.7rem', lineHeight: 1 }}>▦</span>
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
                  fontSize: '0.6rem', color: 'var(--wks-text-faint)',
                  display: 'flex', alignItems: 'baseline', gap: 4,
                  overflow: 'hidden', whiteSpace: 'nowrap',
                }}>
                  <span style={{ overflow: 'hidden', textOverflow: 'ellipsis' }}>
                    {isGlobal ? 'Dashboards & plugins' : agent.sessionId ? label : 'Stopped — click to respawn'}
                  </span>
                  {usage && usage.contextTokens > 0 && (
                    <span style={{ marginLeft: 'auto', flexShrink: 0, color: contextColor(ctxFrac), fontVariantNumeric: 'tabular-nums' }}>
                      {Math.round(ctxFrac * 100)}%
                    </span>
                  )}
                </span>
                {usage && usage.contextTokens > 0 && (
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
        })}
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
      <HubStatus />

      {/* Context menu */}
      {contextMenu && (
        <div
          ref={cmRef}
          style={{
            position: 'fixed',
            left: `${SIDEBAR_WIDTH + 4}px`,
            top: contextMenu.y,
            backgroundColor: 'var(--wks-bg-surface)',
            border: '1px solid var(--wks-border-input)',
            borderRadius: '4px',
            padding: '4px 0',
            zIndex: 10000,
            minWidth: '140px',
            boxShadow: '0 4px 12px var(--wks-shadow)',
          }}
        >
          <SideMenuItem
            label="Rename"
            onClick={() => {
              const agent = agents.find((a) => a.id === contextMenu.agentId);
              setRenameValue(agent?.name ?? '');
              setRenamingId(contextMenu.agentId);
              setContextMenu(null);
            }}
          />
          <SideMenuItem
            label="Terminate"
            danger
            onClick={() => {
              const agent = agents.find((a) => a.id === contextMenu.agentId);
              setContextMenu(null);
              if (window.confirm(`Terminate agent "${agent?.name}"? This kills its Claude session.`)) {
                onTerminateAgent(contextMenu.agentId);
              }
            }}
          />
        </div>
      )}
    </div>
  );
};

function SideMenuItem({ label, onClick, danger }: { label: string; onClick: () => void; danger?: boolean }) {
  return (
    <button
      onClick={onClick}
      style={{
        display: 'block', width: '100%', padding: '5px 12px', margin: 0,
        border: 'none', borderRadius: 0, cursor: 'pointer',
        fontSize: '0.7rem', fontFamily: 'inherit', fontWeight: 400,
        backgroundColor: 'transparent',
        color: danger ? 'var(--wks-danger, #e05555)' : 'var(--wks-text-tertiary)',
        textAlign: 'left', height: 'auto', lineHeight: '1.4',
      }}
      onMouseEnter={(e) => { (e.currentTarget as HTMLElement).style.backgroundColor = 'var(--wks-bg-selected)'; }}
      onMouseLeave={(e) => { (e.currentTarget as HTMLElement).style.backgroundColor = 'transparent'; }}
    >
      {label}
    </button>
  );
}

export default SideBar;
