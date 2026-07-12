import React, { useState, useRef, useMemo } from 'react';
import { Plus, ChevronLeft, ChevronRight, HelpCircle } from 'lucide-react';
import { IconInbox, IconOverview, IconWorking } from './wksIcons';
import { BrandMark, Wordmark } from './Brand';
import { AgentWorkspace } from '../types/pane';
import type { SessionAmbientState, ClaudeSessionSnapshot } from '../types/claudeSession';
import type { AttentionItem, AttentionKind } from '../types/attention';
import { deriveSessionStats, fmtTokens, fmtUSD, ctxColor } from '../lib/sessionStats';
import { agentAttentionScore } from '../lib/attentionRouter';
import { AgentLogo } from './agentLogos';
import { requestInspector } from '../lib/watchBus';
import { useAttention } from '../contexts/AttentionContext';
import { useUiMode } from '../hooks/useUiMode';
import HubStatus from './HubStatus';
import { ContextMenu, ContextMenuItem } from './ContextMenu';

export const SIDEBAR_WIDTH = 268;
/** Width of the collapsed monogram rail (desktop). */
export const SIDEBAR_RAIL_WIDTH = 74;

/** Ambient state (or `undefined` = stopped) → status dot color + label. */
function statusVisual(state: SessionAmbientState | undefined): { color: string; label: string } {
  switch (state) {
    case 'waiting_approval':
      return { color: 'var(--wks-warning, #e0a000)', label: 'Needs approval' };
    case 'waiting_input':
      return { color: 'var(--wks-warning, #e0a000)', label: 'Waiting for input' };
    case 'thinking':
      return { color: 'var(--wks-busy, var(--wks-accent, #4a9eff))', label: 'Thinking' };
    case 'streaming':
      return { color: 'var(--wks-busy, var(--wks-accent, #4a9eff))', label: 'Working' };
    case 'idle':
      return { color: 'var(--wks-success, #3fb950)', label: 'Idle' };
    default:
      return { color: 'var(--wks-text-faint, #666)', label: 'Stopped' };
  }
}

