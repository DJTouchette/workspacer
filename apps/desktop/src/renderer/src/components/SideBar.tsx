import React, { useState, useRef, useMemo, useEffect, useCallback } from 'react';
import { Plus, ChevronLeft, ChevronRight, ChevronDown, LayoutGrid, Compass } from 'lucide-react';
import { BrandMark, Wordmark } from './Brand';
import { AgentWorkspace, AgentProvider } from '../types/pane';
import type { RecentAgentSession } from '../../../main/shared/ipcTypes';
import type { SessionAmbientState, ClaudeSessionSnapshot } from '../types/claudeSession';
import type { AttentionItem, AttentionKind } from '../types/attention';
import { deriveSessionStats, fmtTokens, fmtUSD, ctxColor, planProgress } from '../lib/sessionStats';
import { ensureKeyframes } from './claude-shared';
import { collectRecentActivity, type ActivityLine } from '../lib/agentActivityLog';
import { recentSessionLabel } from '../lib/recentSessionFilter';
import { shortModelLabel } from '../lib/modelLabel';
import { agentAttentionScore } from '../lib/attentionRouter';
import { AgentLogo } from './agentLogos';
import { requestInspector } from '../lib/watchBus';
import { useAttention } from '../contexts/AttentionContext';
import { useUiMode } from '../hooks/useUiMode';
import HubStatus from './HubStatus';
import { ContextMenu, ContextMenuItem } from './ContextMenu';

export const SIDEBAR_WIDTH = 296;
/** Width of the collapsed monogram rail (desktop). */
export const SIDEBAR_RAIL_WIDTH = 74;

/** Ambient state (or `undefined` = stopped) → status dot color + label. */
function statusVisual(state: SessionAmbientState | undefined): { color: string; label: string } {
  switch (state) {
    case 'waiting_approval':
      return { color: 'var(--wks-warning)', label: 'Needs approval' };
    case 'waiting_input':
      return { color: 'var(--wks-warning)', label: 'Waiting for input' };
    case 'thinking':
      return { color: 'var(--wks-busy)', label: 'Thinking' };
    case 'streaming':
      return { color: 'var(--wks-busy)', label: 'Working' };
    case 'background':
      return { color: 'var(--wks-busy)', label: 'Background work' };
    case 'idle':
      return { color: 'var(--wks-success)', label: 'Idle' };
    default:
      return { color: 'var(--wks-text-faint)', label: 'Stopped' };
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
  approval: 'var(--wks-warning)',
  question: 'var(--wks-accent)',
  error: 'var(--wks-error)',
  stuck: 'var(--wks-warning)',
  bigdiff: 'var(--wks-warning)',
  done: 'var(--wks-success)',
};
const KIND_VISUAL_LABEL: Record<AttentionKind, string> = {
  approval: 'Needs approval',
  question: 'Question',
  error: 'Error',
  stuck: 'Stuck',
  bigdiff: 'Review changes',
  done: 'Finished',
};

// ── Live-feed cards (sidebar spec 2a) ────────────────────────────────────────

/** Attention kinds that put a card in the amber "waiting on you" state. Mirrors
 *  AttentionContext's NEEDS_KINDS — done/bigdiff are review items, not blocks. */
const WAITING_KINDS: ReadonlySet<AttentionKind> = new Set<AttentionKind>([
  'approval',
  'question',
  'stuck',
  'error',
]);

/** Provider identity hue — tints the card's chip fill and working spinner.
 *  Brand-ish constants (like ClaudeLogo's clay fill), not theme tokens. */
const PROVIDER_HUE: Record<AgentProvider, string> = {
  claude: '#e67e80',
  codex: '#7fbbb3',
  opencode: '#d699b6',
  pi: '#83c092',
};

/** A finished agent's card collapses into the EARLIER list after this long. */
const EARLIER_AFTER_MS = 3_600_000;

/** Compact relative age for card headers: 45s → "45s", then 2m / 3h / 2d. */
function relTime(ms: number): string {
  const s = Math.max(0, Math.round(ms / 1000));
  if (s < 60) return `${s}s`;
  const m = Math.round(s / 60);
  if (m < 60) return `${m}m`;
  const h = Math.round(m / 60);
  if (h < 24) return `${h}h`;
  return `${Math.round(h / 24)}d`;
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
  /** Toggle the cross-agent fleet surface. */
  onToggleFleet?: () => void;
  /** Current altitude — highlights the Fleet button when active. */
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
  /** Daemon sessions not in the layout — the RECENT list (already filtered). */
  recentSessions?: RecentAgentSession[];
  /** Bring a recent session back as an agent (spawn with --resume). */
  onResumeSession?: (session: RecentAgentSession) => void;
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
  recentSessions,
  onResumeSession,
}) => {
  // Counts come from the single attention feed (the spine) — the rail's
  // "needs you" badge can never disagree with the cards' waiting states.
  const { counts, topByAgent, approve } = useAttention();
  const needYouCount = counts.needsYou;
  // Focus mode reduces attention to one compact badge pinned in the rail —
  // "agents need you" must never disappear entirely (UI-mode manifest).
  const { manifest: uiManifest } = useUiMode();
  const [contextMenu, setContextMenu] = useState<{ agentId: string; x: number; y: number } | null>(
    null,
  );
  const [renamingId, setRenamingId] = useState<string | null>(null);
  const [renameValue, setRenameValue] = useState('');
  // Type-to-filter the agent list by name or provider (nav rows always shown).
  const [filter, setFilter] = useState('');
  // Collapsed state for the EARLIER / RECENT sub-sections. Deliberately NOT
  // persisted: every boot starts fully expanded, and collapsing only tucks a
  // section away for the current session.
  const [collapsedSections, setCollapsedSections] = useState<Record<string, boolean>>({});
  const toggleSection = useCallback((key: string) => {
    setCollapsedSections((prev) => ({ ...prev, [key]: !prev[key] }));
  }, []);

  // Clickable section heading (EARLIER / RECENT) that collapses its rows. A
  // chevron rotates and the item count rides on the right so a collapsed
  // section still tells you how much it's hiding. When collapsed, the heading
  // sinks to the bottom of the feed (flex `order` sorts it below the expanded
  // content; `marginTop: auto` pins it to the bottom with the empty space
  // above) so tucked-away sections get out of the way. `order` keeps a
  // collapsed EARLIER above a collapsed RECENT.
  const renderSectionHeading = (key: string, label: string, count: number, order: number) => {
    const collapsed = !!collapsedSections[key];
    return (
      <div
        key={`${key}-heading`}
        role="button"
        tabIndex={0}
        onClick={() => toggleSection(key)}
        onKeyDown={(e) => {
          if (e.key === 'Enter' || e.key === ' ') {
            e.preventDefault();
            toggleSection(key);
          }
        }}
        aria-expanded={!collapsed}
        title={collapsed ? `Show ${label.toLowerCase()}` : `Hide ${label.toLowerCase()}`}
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 4,
          padding: '10px 16px 2px',
          fontSize: '0.6rem',
          fontWeight: 700,
          letterSpacing: '0.12em',
          color: 'var(--wks-text-faint)',
          cursor: 'pointer',
          userSelect: 'none',
          ...(collapsed ? { order, marginTop: 'auto' } : null),
        }}
        onMouseEnter={(e) => {
          (e.currentTarget as HTMLElement).style.color = 'var(--wks-text-secondary)';
        }}
        onMouseLeave={(e) => {
          (e.currentTarget as HTMLElement).style.color = 'var(--wks-text-faint)';
        }}
      >
        <ChevronDown
          size={11}
          style={{
            flexShrink: 0,
            transform: collapsed ? 'rotate(-90deg)' : 'none',
            transition: 'transform 0.12s',
          }}
        />
        <span>{label}</span>
        <span style={{ marginLeft: 'auto', opacity: 0.7 }}>{count}</span>
      </div>
    );
  };
  // The pinned global workspace — reached via the brand header, not a feed row.
  const overviewAgent = agents.find((a) => a.global);

  // claudeSpinner keyframes for the working card's provider-tinted status ring.
  useEffect(() => {
    ensureKeyframes();
  }, []);
  // Relative "2m / 41m" ages tick on a coarse timer — working cards re-render on
  // every snapshot anyway; this keeps quiet done-cards from freezing at "now".
  const [, setAgeTick] = useState(0);
  useEffect(() => {
    const t = window.setInterval(() => setAgeTick((n) => n + 1), 30_000);
    return () => window.clearInterval(t);
  }, []);

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

  // ── Collapsed rail ────────────────────────────────────────────────────────
  // A 74px rail that keeps every agent reachable with one click. Tiles mirror
  // the full panel's provider-logo + status-dot vocabulary so an agent reads as
  // the same mark whether the sidebar is expanded or collapsed.
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
      const working = state === 'thinking' || state === 'streaming' || state === 'background';
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
            <LayoutGrid
              size={16}
              strokeWidth={1.75}
              style={{ color: 'var(--wks-text-tertiary)' }}
            />
          ) : isSupervisor ? (
            <Compass size={16} strokeWidth={1.75} style={{ color: 'var(--wks-text-primary)' }} />
          ) : (
            // Provider logo — same vocabulary as the expanded panel, so an agent
            // reads as the same Claude / Codex / OpenCode mark in either state.
            <AgentLogo
              provider={agent.provider ?? 'claude'}
              size={19}
              style={{ color: 'var(--wks-text-primary)', lineHeight: 1 }}
            />
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
                  fontSize: '0.6rem',
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
              ...pillStyle('var(--wks-warning)'),
              border: 'none',
              cursor: 'pointer',
              flexShrink: 0,
              margin: '0 0 6px',
            }}
          >
            <span style={dotStyle('var(--wks-warning)', true)} />
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
      {/* Brand header — the { ▮ } mark + work{spacer} wordmark IS the way home:
          clicking it opens the Overview workspace (dashboards & plugin panes),
          which no longer has its own row in the feed. Collapse toggle top-right. */}
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 10,
          padding: '4px 12px 10px 16px',
        }}
      >
        <button
          onClick={() => overviewAgent && onSelectAgent(overviewAgent.id)}
          title="Overview — cross-agent dashboards & plugin panes"
          style={{
            display: 'flex',
            alignItems: 'center',
            gap: 10,
            padding: 0,
            border: 'none',
            background: 'transparent',
            cursor: 'pointer',
            fontFamily: 'inherit',
            minWidth: 0,
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
              background:
                overviewAgent && activeAgentId === overviewAgent.id
                  ? 'var(--wks-accent-bg)'
                  : 'var(--wks-bg-base)',
              border: `1px solid ${
                overviewAgent && activeAgentId === overviewAgent.id
                  ? 'var(--wks-accent-glow)'
                  : 'var(--wks-border-subtle)'
              }`,
              borderRadius: 9,
              transition: 'border-color 0.12s, background 0.12s',
            }}
          >
            <BrandMark size={17} blink />
          </span>
          <Wordmark size={17} />
        </button>
        {onToggleCollapse && (
          <button
            onClick={onToggleCollapse}
            title="Collapse sidebar (Ctrl+B)"
            style={{
              marginLeft: 'auto',
              width: 26,
              height: 26,
              padding: 0,
              flexShrink: 0,
              display: 'flex',
              alignItems: 'center',
              justifyContent: 'center',
              border: '1px solid transparent',
              borderRadius: 'var(--wks-radius-md)',
              cursor: 'pointer',
              background: 'transparent',
              color: 'var(--wks-text-faint)',
              transition: 'color 0.12s, border-color 0.12s, background 0.12s',
            }}
            onMouseEnter={(e) => {
              (e.currentTarget as HTMLElement).style.color = 'var(--wks-text-primary)';
              (e.currentTarget as HTMLElement).style.background = 'var(--wks-bg-elevated)';
            }}
            onMouseLeave={(e) => {
              (e.currentTarget as HTMLElement).style.color = 'var(--wks-text-faint)';
              (e.currentTarget as HTMLElement).style.background = 'transparent';
            }}
          >
            <ChevronLeft size={16} strokeWidth={2} />
          </button>
        )}
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
              borderRadius: 'var(--wks-radius-md)',
              border: '1px solid var(--wks-border-input)',
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
          gap: '8px',
          padding: '2px 0',
        }}
      >
        {/* The pinned Overview doesn't count — with no real agents the feed is
            empty (the app shows the Overview workspace) and this hint explains. */}
        {agents.every((a) => a.global) && (
          <div
            style={{
              padding: '8px 16px',
              fontFamily: 'var(--wks-font-mono)',
              fontSize: '0.68rem',
              color: 'var(--wks-text-faint)',
              lineHeight: 1.6,
            }}
          >
            No agents yet. Spawn one to start a session.
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

          const now = Date.now();

          // Live-feed card state (spec 2a): amber "waiting on you" beats green
          // "working"; everything else (idle / stopped) reads as done.
          type CardState = 'waiting' | 'working' | 'done';
          const cardStateOf = (agent: (typeof agents)[0]): CardState => {
            const state = agent.sessionId ? statusBySession[agent.sessionId] : undefined;
            const top = topByAgent.get(agent.id);
            if (
              state === 'waiting_approval' ||
              state === 'waiting_input' ||
              (top && WAITING_KINDS.has(top.kind))
            )
              return 'waiting';
            if (state === 'thinking' || state === 'streaming' || state === 'background')
              return 'working';
            return 'done';
          };

          const renderAgentCard = (agent: (typeof agents)[0], indent?: boolean) => {
            const isActive = agent.id === activeAgentId;
            const isSupervisor = agent.kind === 'supervisor';
            const provider = agent.provider ?? 'claude';
            const hue = PROVIDER_HUE[provider] ?? 'var(--wks-accent)';
            const state = agent.sessionId ? statusBySession[agent.sessionId] : undefined;
            const cardState = cardStateOf(agent);
            const top: AttentionItem | undefined = topByAgent.get(agent.id);
            const snap = agent.sessionId ? snapshotBySession[agent.sessionId] : undefined;
            const stats =
              (agent.sessionId && statsBySession[agent.sessionId]) || deriveSessionStats(snap);
            const model = shortModelLabel(stats.model) || shortModelLabel(agent.model);
            const isRenaming = renamingId === agent.id;
            const hasCtx = stats.ctxPct !== undefined;
            const ctxFrac = hasCtx ? Math.min(1, stats.ctxPct! / 100) : 0;

            // Mini action log — the last few things the agent actually did:
            // tool calls and assistant messages, merged in time order (see
            // collectRecentActivity). A busy card tints its freshest line
            // green; only waiting/stopped add a state line. Never the vague
            // "Working…" — the header spinner already says that much.
            // Depth is state-scaled: busy cards earn 3 lines, resting cards
            // stay short so a full sidebar doesn't read as a wall of logs.
            type LogLine = { text: string; color: string; kind?: ActivityLine['kind'] };
            const pushActivity = (lines: ActivityLine[], color?: string) => {
              for (const line of lines) {
                log.push({
                  text: line.text,
                  color: color ?? 'var(--wks-text-faint)',
                  kind: line.kind,
                });
              }
            };
            const activity = collectRecentActivity(snap, 3);
            const log: LogLine[] = [];
            if (cardState === 'waiting') {
              const what =
                top && WAITING_KINDS.has(top.kind)
                  ? top.title
                  : state === 'waiting_approval'
                    ? 'approve a tool call'
                    : 'your input';
              pushActivity(activity.slice(-2));
              log.push({ text: `Waiting: ${what}`, color: 'var(--wks-warning)' });
            } else if (!agent.sessionId) {
              log.push({ text: 'Stopped — click to respawn', color: 'var(--wks-text-faint)' });
            } else if (cardState === 'working') {
              let lines = activity;
              if (!lines.length) {
                // Nothing observable yet (turn just started) — the active plan
                // step is the only real signal available.
                const step = planProgress(snap?.plan)?.active;
                const stepText = step?.activeForm ?? step?.content;
                if (stepText) lines = [{ text: stepText, at: 0, kind: 'message' }];
              }
              pushActivity(lines.slice(0, -1));
              pushActivity(lines.slice(-1), 'var(--wks-success)');
            } else if (activity.length) {
              // Resting card — the last two things it did, muted.
              pushActivity(activity.slice(-2));
            } else {
              log.push({ text: 'Idle', color: 'var(--wks-text-faint)' });
            }

            const age = snap
              ? now - snap.lastActivity < 60_000
                ? 'now'
                : relTime(now - snap.lastActivity)
              : '';
            const label =
              cardState === 'waiting'
                ? top && WAITING_KINDS.has(top.kind)
                  ? KIND_VISUAL_LABEL[top.kind]
                  : 'Waiting on you'
                : cardState === 'working'
                  ? 'Working'
                  : agent.sessionId
                    ? 'Idle'
                    : 'Stopped — click to respawn';
            const usageTip = hasCtx
              ? `\n${Math.round(stats.ctxPct!)}% context${stats.tokens !== undefined ? ` · ${fmtTokens(stats.tokens)} tok` : ''}${stats.costUSD !== undefined ? ` · ${fmtUSD(stats.costUSD)}` : ''}${stats.model ? ` · ${stats.model}` : ''}`
              : '';

            const borderColor = isActive
              ? 'var(--wks-accent-glow)'
              : cardState === 'working'
                ? 'color-mix(in srgb, var(--wks-success) 32%, transparent)'
                : cardState === 'waiting'
                  ? 'color-mix(in srgb, var(--wks-warning) 48%, transparent)'
                  : 'transparent';
            const dimmed = cardState === 'done' && !isActive;

            return (
              <div
                key={agent.id}
                role="button"
                tabIndex={0}
                onClick={() => onSelectAgent(agent.id)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter' && !isRenaming) onSelectAgent(agent.id);
                }}
                onContextMenu={(e) => {
                  e.preventDefault();
                  setContextMenu({ agentId: agent.id, x: e.clientX, y: e.clientY });
                }}
                onMouseEnter={(e) => {
                  (e.currentTarget as HTMLElement).style.opacity = '1';
                }}
                onMouseLeave={(e) => {
                  (e.currentTarget as HTMLElement).style.opacity = dimmed ? '0.55' : '1';
                }}
                title={`${agent.name} — ${label}\n${agent.cwd}${usageTip}`}
                style={{
                  position: 'relative',
                  width: indent ? 'calc(100% - 36px)' : 'calc(100% - 24px)',
                  margin: indent ? '0 12px 0 24px' : '0 12px',
                  padding: '10px 12px',
                  borderRadius: 'var(--wks-radius-lg)',
                  cursor: 'pointer',
                  boxSizing: 'border-box',
                  backgroundColor: isActive ? 'var(--wks-accent-bg)' : 'var(--wks-bg-elevated)',
                  border: `1px solid ${borderColor}`,
                  boxShadow:
                    cardState === 'waiting'
                      ? '0 0 14px color-mix(in srgb, var(--wks-warning) 9%, transparent)'
                      : 'none',
                  opacity: dimmed ? 0.55 : 1,
                  transition: 'opacity 0.15s, border-color 0.15s, background-color 0.15s',
                }}
              >
                {isActive && (
                  <span
                    style={{
                      position: 'absolute',
                      left: 0,
                      top: 12,
                      bottom: 12,
                      width: 3,
                      borderRadius: 'var(--wks-radius-pill)',
                      background: 'var(--wks-accent)',
                    }}
                  />
                )}

                {/* Header: status glyph + name + relative age */}
                <span style={{ display: 'flex', alignItems: 'center', gap: 8, minWidth: 0 }}>
                  {cardState === 'working' ? (
                    <span
                      style={{
                        width: 11,
                        height: 11,
                        flexShrink: 0,
                        boxSizing: 'border-box',
                        borderRadius: '50%',
                        border: `2px solid ${hue}`,
                        borderTopColor: 'transparent',
                        animation: 'claudeSpinner 1s linear infinite',
                      }}
                    />
                  ) : cardState === 'waiting' ? (
                    <span
                      style={{
                        flexShrink: 0,
                        color: 'var(--wks-warning)',
                        fontSize: '0.6rem',
                        lineHeight: 1,
                        animation: 'wks-pulse 1.4s ease-in-out infinite',
                      }}
                    >
                      ■
                    </span>
                  ) : (
                    <span
                      style={{
                        flexShrink: 0,
                        color: 'var(--wks-text-faint)',
                        fontSize: '0.7rem',
                        lineHeight: 1,
                      }}
                    >
                      {agent.sessionId ? '✓' : '○'}
                    </span>
                  )}
                  {isSupervisor && (
                    <Compass
                      size={12}
                      strokeWidth={2}
                      style={{ flexShrink: 0, color: 'var(--wks-text-secondary)' }}
                    />
                  )}
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
                        fontSize: '0.8rem',
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
                  {age && (
                    <span
                      style={{
                        flexShrink: 0,
                        fontFamily: 'var(--wks-font-mono)',
                        fontSize: '0.62rem',
                        color:
                          cardState === 'waiting' ? 'var(--wks-warning)' : 'var(--wks-text-faint)',
                      }}
                    >
                      {cardState === 'waiting' ? `${age} · paused` : age}
                    </span>
                  )}
                </span>

                {/* Action log — └-style ticker of what the agent just did / is doing */}
                {log.length > 0 && (
                  <span
                    style={{
                      display: 'flex',
                      flexDirection: 'column',
                      gap: 2,
                      marginTop: 7,
                      paddingLeft: 9,
                      borderLeft: `1px solid ${
                        cardState === 'waiting'
                          ? 'color-mix(in srgb, var(--wks-warning) 30%, transparent)'
                          : 'var(--wks-border-subtle)'
                      }`,
                    }}
                  >
                    {log.map((l, i) => (
                      <span
                        key={i}
                        style={{
                          // Tool calls read as code (mono); the agent's own
                          // words read as prose (UI font, italic) so the two
                          // don't blur into one log dump.
                          fontFamily: l.kind === 'message' ? 'inherit' : 'var(--wks-font-mono)',
                          fontStyle: l.kind === 'message' ? 'italic' : 'normal',
                          fontSize: '0.68rem',
                          lineHeight: 1.5,
                          color: l.color,
                          overflow: 'hidden',
                          textOverflow: 'ellipsis',
                          whiteSpace: 'nowrap',
                        }}
                      >
                        {l.text}
                      </span>
                    ))}
                  </span>
                )}

                {/* Context-fill bar — working cards only (mock's progress line) */}
                {cardState === 'working' && hasCtx && (
                  <span
                    style={{
                      display: 'block',
                      height: 3,
                      borderRadius: 'var(--wks-radius-pill)',
                      marginTop: 9,
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

                {/* Footer: provider chip + tokens/cost meta */}
                <span
                  style={{
                    display: 'flex',
                    alignItems: 'center',
                    gap: 6,
                    marginTop: 8,
                    minWidth: 0,
                  }}
                >
                  <span
                    title={provider}
                    style={{
                      display: 'inline-flex',
                      alignItems: 'center',
                      gap: 5,
                      minWidth: 0,
                      overflow: 'hidden',
                      whiteSpace: 'nowrap',
                      textOverflow: 'ellipsis',
                      background: `color-mix(in srgb, ${hue} 12%, transparent)`,
                      color: hue,
                      borderRadius: 4,
                      padding: '2px 7px',
                      fontFamily: 'var(--wks-font-mono)',
                      fontSize: '0.62rem',
                    }}
                  >
                    <AgentLogo provider={provider} size={11} style={{ flexShrink: 0 }} />
                    {model}
                  </span>
                  {(stats.tokens !== undefined || stats.costUSD !== undefined) && (
                    <span
                      style={{
                        marginLeft: 'auto',
                        flexShrink: 0,
                        fontFamily: 'var(--wks-font-mono)',
                        fontSize: '0.62rem',
                        color: 'var(--wks-text-faint)',
                        whiteSpace: 'nowrap',
                      }}
                    >
                      {stats.tokens !== undefined ? `${fmtTokens(stats.tokens)} tok` : ''}
                      {stats.tokens !== undefined && stats.costUSD !== undefined ? ' · ' : ''}
                      {stats.costUSD !== undefined ? fmtUSD(stats.costUSD) : ''}
                    </span>
                  )}
                </span>

                {/* Waiting cards act inline — approve here, or jump in to reply */}
                {cardState === 'waiting' && (
                  <span style={{ display: 'flex', gap: 7, marginTop: 9 }}>
                    {top?.payload.type === 'approval' && (
                      <button
                        onClick={(e) => {
                          e.stopPropagation();
                          approve(top, 'yes');
                        }}
                        title={top.title}
                        style={cardActionStyle(true)}
                      >
                        Approve
                      </button>
                    )}
                    <button
                      onClick={(e) => {
                        e.stopPropagation();
                        onSelectAgent(agent.id);
                      }}
                      style={cardActionStyle(false)}
                    >
                      {top?.payload.type === 'question' ? 'Answer' : 'Reply'}
                    </button>
                  </span>
                )}
              </div>
            );
          };

          // Hour-old finished agents collapse to one quiet line each (spec 04).
          const renderEarlierLine = (agent: (typeof agents)[0]) => {
            const snap = agent.sessionId ? snapshotBySession[agent.sessionId] : undefined;
            const age = snap ? relTime(now - snap.lastActivity) : '';
            // "✓ name · what it finished · 1h" — headline from the last message.
            let headline: string | undefined;
            const turns = snap?.conversation ?? [];
            for (let i = turns.length - 1; i >= 0; i--) {
              if (turns[i].role === 'assistant' && turns[i].content?.trim()) {
                headline = turns[i].content
                  .trim()
                  .split('\n')[0]
                  .replace(/^[#>*\-\s`]+/, '');
                break;
              }
            }
            return (
              <div
                key={agent.id}
                role="button"
                tabIndex={0}
                onClick={() => onSelectAgent(agent.id)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter') onSelectAgent(agent.id);
                }}
                onContextMenu={(e) => {
                  e.preventDefault();
                  setContextMenu({ agentId: agent.id, x: e.clientX, y: e.clientY });
                }}
                onMouseEnter={(e) => {
                  (e.currentTarget as HTMLElement).style.color = 'var(--wks-text-secondary)';
                }}
                onMouseLeave={(e) => {
                  (e.currentTarget as HTMLElement).style.color = 'var(--wks-text-faint)';
                }}
                title={`${agent.name}${agent.sessionId ? '' : ' — stopped, click to respawn'}\n${agent.cwd}`}
                style={{
                  margin: '0 16px',
                  padding: '3px 2px',
                  display: 'flex',
                  alignItems: 'center',
                  gap: 7,
                  minWidth: 0,
                  borderRadius: 'var(--wks-radius-md)',
                  cursor: 'pointer',
                  fontFamily: 'var(--wks-font-mono)',
                  fontSize: '0.66rem',
                  color: 'var(--wks-text-faint)',
                  transition: 'color 0.12s',
                }}
              >
                <span style={{ flexShrink: 0 }}>{agent.sessionId ? '✓' : '○'}</span>
                <span
                  style={{
                    overflow: 'hidden',
                    textOverflow: 'ellipsis',
                    whiteSpace: 'nowrap',
                  }}
                >
                  {agent.name}
                  {headline ? ` · ${headline}` : ''}
                </span>
                {age && <span style={{ marginLeft: 'auto', flexShrink: 0 }}>{age}</span>}
              </div>
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

          // Live-feed grouping (spec 2a): Overview nav row pinned first, then
          // waiting → working → done cards (the sort above), then hour-old
          // finished agents collapsed into a compact EARLIER list.
          const cards: typeof agents = [];
          const earlier: typeof agents = [];
          for (const agent of topLevel) {
            if (agent.global) continue;
            const children = childrenByParent.get(agent.id) ?? [];
            const snap = agent.sessionId ? snapshotBySession[agent.sessionId] : undefined;
            const oldDone =
              cardStateOf(agent) === 'done' &&
              agent.id !== activeAgentId &&
              children.length === 0 &&
              (!agent.sessionId || (snap && now - snap.lastActivity > EARLIER_AFTER_MS));
            (oldDone ? earlier : cards).push(agent);
          }

          const rows: React.ReactNode[] = [];
          for (const agent of cards) {
            rows.push(renderAgentCard(agent, false));
            // Render children indented directly after their parent.
            for (const child of childrenByParent.get(agent.id) ?? []) {
              rows.push(renderAgentCard(child, true));
            }
          }
          const historyRows: React.ReactNode[] = [];
          if (earlier.length > 0) {
            historyRows.push(renderSectionHeading('earlier', 'EARLIER', earlier.length, 2));
            if (!collapsedSections.earlier) {
              for (const agent of earlier) historyRows.push(renderEarlierLine(agent));
            }
          }
          const hasRecent = !!onResumeSession && (recentSessions?.length ?? 0) > 0;
          return (
            <>
              {rows}
              {/* History dock: EARLIER + RECENT pinned to the feed's bottom
                  (marginTop:auto — collapses to 0 when the feed overflows), so
                  past work clusters above Spawn agent and the active cards
                  keep the top. The collapse behavior (flex `order` +
                  marginTop:auto on collapsed headings) applies within the
                  dock, unchanged. */}
              {(historyRows.length > 0 || hasRecent) && (
                <div
                  style={{
                    marginTop: 'auto',
                    display: 'flex',
                    flexDirection: 'column',
                    gap: '8px',
                  }}
                >
                  {historyRows}
                  {/* RECENT — daemon sessions with no card in the layout. Since named
            workspace sessions are gone, this is how past conversations stay
            reachable: click one and it comes back as an agent via --resume. */}
                  {onResumeSession && (recentSessions?.length ?? 0) > 0 && (
                    <>
                      {renderSectionHeading('recent', 'RECENT', recentSessions!.length, 3)}
                      {!collapsedSections.recent &&
                        recentSessions!.map((s) => {
                          const provider = (s.provider || 'claude') as AgentProvider;
                          const hue = PROVIDER_HUE[provider] ?? 'var(--wks-accent)';
                          const name = recentSessionLabel(s);
                          const age = s.updatedAt ? relTime(Date.now() - s.updatedAt) : '';
                          return (
                            <div
                              key={s.sessionId}
                              role="button"
                              tabIndex={0}
                              onClick={() => onResumeSession(s)}
                              onKeyDown={(e) => {
                                if (e.key === 'Enter') onResumeSession(s);
                              }}
                              onMouseEnter={(e) => {
                                (e.currentTarget as HTMLElement).style.color =
                                  'var(--wks-text-secondary)';
                              }}
                              onMouseLeave={(e) => {
                                (e.currentTarget as HTMLElement).style.color =
                                  'var(--wks-text-faint)';
                              }}
                              title={`${name} — click to resume\n${s.cwd}${s.model ? `\n${s.model}` : ''}`}
                              style={{
                                margin: '0 16px',
                                padding: '3px 2px',
                                display: 'flex',
                                alignItems: 'center',
                                gap: 7,
                                minWidth: 0,
                                borderRadius: 'var(--wks-radius-md)',
                                cursor: 'pointer',
                                fontFamily: 'var(--wks-font-mono)',
                                fontSize: '0.66rem',
                                color: 'var(--wks-text-faint)',
                                transition: 'color 0.12s',
                              }}
                            >
                              <span
                                style={{
                                  flexShrink: 0,
                                  width: 6,
                                  height: 6,
                                  borderRadius: '50%',
                                  background: hue,
                                  opacity: 0.7,
                                }}
                              />
                              <span
                                style={{
                                  overflow: 'hidden',
                                  textOverflow: 'ellipsis',
                                  whiteSpace: 'nowrap',
                                }}
                              >
                                {name}
                              </span>
                              {age && (
                                <span
                                  style={{
                                    marginLeft: 'auto',
                                    flexShrink: 0,
                                    whiteSpace: 'nowrap',
                                  }}
                                >
                                  {age}
                                </span>
                              )}
                            </div>
                          );
                        })}
                    </>
                  )}
                </div>
              )}
            </>
          );
        })()}
      </div>

      {/* Spawn agent — the mock's bottom pill: green + coin, label, kbd hint. */}
      <button
        onClick={onSpawnAgent}
        style={{
          width: 'calc(100% - 24px)',
          margin: '4px 12px 10px',
          padding: '11px 12px',
          display: 'flex',
          alignItems: 'center',
          gap: 9,
          border: '1px solid transparent',
          borderRadius: 9,
          cursor: 'pointer',
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
          (e.currentTarget as HTMLElement).style.borderColor = 'transparent';
        }}
        title="Spawn a new agent (Ctrl+Shift+N)"
      >
        <span
          style={{
            flexShrink: 0,
            display: 'grid',
            placeItems: 'center',
            width: 22,
            height: 22,
            borderRadius: 'var(--wks-radius-pill)',
            background: 'var(--wks-success)',
            color: 'var(--wks-bg-base)',
            fontWeight: 700,
            fontSize: '0.85rem',
            lineHeight: 1,
          }}
        >
          +
        </span>
        <span style={{ fontSize: '0.78rem', fontWeight: 600, whiteSpace: 'nowrap' }}>
          Spawn agent
        </span>
        <span
          style={{
            marginLeft: 'auto',
            fontFamily: 'var(--wks-font-mono)',
            fontSize: '0.64rem',
            color: 'var(--wks-text-faint)',
            whiteSpace: 'nowrap',
          }}
        >
          ⌃⇧N
        </span>
      </button>

      {/* Footer — one row like the mock: ● hub on the left, ? Help right. */}
      <HubStatus onOpenRemote={onOpenRemote} onToggleHelp={onToggleHelp} />

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
    fontSize: '0.66rem',
    fontWeight: 600,
    letterSpacing: 0,
    textTransform: 'none',
    whiteSpace: 'nowrap',
  };
}

/** Inline waiting-card actions (spec 2a): solid Approve, quiet Reply/Answer. */
function cardActionStyle(primary: boolean): React.CSSProperties {
  return {
    flex: 1,
    padding: '4px 0',
    textAlign: 'center',
    borderRadius: 'var(--wks-radius-md)',
    border: 'none',
    cursor: 'pointer',
    fontFamily: 'inherit',
    fontSize: '0.7rem',
    fontWeight: 700,
    background: primary ? 'var(--wks-success)' : 'var(--wks-bg-base)',
    color: primary ? 'var(--wks-bg-base)' : 'var(--wks-text-primary)',
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