/** A top attention item tints the row dot and shows a tiny kind glyph. */
const KIND_GLYPH: Record<AttentionKind, string> = {
  approval: '!',
  question: '?',
  error: '×',
  stuck: '◷',
  bigdiff: '±',
  done: '✓',
};
const KIND_COLOR: Record<AttentionKind, string> = {
  approval: 'var(--wks-warning, #e0a000)',
  question: 'var(--wks-accent, #4a9eff)',
  error: 'var(--wks-danger, #e05555)',
  stuck: 'var(--wks-warning, #e0a000)',
  bigdiff: 'var(--wks-warning, #e0a000)',
  done: 'var(--wks-success, #3fb950)',
};
const KIND_VISUAL_LABEL: Record<AttentionKind, string> = {
  approval: 'Needs approval',
  question: 'Question',
  error: 'Error',
  stuck: 'Stuck',
  bigdiff: 'Review changes',
  done: 'Finished',
};

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
  /** Toggle the advanced cross-agent overview. */
  onToggleFleet?: () => void;
  /** Current altitude — highlights the overview button when active. */
  viewLevel?: 'fleet' | 'piloting';
  /** Open the remote-control (phone sharing) panel. */
  onOpenRemote?: () => void;
  /** Collapse/expand the sidebar (toggles between the full panel and the rail). */
  onToggleCollapse?: () => void;
  /** Render the compact monogram rail instead of the full panel. */
  collapsed?: boolean;
  /** Toggle the keyboard-shortcuts help overlay (footer "?" button). */
  onToggleHelp?: () => void;
  /** Brief flash on the header when "next attention" found nothing to jump to. */
  noAttentionFlash?: boolean;
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
  onToggleHelp,
  noAttentionFlash,
  collapsed,
}) => {
  // Counts come from the single attention feed (the spine), not a parallel
  // reduction over statusBySession — so the header can never disagree with the
  // Inbox / Overview. needsYou counts approval/question/stuck/error items.
  const { counts, topByAgent } = useAttention();
  const needYouCount = counts.needsYou;
  // Focus mode reduces attention to one compact badge pinned in the rail —
  // "agents need you" must never disappear entirely (UI-mode manifest).
  const { manifest: uiManifest } = useUiMode();
  // "working" still reflects live ambient state (not an attention kind).
  const workingCount = agents.reduce((n, a) => {
    const s = a.sessionId ? statusBySession[a.sessionId] : undefined;
    return n + (s === 'thinking' || s === 'streaming' ? 1 : 0);
  }, 0);
  const [contextMenu, setContextMenu] = useState<{ agentId: string; x: number; y: number } | null>(
    null,
  );
  const [renamingId, setRenamingId] = useState<string | null>(null);
  const [renameValue, setRenameValue] = useState('');
  // Type-to-filter the agent list by name or provider (nav rows always shown).
  const [filter, setFilter] = useState('');

  // Inspecting an agent is deliberate, not ambient: no hover popover. The
  // right-click menu's "Inspect" opens the InspectorCard as a pinned pane.
  const inspectAgent = (agentId: string) => {
    const agent = agents.find((a) => a.id === agentId);
    if (agent?.sessionId && !agent.global)
      requestInspector({ sessionId: agent.sessionId, agentName: agent.name });
  };

  // Per-session derived stats, memoized by the *snapshot object identity* so a
  // tick on one agent's session doesn't recompute deriveSessionStats for every
  // other row. The snapshotBySession map is replaced wholesale on each update,
  // but the unchanged sessions keep their prior snapshot object references, so
  // we reuse the cached stats for those and only recompute the one that moved.
  const statsCacheRef = useRef<
    WeakMap<ClaudeSessionSnapshot, ReturnType<typeof deriveSessionStats>>
  >(new WeakMap());
  const statsBySession = useMemo(() => {
    const cache = statsCacheRef.current;
    const out: Record<string, ReturnType<typeof deriveSessionStats>> = {};
    for (const [sid, snap] of Object.entries(snapshotBySession)) {
      if (!snap) continue;
      let stats = cache.get(snap);
      if (!stats) {
        stats = deriveSessionStats(snap);
        cache.set(snap, stats);
      }
      out[sid] = stats;
    }
    return out;
  }, [snapshotBySession]);

  const commitRename = () => {
    if (renamingId && renameValue.trim()) onRenameAgent(renamingId, renameValue.trim());
    setRenamingId(null);
    setRenameValue('');
  };

  // Shared chrome for both layouts — glass surface, rounded inner corners.
  const surfaceStyle: React.CSSProperties = {
    position: 'absolute',
    top: 0,
    left: 0,
    bottom: 0,
    display: 'flex',
    flexDirection: 'column',
    backgroundColor: 'var(--wks-bg-raised)',
    borderRight: '1px solid var(--wks-border-subtle)',
    borderTopRightRadius: 'var(--wks-radius-lg)',
    borderBottomRightRadius: 'var(--wks-radius-lg)',
    overflow: 'hidden',
    zIndex: 100,
    userSelect: 'none',
    boxSizing: 'border-box',
  };

  // ── Collapsed monogram rail ───────────────────────────────────────────────
  // A 74px rail that keeps every agent reachable with one click. Tiles mirror
  // the full panel's monogram + status-dot vocabulary so the two read as one UI.
  if (collapsed) {
    const railTile = (agent: AgentWorkspace) => {
      const isActive = agent.id === activeAgentId;
      const isGlobal = !!agent.global;
      const isSupervisor = agent.kind === 'supervisor';
      const state = agent.sessionId ? statusBySession[agent.sessionId] : undefined;
      const base = statusVisual(state);
      const top: AttentionItem | undefined = topByAgent.get(agent.id);
      const color = top ? KIND_COLOR[top.kind] : base.color;
      const label = top ? KIND_VISUAL_LABEL[top.kind] : base.label;
      const glyph = top ? KIND_GLYPH[top.kind] : '';
      const working = state === 'thinking' || state === 'streaming';
      const tileLetter = (
        agent.name.match(/[a-z0-9]/i)?.[0] ||
        agent.name.charAt(0) ||
        '?'
      ).toUpperCase();
      return (
        <button
          key={agent.id}
          onClick={() => onSelectAgent(agent.id)}
          onContextMenu={(e) => {
            e.preventDefault();
            if (isGlobal) return;
            setContextMenu({ agentId: agent.id, x: e.clientX, y: e.clientY });
          }}
          title={isGlobal ? 'Overview' : `${agent.name} — ${label}`}
          style={{
            position: 'relative',
            width: 40,
            height: 40,
            flexShrink: 0,
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            borderRadius: 11,
            cursor: 'pointer',
            padding: 0,
            background: isActive ? 'var(--wks-accent-bg)' : 'var(--wks-bg-base)',
            border: `1px solid ${isActive ? 'var(--wks-accent-glow)' : 'var(--wks-border-subtle)'}`,
            boxShadow:
              working && !isGlobal
                ? '0 0 0 1px color-mix(in srgb, var(--wks-busy) 24%, transparent)'
                : 'none',
            transition: 'border-color 0.12s, background 0.12s',
          }}
        >
          {isGlobal ? (
            <span style={{ fontSize: '0.95rem', color: 'var(--wks-text-tertiary)', lineHeight: 1 }}>
              ▦
            </span>
          ) : isSupervisor ? (
            <span style={{ fontSize: '0.9rem', lineHeight: 1 }}>🧭</span>
          ) : (
            <span
              style={{
                fontFamily: 'var(--wks-font-mono)',
                fontSize: '0.9rem',
                fontWeight: 700,
                color: 'var(--wks-text-primary)',
                lineHeight: 1,
              }}
            >
              {tileLetter}
            </span>
          )}
          {!isGlobal &&
            (glyph ? (
              <span
                style={{
                  position: 'absolute',
                  right: -3,
                  bottom: -3,
                  width: 14,
                  height: 14,
                  borderRadius: 'var(--wks-radius-pill)',
                  background: 'var(--wks-bg-raised)',
                  border: '2px solid var(--wks-bg-raised)',
                  display: 'flex',
                  alignItems: 'center',
                  justifyContent: 'center',
                  color,
                  fontSize: '0.55rem',
                  fontWeight: 700,
                  lineHeight: 1,
                  textShadow: `0 0 3px ${color}`,
                }}
              >
                {glyph}
              </span>
            ) : (
              <span
                style={{
                  position: 'absolute',
                  right: -3,
                  bottom: -3,
                  width: 11,
                  height: 11,
                  borderRadius: 'var(--wks-radius-pill)',
                  backgroundColor: color,
                  border: '2px solid var(--wks-bg-raised)',
                  boxShadow: working ? `0 0 4px ${color}` : 'none',
                  animation: working ? 'wks-pulse 1.6s ease-in-out infinite' : 'none',
                }}
              />
            ))}
        </button>
      );
    };

    return (
      <div
        style={{
          ...surfaceStyle,
          width: `${SIDEBAR_RAIL_WIDTH}px`,
          alignItems: 'center',
          paddingTop: '8px',
          gap: '8px',
        }}
      >
        <div
          title="Workspacer"
          style={{
            width: 38,
            height: 38,
            flexShrink: 0,
            marginBottom: 2,
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            background: 'var(--wks-bg-base)',
            border: '1px solid var(--wks-border-subtle)',
            borderRadius: 11,
          }}
        >
          <BrandMark size={19} />
        </div>

        <button
          onClick={onToggleCollapse}
          title="Expand sidebar (Ctrl+B)"
          style={{
            width: 28,
            height: 28,
            flexShrink: 0,
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            border: 'none',
            borderRadius: 7,
            cursor: 'pointer',
            background: 'transparent',
            color: 'var(--wks-text-faint)',
          }}
        >
          <ChevronRight size={16} strokeWidth={2} />
        </button>

        <div
          style={{
            flex: 1,
            overflowY: 'auto',
            overflowX: 'hidden',
            display: 'flex',
            flexDirection: 'column',
            alignItems: 'center',
            gap: '8px',
            padding: '2px 0',
            width: '100%',
          }}
        >
          {agents.map(railTile)}
        </div>

        <button
          onClick={onSpawnAgent}
          title="Spawn a new agent (Ctrl+Shift+N)"
          style={{
            width: 40,
            height: 40,
            flexShrink: 0,
            margin: '4px 0 10px',
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            border: 'none',
            borderRadius: 11,
            cursor: 'pointer',
            background: 'var(--wks-accent)',
            color: 'var(--wks-bg-raised)',
          }}
        >
          <Plus size={18} strokeWidth={2.5} />
        </button>

        {/* Compact needs-you badge (focus mode's attention surface) — pinned
            above the hub dot. Click jumps to the next agent blocked on you. */}
        {uiManifest.attention === 'badge' && needYouCount > 0 && (
          <button
            onClick={onJumpToAttention}
            title={`${needYouCount} agent${needYouCount === 1 ? '' : 's'} need you — click to jump`}
            style={{
              ...pillStyle('var(--wks-warning, #e0a000)'),
              border: 'none',
              cursor: 'pointer',
              flexShrink: 0,
              margin: '0 0 6px',
            }}
          >
            <span style={dotStyle('var(--wks-warning, #e0a000)', true)} />
            {needYouCount}
          </button>
        )}

        <HubStatus onOpenRemote={onOpenRemote} compact />

        {contextMenu && (
          <ContextMenu
            x={contextMenu.x}
            y={contextMenu.y}
            minWidth={140}
            onClose={() => setContextMenu(null)}
          >
            <ContextMenuItem
              label="Inspect"
              onClick={() => {
                const id = contextMenu.agentId;
                setContextMenu(null);
                inspectAgent(id);
              }}
            />
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
  }

  return (
    <div
      style={{
        position: 'absolute',
        top: 0,
        left: 0,
        bottom: 0,
        width: `${SIDEBAR_WIDTH}px`,
        display: 'flex',
        flexDirection: 'column',
        paddingTop: '6px',
        gap: '2px',
        backgroundColor: 'var(--wks-bg-raised)',
        borderRight: '1px solid var(--wks-border-subtle)',
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
      }}
    >
      {/* Brand header — the { ▮ } mark + work{spacer} wordmark. */}
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 10,
          padding: '4px 14px 10px 16px',
        }}
      >
        <span
          style={{
            width: 30,
            height: 30,
            flexShrink: 0,
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            background: 'var(--wks-bg-base)',
            border: '1px solid var(--wks-border-subtle)',
            borderRadius: 9,
          }}
        >
          <BrandMark size={17} blink />
        </span>
        <Wordmark size={17} />
      </div>

      <div
        style={{
          padding: '6px 14px 10px 16px',
          fontSize: '0.6rem',
          fontWeight: 700,
          letterSpacing: '0.08em',
          textTransform: 'uppercase',
          color: 'var(--wks-text-faint)',
          display: 'flex',
          alignItems: 'center',
          gap: 6,
        }}
      >
        <span>Agents</span>
        {noAttentionFlash && (
          <span style={pillStyle('var(--wks-success, #3fb950)')}>all clear</span>
        )}
        {!noAttentionFlash && needYouCount > 0 && (
          <span
            onClick={onOpenInbox ?? onJumpToAttention}
            title="Open the Triage Inbox"
            style={{
              ...pillStyle('var(--wks-warning, #e0a000)'),
              cursor: (onOpenInbox ?? onJumpToAttention) ? 'pointer' : 'default',
            }}
          >
            <span style={dotStyle('var(--wks-warning, #e0a000)', true)} />
            {needYouCount} need you
          </span>
        )}
        {!noAttentionFlash && needYouCount === 0 && workingCount > 0 && (
          <span style={pillStyle('var(--wks-busy, var(--wks-accent-text, #4a9eff))')}>
            <IconWorking size={12} strokeWidth={2.2} accent="currentColor" />
            {workingCount} working
          </span>
        )}
        <div style={{ flex: 1 }} />
        {onToggleCollapse && (
          <button
            onClick={onToggleCollapse}
            title="Collapse sidebar (Ctrl+B)"
            style={{
              width: 20,
              height: 20,
              padding: 0,
              flexShrink: 0,
              display: 'flex',
              alignItems: 'center',
              justifyContent: 'center',
              border: 'none',
              borderRadius: 'var(--wks-radius-sm)',
              cursor: 'pointer',
              background: 'transparent',
              color: 'var(--wks-text-faint)',
              fontSize: '0.9rem',
              lineHeight: 1,
            }}
          >
            <ChevronLeft size={15} strokeWidth={2} />
          </button>
        )}
      </div>

      {/* Primary attention surface plus the advanced cross-agent overview. */}
      <div
        style={{
          display: 'flex',
          gap: 4,
          margin: '2px 12px 10px',
          padding: 4,
          background: 'var(--wks-bg-base)',
          border: '1px solid var(--wks-border-subtle)',
          borderRadius: 'var(--wks-radius-lg)',
        }}
      >
        <button
          onClick={onOpenInbox}
          title="Triage Inbox (Ctrl+Shift+A)"
          style={segBtnStyle(false)}
        >
          <IconInbox size={15} strokeWidth={2} />
          <span>Inbox</span>
          {counts.total > 0 && (
            // Amber when something genuinely needs you; muted green when the inbox
            // only holds finished / review items (nothing blocking).
            <span
              style={{
                marginLeft: 'auto',
                minWidth: 16,
                height: 16,
                padding: '0 4px',
                borderRadius: 'var(--wks-radius-md)',
                display: 'inline-flex',
                alignItems: 'center',
                justifyContent: 'center',
                background:
                  needYouCount > 0 ? 'var(--wks-warning, #e0a000)' : 'var(--wks-success, #3fb950)',
                color: 'var(--wks-bg-base, #1a1a1a)',
                fontFamily: 'var(--wks-font-mono)',
                fontSize: '0.6rem',
                fontWeight: 700,
                opacity: needYouCount > 0 ? 1 : 0.85,
              }}
            >
              {needYouCount > 0 ? needYouCount : counts.total}
            </span>
          )}
        </button>
        <button
          onClick={onToggleFleet}
          title="Agent Overview (Ctrl+Shift+F)"
          style={segBtnStyle(viewLevel === 'fleet')}
        >
          <IconOverview size={15} strokeWidth={2} />
          Overview
        </button>
      </div>

      {/* Filter — only worth showing once there's a handful of agents. */}
      {agents.filter((a) => !a.global).length > 4 && (
        <div style={{ padding: '2px 12px 6px' }}>
          <input
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
            placeholder="Filter agents…"
            spellCheck={false}
            style={{
              width: '100%',
              boxSizing: 'border-box',
              fontSize: '0.72rem',
              fontFamily: 'inherit',
              padding: '5px 9px',
              borderRadius: 'var(--wks-radius-md, 6px)',
              border: '1px solid var(--wks-border-input, var(--wks-border-subtle))',
              background: 'var(--wks-bg-base)',
              color: 'var(--wks-text-primary)',
            }}
          />
        </div>
      )}

      <div
        style={{
          flex: 1,
          overflowY: 'auto',
          display: 'flex',
          flexDirection: 'column',
          gap: '7px',
          padding: '2px 0',
        }}
      >
        {agents.length === 0 && (
          <div
            style={{
              padding: '8px 12px',
              fontSize: '0.7rem',
              color: 'var(--wks-text-faint)',
              lineHeight: 1.5,
            }}
          >
            No agents yet. Spawn one to start a Claude Code session.
          </div>
        )}

        {/* Render agents with nested children beneath their parent.
            Strategy: any agent with a parentId that resolves to a known agent's
            id is rendered indented below that parent. Top-level agents are those
            with no parentId, or whose parentId doesn't resolve (fallback so
            nothing disappears). Children are NOT rendered again at top level. */}
        {(() => {
          // Apply the name/provider filter (nav rows like Overview always shown).
          const q = filter.trim().toLowerCase();
          const shown = q
            ? agents.filter(
                (a) =>
                  a.global ||
                  a.name.toLowerCase().includes(q) ||
                  (a.provider ?? 'claude').toLowerCase().includes(q),
              )
            : agents;
          // Build a set of all known agent ids for fast parent-resolution checks.
          const agentIds = new Set(shown.map((a) => a.id));
          // Build a lookup: parentId → child agents (any kind with a resolvable parentId).
          const childrenByParent = new Map<string, typeof agents>();
          const topLevel: typeof agents = [];
          for (const agent of shown) {
            if (agent.parentId && agentIds.has(agent.parentId)) {
              const bucket = childrenByParent.get(agent.parentId) ?? [];
              bucket.push(agent);
              childrenByParent.set(agent.parentId, bucket);
            } else {
              topLevel.push(agent);
            }
          }

          const renderAgentRow = (agent: (typeof agents)[0], indent?: boolean) => {
            const isActive = agent.id === activeAgentId;
            const isGlobal = !!agent.global;
            const isSupervisor = agent.kind === 'supervisor';
            const state = agent.sessionId ? statusBySession[agent.sessionId] : undefined;
            const base = statusVisual(state);
            // The agent's most-urgent open attention item (if any) tints the dot
            // and adds a tiny kind glyph, so the row collapses to ~5 readable states.
            const top: AttentionItem | undefined = topByAgent.get(agent.id);
            const color = top ? KIND_COLOR[top.kind] : base.color;
            const label = top ? KIND_VISUAL_LABEL[top.kind] : base.label;
            const glyph = top ? KIND_GLYPH[top.kind] : '';
            const isRenaming = renamingId === agent.id;
            const snap = agent.sessionId ? snapshotBySession[agent.sessionId] : undefined;
            const stats =
              (agent.sessionId && statsBySession[agent.sessionId]) || deriveSessionStats(snap);
            const hasCtx = stats.ctxPct !== undefined;
            const ctxFrac = hasCtx ? Math.min(1, stats.ctxPct! / 100) : 0;
            const usageTip = hasCtx
              ? `\n${Math.round(stats.ctxPct!)}% context${stats.tokens !== undefined ? ` · ${fmtTokens(stats.tokens)} tok` : ''}${stats.costUSD !== undefined ? ` · ${fmtUSD(stats.costUSD)}` : ''}${stats.model ? ` · ${stats.model}` : ''}`
              : '';
            const working = state === 'thinking' || state === 'streaming';
            const statusText = isGlobal
              ? 'Dashboards & plugins'
              : agent.sessionId
                ? label
                : 'Stopped — click to respawn';

            return (
              <button
                key={agent.id}
                onClick={() => onSelectAgent(agent.id)}
                onContextMenu={(e) => {
                  e.preventDefault();
                  if (isGlobal) return; // Overview can't be renamed/terminated
                  setContextMenu({ agentId: agent.id, x: e.clientX, y: e.clientY });
                }}
                onMouseEnter={(e) => {
                  if (!isActive)
                    (e.currentTarget as HTMLElement).style.backgroundColor =
                      'var(--wks-bg-elevated)';
                }}
                onMouseLeave={(e) => {
                  if (!isActive)
                    (e.currentTarget as HTMLElement).style.backgroundColor = 'transparent';
                }}
                style={{
                  position: 'relative',
                  width: indent ? 'calc(100% - 36px)' : 'calc(100% - 24px)',
                  margin: indent ? '0 12px 0 24px' : '0 12px',
                  padding: '11px 13px',
                  display: 'block',
                  borderRadius: 'var(--wks-radius-lg)',
                  cursor: 'pointer',
                  fontFamily: 'inherit',
                  backgroundColor: 'transparent',
                  color: 'var(--wks-text-secondary)',
                  border: '1px solid transparent',
                  textAlign: 'left',
                  boxSizing: 'border-box',
                  transition: 'background-color 0.12s, box-shadow 0.2s',
                  // Working agents get a faint "busy" ring (mockup), independent of
                  // selection — subtle so inactive rows don't read as glowing.
                  boxShadow:
                    working && !isGlobal
                      ? '0 0 0 1px color-mix(in srgb, var(--wks-busy) 20%, transparent)'
                      : 'none',
                  opacity: indent ? 0.9 : 1,
                }}
                title={
                  isGlobal
                    ? 'Overview — cross-agent dashboards & plugin panes'
                    : `${agent.name} — ${label}\n${agent.cwd}${usageTip}`
                }
              >
                {isActive && (
                  <>
                    <span
                      style={{
                        position: 'absolute',
                        inset: 0,
                        borderRadius: 'var(--wks-radius-lg)',
                        background: 'var(--wks-accent-bg)',
                        border: '1px solid var(--wks-accent-glow)',
                        pointerEvents: 'none',
                      }}
                    />
                    <span
                      style={{
                        position: 'absolute',
                        left: 0,
                        top: 13,
                        bottom: 13,
                        width: 3,
                        borderRadius: 'var(--wks-radius-pill)',
                        background: 'var(--wks-accent)',
                      }}
                    />
                  </>
                )}
                <span style={{ position: 'relative', display: 'block' }}>
                  <span style={{ display: 'flex', alignItems: 'center', gap: 11 }}>
                    {/* Monogram tile with corner status badge */}
                    <span
                      style={{
                        position: 'relative',
                        width: 30,
                        height: 30,
                        flexShrink: 0,
                        borderRadius: 9,
                        background: 'var(--wks-bg-base)',
                        border: '1px solid var(--wks-border-subtle)',
                        display: 'flex',
                        alignItems: 'center',
                        justifyContent: 'center',
                      }}
                    >
                      {isGlobal ? (
                        <span
                          style={{
                            fontSize: '0.85rem',
                            color: 'var(--wks-text-tertiary)',
                            lineHeight: 1,
                          }}
                        >
                          ▦
                        </span>
                      ) : isSupervisor ? (
                        <span style={{ fontSize: '0.78rem', lineHeight: 1 }}>🧭</span>
                      ) : (
                        // Provider logo so Claude / Codex / OpenCode / Pi agents are
                        // distinguishable at a glance (was a generic name initial).
                        <AgentLogo
                          provider={agent.provider ?? 'claude'}
                          size={17}
                          style={{ color: 'var(--wks-text-primary)', lineHeight: 1 }}
                        />
                      )}
                      {!isGlobal &&
                        (glyph ? (
                          <span
                            title={label}
                            style={{
                              position: 'absolute',
                              right: -3,
                              bottom: -3,
                              width: 13,
                              height: 13,
                              borderRadius: 'var(--wks-radius-pill)',
                              background: 'var(--wks-bg-raised)',
                              border: '2px solid var(--wks-bg-raised)',
                              display: 'flex',
                              alignItems: 'center',
                              justifyContent: 'center',
                              color,
                              fontSize: '0.5rem',
                              fontWeight: 700,
                              lineHeight: 1,
                              textShadow: `0 0 3px ${color}`,
                            }}
                          >
                            {glyph}
                          </span>
                        ) : (
                          <span
                            style={{
                              position: 'absolute',
                              right: -3,
                              bottom: -3,
                              width: 11,
                              height: 11,
                              borderRadius: 'var(--wks-radius-pill)',
                              backgroundColor: color,
                              border: '2px solid var(--wks-bg-raised)',
                              boxShadow: working ? `0 0 4px ${color}` : 'none',
                              animation: working ? 'wks-pulse 1.6s ease-in-out infinite' : 'none',
                            }}
                          />
                        ))}
                    </span>
                    {/* Name + percentage / status */}
                    <span style={{ minWidth: 0, flex: 1 }}>
                      <span style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                        {isRenaming ? (
                          <input
                            autoFocus
                            value={renameValue}
                            onChange={(e) => setRenameValue(e.target.value)}
                            onBlur={commitRename}
                            onKeyDown={(e) => {
                              if (e.key === 'Enter') commitRename();
                              if (e.key === 'Escape') {
                                setRenamingId(null);
                                setRenameValue('');
                              }
                              e.stopPropagation();
                            }}
                            onClick={(e) => e.stopPropagation()}
                            style={{
                              flex: 1,
                              minWidth: 0,
                              fontSize: '0.8rem',
                              fontFamily: 'inherit',
                              background: 'var(--wks-bg-base)',
                              color: 'var(--wks-text-primary)',
                              border: '1px solid var(--wks-accent)',
                              borderRadius: 4,
                              padding: '1px 4px',
                              boxSizing: 'border-box',
                            }}
                          />
                        ) : (
                          <span
                            style={{
                              flex: 1,
                              minWidth: 0,
                              fontSize: indent ? '0.8rem' : '0.85rem',
                              fontWeight: 600,
                              color: 'var(--wks-text-primary)',
                              overflow: 'hidden',
                              textOverflow: 'ellipsis',
                              whiteSpace: 'nowrap',
                              lineHeight: 1.3,
                            }}
                          >
                            {agent.name}
                          </span>
                        )}
                        {hasCtx && (
                          <span
                            style={{
                              flexShrink: 0,
                              fontFamily: 'var(--wks-font-mono)',
                              fontSize: '0.75rem',
                              fontWeight: 700,
                              color: ctxColor(ctxFrac * 100),
                              fontVariantNumeric: 'tabular-nums',
                            }}
                          >
                            {Math.round(ctxFrac * 100)}%
                          </span>
                        )}
                      </span>
                      <span
                        style={{
                          display: 'block',
                          marginTop: 3,
                          fontFamily: 'var(--wks-font-mono)',
                          fontSize: '0.68rem',
                          color: isGlobal ? 'var(--wks-text-faint)' : color,
                          overflow: 'hidden',
                          textOverflow: 'ellipsis',
                          whiteSpace: 'nowrap',
                        }}
                      >
                        {statusText}
                      </span>
                    </span>
                  </span>
                  {hasCtx && (
                    <span
                      style={{
                        display: 'block',
                        height: 3,
                        borderRadius: 'var(--wks-radius-pill)',
                        marginTop: 11,
                        background: 'var(--wks-bg-base)',
                        overflow: 'hidden',
                      }}
                    >
                      <span
                        style={{
                          display: 'block',
                          height: '100%',
                          borderRadius: 'var(--wks-radius-pill)',
                          width: `${Math.max(2, ctxFrac * 100)}%`,
                          background: ctxColor(ctxFrac * 100),
                        }}
                      />
                    </span>
                  )}
                </span>
              </button>
            );
          };

          // Needy-first ordering — same rule as the Fleet Deck (via
          // `agentAttentionScore` + the shared attention feed) so the agent
          // blocked on you rises to the top instead of scrolling off. Global/nav
          // rows (Overview) stay pinned above the fleet. V8 sort is stable, so
          // equal-priority agents keep their existing order.
          topLevel.sort((a, b) => {
            const ga = a.global ? 1 : 0;
            const gb = b.global ? 1 : 0;
            if (ga !== gb) return gb - ga;
            const sa = agentAttentionScore(
              a.sessionId ? statusBySession[a.sessionId] : undefined,
              topByAgent.get(a.id)?.priority ?? 0,
            );
            const sb = agentAttentionScore(
              b.sessionId ? statusBySession[b.sessionId] : undefined,
              topByAgent.get(b.id)?.priority ?? 0,
            );
            return sb - sa;
          });

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

      {/* Spawn button — solid affordance with an accent icon chip + kbd hint. */}
      <button
        onClick={onSpawnAgent}
        style={{
          width: 'calc(100% - 24px)',
          margin: '4px 12px 10px',
          padding: '8px',
          display: 'flex',
          alignItems: 'center',
          gap: '11px',
          border: '1px solid var(--wks-border-subtle)',
          borderRadius: 'var(--wks-radius-lg)',
          cursor: 'pointer',
          fontSize: '0.8rem',
          fontWeight: 600,
          fontFamily: 'inherit',
          backgroundColor: 'var(--wks-bg-elevated)',
          color: 'var(--wks-text-primary)',
          textAlign: 'left',
          boxSizing: 'border-box',
          transition: 'border-color 0.14s',
        }}
        onMouseEnter={(e) => {
          (e.currentTarget as HTMLElement).style.borderColor = 'var(--wks-accent)';
        }}
        onMouseLeave={(e) => {
          (e.currentTarget as HTMLElement).style.borderColor = 'var(--wks-border-subtle)';
        }}
        title="Spawn a new agent (Ctrl+Shift+N)"
      >
        <span
          style={{
            flexShrink: 0,
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            width: 28,
            height: 28,
            borderRadius: 'var(--wks-radius-md)',
            background: 'var(--wks-accent)',
            color: 'var(--wks-bg-raised)',
          }}
        >
          <Plus size={16} strokeWidth={2.5} />
        </span>
        <span
          style={{
            display: 'flex',
            flexDirection: 'column',
            alignItems: 'flex-start',
            lineHeight: 1.25,
            minWidth: 0,
          }}
        >
          <span style={{ whiteSpace: 'nowrap' }}>Spawn agent</span>
          <span
            style={{
              fontFamily: 'var(--wks-font-mono)',
              fontSize: '0.62rem',
              fontWeight: 500,
              color: 'var(--wks-text-faint)',
              whiteSpace: 'nowrap',
            }}
          >
            Ctrl+Shift+N
          </span>
        </span>
      </button>

      {/* Footer: persistent help affordance so onboarding guidance is always
          re-enterable (re-uses the existing keyboard-shortcuts overlay). */}
      {onToggleHelp && (
        <button
          onClick={onToggleHelp}
          title="Keyboard shortcuts & help"
          style={{
            width: 'calc(100% - 8px)',
            margin: '0 4px 6px',
            padding: '6px 8px',
            display: 'flex',
            alignItems: 'center',
            gap: 8,
            border: '1px solid var(--wks-glass-border)',
            borderRadius: 6,
            cursor: 'pointer',
            fontSize: '0.72rem',
            fontFamily: 'inherit',
            fontWeight: 600,
            background: 'transparent',
            color: 'var(--wks-text-muted)',
            textAlign: 'left',
            boxSizing: 'border-box',
          }}
        >
          <span style={{ width: 8, display: 'inline-flex', justifyContent: 'center' }}>
            <HelpCircle size={13} strokeWidth={2} />
          </span>
          <span>Help &amp; shortcuts</span>
        </button>
      )}

      {/* Hub bus status — sits in-flow at the bottom of the sidebar */}
      <HubStatus onOpenRemote={onOpenRemote} />

      {/* Context menu */}
      {contextMenu && (
        <ContextMenu
          x={contextMenu.x}
          y={contextMenu.y}
          minWidth={140}
          onClose={() => setContextMenu(null)}
        >
          <ContextMenuItem
            label="Inspect"
            onClick={() => {
              const id = contextMenu.agentId;
              setContextMenu(null);
              inspectAgent(id);
            }}
          />
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

/** A flush button inside the Inbox/Fleet segmented track. */
function segBtnStyle(active: boolean): React.CSSProperties {
  return {
    flex: 1,
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'center',
    gap: 7,
    padding: '6px 10px',
    borderRadius: 'var(--wks-radius-md)',
    cursor: 'pointer',
    fontSize: '0.8rem',
    fontFamily: 'inherit',
    fontWeight: 600,
    border: 'none',
    background: active ? 'var(--wks-bg-elevated)' : 'transparent',
    color: active ? 'var(--wks-text-primary)' : 'var(--wks-text-tertiary)',
    boxShadow: active ? '0 1px 2px var(--wks-shadow)' : 'none',
    boxSizing: 'border-box',
  };
}

/** Header status pill (e.g. "1 working") — soft chip tinted to match its own
 *  status color (green/amber), with a live dot. */
function pillStyle(color: string): React.CSSProperties {
  return {
    display: 'inline-flex',
    alignItems: 'center',
    gap: 6,
    padding: '2px 8px',
    borderRadius: 'var(--wks-radius-pill)',
    background: `color-mix(in srgb, ${color} 16%, transparent)`,
    color,
    fontFamily: 'var(--wks-font-mono)',
    fontSize: '0.62rem',
    fontWeight: 600,
    letterSpacing: 0,
    textTransform: 'none',
    whiteSpace: 'nowrap',
  };
}

function dotStyle(color: string, pulse: boolean): React.CSSProperties {
  return {
    width: 6,
    height: 6,
    borderRadius: '50%',
    flexShrink: 0,
    background: color,
    animation: pulse ? 'wks-pulse 1.8s ease-in-out infinite' : 'none',
  };
}

export default SideBar;
