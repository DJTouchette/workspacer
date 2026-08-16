import React, { useState, useRef, useMemo, useEffect, useCallback } from 'react';
import {
  Plus,
  ChevronLeft,
  ChevronRight,
  LayoutGrid,
  Compass,
  History,
  Settings,
  Smartphone,
} from 'lucide-react';
import { BrandMark, Wordmark } from './Brand';
import { AgentWorkspace } from '../types/pane';
import type { RecentAgentSession } from '../../../main/shared/ipcTypes';
import type { SessionAmbientState, ClaudeSessionSnapshot } from '../types/claudeSession';
import type { AttentionItem, AttentionKind } from '../types/attention';
import { deriveSessionStats, fmtTokens, fmtUSD, ctxColor, planProgress } from '../lib/sessionStats';
import { ensureKeyframes } from './claude-shared';
import { collectRecentActivity, type ActivityLine } from '../lib/agentActivityLog';
import { shortModelLabel } from '../lib/modelLabel';
import { agentAttentionScore } from '../lib/attentionRouter';
import { AgentLogo } from './agentLogos';
import { HubChip } from './HubChip';
import { hubOfflineLabel } from '../lib/federation';
import { ProjectMark } from './ProjectMark';
import type { ProjectIdentity } from '../hooks/useConfig';
import { requestInspector } from '../lib/watchBus';
import { useAttention } from '../contexts/AttentionContext';
import { useUiMode } from '../hooks/useUiMode';
import NotificationCenter from './notifications/NotificationCenter';
import { ContextMenu, ContextMenuItem } from './ContextMenu';
import { SIDEBAR_DEFAULT_WIDTH, SIDEBAR_RAIL_WIDTH } from '../lib/sidebarWidth';

// The expanded width is user-resizable and persisted, so it is NOT a constant
// here — it arrives as the `width` prop (see lib/sidebarWidth.ts for the
// default, the clamp band, and the constants the app shell offsets by).

/** Below this width the header drops the wordmark and keeps only the { ▮ } mark.
 *  Measured, not guessed: mark + wordmark + the action cluster + the collapse
 *  toggle need 290px, so anything narrower would push brand text under the
 *  icons. (The brand button also clips, so a wider font can't overlap either.) */
const WORDMARK_MIN_WIDTH = 292;

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

/**
 * A header/rail action icon. Sized and tinted to match the notification bell's
 * own trigger button so "+ / bell / phone" reads as one cluster of app-level
 * affordances rather than three buttons that happen to sit together.
 */
const IconAction: React.FC<{
  title: string;
  label: string;
  onClick: () => void;
  children: React.ReactNode;
}> = ({ title, label, onClick, children }) => (
  <button
    onClick={onClick}
    title={title}
    aria-label={label}
    style={{
      display: 'flex',
      alignItems: 'center',
      justifyContent: 'center',
      width: 24,
      height: 24,
      padding: 0,
      flexShrink: 0,
      border: 'none',
      borderRadius: 'var(--wks-radius-sm)',
      cursor: 'pointer',
      background: 'transparent',
      color: 'var(--wks-text-muted)',
      transition: 'background 0.12s, color 0.12s',
    }}
    onMouseEnter={(e) => {
      (e.currentTarget as HTMLElement).style.background = 'var(--wks-bg-hover)';
      (e.currentTarget as HTMLElement).style.color = 'var(--wks-text-primary)';
    }}
    onMouseLeave={(e) => {
      (e.currentTarget as HTMLElement).style.background = 'transparent';
      (e.currentTarget as HTMLElement).style.color = 'var(--wks-text-muted)';
    }}
  >
    {children}
  </button>
);

/**
 * The app-level action cluster: new agent, notifications, mobile client. One
 * definition, two mount points (header row and collapsed rail column) in the
 * same order, so nothing jumps when the sidebar collapses. Spawn is an icon
 * here — it used to be a full-width pill pinned to the bottom, which spent the
 * sidebar's scarcest space on a once-per-session action.
 */
const ActionCluster: React.FC<{
  vertical?: boolean;
  onSpawnAgent: () => void;
  onOpenRemote?: () => void;
}> = ({ vertical, onSpawnAgent, onOpenRemote }) => (
  <div
    style={{
      display: 'flex',
      flexDirection: vertical ? 'column' : 'row',
      alignItems: 'center',
      gap: 2,
      flexShrink: 0,
    }}
  >
    <IconAction title="New agent (Ctrl+Shift+N)" label="New agent" onClick={onSpawnAgent}>
      <Plus size={15} strokeWidth={2} />
    </IconAction>
    {/* The bell's history lives with the agents, where what it records happened. */}
    <NotificationCenter />
    {onOpenRemote && (
      <IconAction
        title="Mobile client — drive agents from your phone"
        label="Mobile client"
        onClick={onOpenRemote}
      >
        <Smartphone size={14} strokeWidth={1.75} />
      </IconAction>
    )}
  </div>
);

/**
 * A row in the sidebar's bottom strip (History, Settings). Deliberately the
 * quietest thing in the panel — mono, faint, no fill — so a pointer out of the
 * live feed never competes with the agent cards above it.
 */
const FooterRow: React.FC<{
  icon: React.ReactNode;
  label: string;
  title: string;
  onClick: () => void;
  /** Optional right-aligned detail (e.g. History's count). */
  trailing?: React.ReactNode;
}> = ({ icon, label, title, onClick, trailing }) => (
  <div
    role="button"
    tabIndex={0}
    onClick={onClick}
    onKeyDown={(e) => {
      if (e.key === 'Enter') onClick();
    }}
    title={title}
    style={{
      padding: '5px 6px',
      display: 'flex',
      alignItems: 'center',
      gap: 6,
      fontSize: '0.66rem',
      fontFamily: 'var(--wks-font-mono)',
      color: 'var(--wks-text-faint)',
      cursor: 'pointer',
      userSelect: 'none',
      transition: 'color 0.12s',
    }}
    onMouseEnter={(e) => {
      (e.currentTarget as HTMLElement).style.color = 'var(--wks-text-secondary)';
    }}
    onMouseLeave={(e) => {
      (e.currentTarget as HTMLElement).style.color = 'var(--wks-text-faint)';
    }}
  >
    {icon}
    <span>{label}</span>
    <span style={{ marginLeft: 'auto', display: 'flex', alignItems: 'center', gap: 4 }}>
      {trailing}
      <ChevronRight size={11} strokeWidth={2} style={{ flexShrink: 0 }} />
    </span>
  </div>
);

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
  /** config.projects — per-directory identity for the card marks. Passed rather
   *  than read from context so the sidebar harness can render without a
   *  ConfigProvider, exactly as every other config-derived prop here is. */
  projects?: Record<string, ProjectIdentity>;
  onSelectAgent: (id: string) => void;
  onSpawnAgent: () => void;
  onTerminateAgent: (id: string) => void;
  onRenameAgent: (id: string, name: string) => void;
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
  /** Expanded width in px — user-resizable and persisted by the app shell
   *  (`config.ui.sidebarWidth`). Absent = the shipped default. The rail width is
   *  fixed and ignores this. */
  width?: number;
  /** Brief flash on the header when "next attention" found nothing to jump to. */
  noAttentionFlash?: boolean;
  /** Resumable daemon sessions not in the layout — drives the History footer
   *  row's count (the list itself lives in the Sessions pane). */
  recentSessions?: RecentAgentSession[];
  /** Open the Sessions pane (session history browser). */
  onOpenHistory?: () => void;
  /** Open the Settings pane — the quiet footer row beside History. */
  onOpenSettings?: () => void;
}

const SideBar: React.FC<SideBarProps> = ({
  projects,
  agents,
  activeAgentId,
  statusBySession,
  snapshotBySession,
  onSelectAgent,
  onSpawnAgent,
  onTerminateAgent,
  onRenameAgent,
  onOpenInbox,
  onToggleFleet,
  viewLevel,
  onOpenRemote,
  onToggleCollapse,
  noAttentionFlash,
  collapsed,
  width,
  recentSessions,
  onOpenHistory,
  onOpenSettings,
}) => {
  // Attention comes from the single feed (the spine), so a card's waiting state
  // and the rail tile's amber dot can never disagree.
  const { topByAgent, approve } = useAttention();
  // Focus mode narrows WHICH agents get a full card; it never hides the feed.
  const { manifest: uiManifest } = useUiMode();
  // Focus mode's collapsed "N others" row — a transient reveal, not persisted.
  // Reset on every mode flip so re-entering focus always starts quiet; leaving
  // it expanded would make the next focus session silently behave like fleet.
  const [othersExpanded, setOthersExpanded] = useState(false);
  useEffect(() => {
    setOthersExpanded(false);
  }, [uiManifest.feed]);
  const [contextMenu, setContextMenu] = useState<{ agentId: string; x: number; y: number } | null>(
    null,
  );
  const [renamingId, setRenamingId] = useState<string | null>(null);
  const [renameValue, setRenameValue] = useState('');
  // Type-to-filter the agent list by name or provider (nav rows always shown).
  const [filter, setFilter] = useState('');
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

        {/* Same cluster as the expanded header, stacked. Kept above the tiles
            (not below them) so collapsing the sidebar doesn't move it, and the
            bell stays reachable in the rail — its panel is fixed-positioned and
            opens over the content area. */}
        <div
          style={{
            flexShrink: 0,
            width: '100%',
            display: 'flex',
            justifyContent: 'center',
            paddingBottom: 8,
            borderBottom: '1px solid var(--wks-border-subtle)',
          }}
        >
          <ActionCluster vertical onSpawnAgent={onSpawnAgent} onOpenRemote={onOpenRemote} />
        </div>

        {/* No aggregate needs-you badge here: each rail tile already carries its
            agent's attention (amber dot + kind glyph, see railTile), and
            `next-attention` (Ctrl+Shift+Space) jumps to the next blocked agent
            from anywhere. A count pill on top of that was pure duplication. */}
        <div
          style={{
            flex: 1,
            overflowY: 'auto',
            overflowX: 'hidden',
            display: 'flex',
            flexDirection: 'column',
            alignItems: 'center',
            gap: '8px',
            padding: '2px 0 10px',
            width: '100%',
          }}
        >
          {agents.map(railTile)}
        </div>

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
        width: `${width ?? SIDEBAR_DEFAULT_WIDTH}px`,
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
        // Clip children (card backgrounds, the feed's scroll) to the rounded
        // corners. The context menu is position:fixed so it still escapes.
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
          gap: 8,
          padding: '4px 10px 10px 14px',
        }}
      >
        <button
          onClick={() => overviewAgent && onSelectAgent(overviewAgent.id)}
          title="Overview — cross-agent dashboards & plugin panes"
          style={{
            display: 'flex',
            alignItems: 'center',
            gap: 8,
            padding: 0,
            border: 'none',
            background: 'transparent',
            cursor: 'pointer',
            fontFamily: 'inherit',
            minWidth: 0,
            overflow: 'hidden',
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
          {/* The wordmark is the first thing to go when the user drags the
              sidebar narrow: the mark alone still opens Overview, and the
              action cluster must never be overlapped by brand text. */}
          {(width ?? SIDEBAR_DEFAULT_WIDTH) >= WORDMARK_MIN_WIDTH && <Wordmark size={16} />}
        </button>
        <div style={{ marginLeft: 'auto', display: 'flex', alignItems: 'center' }}>
          <ActionCluster onSpawnAgent={onSpawnAgent} onOpenRemote={onOpenRemote} />
        </div>
        {onToggleCollapse && (
          <button
            onClick={onToggleCollapse}
            title="Collapse sidebar (Ctrl+B)"
            style={{
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
          // Bottom padding is the feed's own now that no footer sits under it —
          // the History row would otherwise touch the window edge.
          padding: '2px 0 10px',
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
            const state = agent.sessionId ? statusBySession[agent.sessionId] : undefined;
            const top: AttentionItem | undefined = topByAgent.get(agent.id);
            const snap = agent.sessionId ? snapshotBySession[agent.sessionId] : undefined;
            // Federation: a peer hub's agent gets a hub chip; when that peer's
            // link is down the card tombstones (muted, "hub offline — last
            // seen …") instead of pretending its last ambient state is live.
            const hub = snap?.hub ?? agent.hub;
            const hubOffline = !!(hub && snap?.hubOffline);
            const cardState = hubOffline ? ('done' as const) : cardStateOf(agent);
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
            // Depth is state-scaled: busy/waiting cards earn up to 5 lines
            // (the room the old EARLIER/RECENT dock gave back), resting cards
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
            const activity = collectRecentActivity(snap, 5);
            const log: LogLine[] = [];
            if (hubOffline) {
              log.push({
                text: hubOfflineLabel(snap?.lastActivity, now),
                color: 'var(--wks-warning)',
              });
            } else if (cardState === 'waiting') {
              const what =
                top && WAITING_KINDS.has(top.kind)
                  ? top.title
                  : state === 'waiting_approval'
                    ? 'approve a tool call'
                    : 'your input';
              pushActivity(activity.slice(-4));
              log.push({ text: `Waiting: ${what}`, color: 'var(--wks-warning)' });
            } else if (!agent.sessionId) {
              // Remote agents can't be respawned locally — no click affordance.
              log.push({
                text: hub ? 'Stopped' : 'Stopped — click to respawn',
                color: 'var(--wks-text-faint)',
              });
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
            const label = hubOffline
              ? hubOfflineLabel(snap?.lastActivity, now)
              : cardState === 'waiting'
                ? top && WAITING_KINDS.has(top.kind)
                  ? KIND_VISUAL_LABEL[top.kind]
                  : 'Waiting on you'
                : cardState === 'working'
                  ? 'Working'
                  : agent.sessionId
                    ? 'Idle'
                    : hub
                      ? 'Stopped'
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
                        border: '2px solid var(--wks-busy)',
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
                        display: 'flex',
                        alignItems: 'center',
                        gap: 6,
                        fontSize: '0.8rem',
                        fontWeight: 600,
                        color: 'var(--wks-text-primary)',
                        overflow: 'hidden',
                        lineHeight: 1.3,
                      }}
                    >
                      {/* Which project this agent belongs to, at a glance. Every
                          card used to be the same shape, so telling one repo's
                          agents from another's meant reading the cwd tooltip.
                          Derived from the path when unconfigured, so this is
                          useful without anyone setting anything up. */}
                      <ProjectMark cwd={agent.cwd} projects={projects} size={14} />
                      <span
                        style={{
                          minWidth: 0,
                          overflow: 'hidden',
                          textOverflow: 'ellipsis',
                          whiteSpace: 'nowrap',
                        }}
                      >
                        {agent.name}
                      </span>
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
                      // No fill, no border, no brand tint. This chip repeats on every
                      // card, so it stops being a surface and joins the tokens/cost
                      // text as one quiet monochrome meta line.
                      color: 'var(--wks-text-tertiary)',
                      fontFamily: 'var(--wks-font-mono)',
                      fontSize: '0.66rem',
                    }}
                  >
                    <AgentLogo provider={provider} size={11} neutral style={{ flexShrink: 0 }} />
                    {model}
                  </span>
                  {hub && <HubChip name={hub} offline={hubOffline} />}
                  {(stats.tokens !== undefined || stats.costUSD !== undefined) && (
                    <span
                      style={{
                        marginLeft: 'auto',
                        flexShrink: 0,
                        fontFamily: 'var(--wks-font-mono)',
                        fontSize: '0.66rem',
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

          // Live-feed order (spec 2a): Overview nav row pinned first, then
          // waiting → working → done cards (the sort above). Old finished
          // agents keep their (compact, dimmed) done cards — the EARLIER
          // demotion left with the history dock; retired sessions live in the
          // Sessions pane instead.
          const cards = topLevel.filter((agent) => !agent.global);

          // Focus mode narrows the feed to what you're actually attending to:
          // the piloted agent, plus anything BLOCKED on you (never hide a block
          // — its inline Approve/Reply is the whole point of the card). Agents
          // that are merely working or already finished fold into one quiet
          // "N others" row below, expandable in place.
          //
          // Subagents follow their parent: quieting a parent quiets its children
          // with it, and they count toward the total.
          const quietOthers = uiManifest.feed === 'active-and-blocked' && !othersExpanded;
          const isLoud = (agent: (typeof agents)[0]) =>
            agent.id === activeAgentId || cardStateOf(agent) === 'waiting';
          const loudCards = quietOthers ? cards.filter(isLoud) : cards;
          const quieted = quietOthers ? cards.filter((a) => !isLoud(a)) : [];
          // Count the folded subtree, not just its roots, so the row's number
          // matches what expanding it actually reveals.
          const quietedCount = quieted.reduce(
            (n, a) => n + 1 + (childrenByParent.get(a.id)?.length ?? 0),
            0,
          );
          const quietedWorking = quieted.filter((a) => cardStateOf(a) === 'working').length;

          const rows: React.ReactNode[] = [];
          for (const agent of loudCards) {
            rows.push(renderAgentCard(agent, false));
            // Render children indented directly after their parent.
            for (const child of childrenByParent.get(agent.id) ?? []) {
              rows.push(renderAgentCard(child, true));
            }
          }
          const historyCount = recentSessions?.length ?? 0;
          return (
            <>
              {rows}
              {/* Focus mode's quieted remainder — the periphery goes quiet, not
                  invisible. Same mono/faint vocabulary as the History row below
                  so the two read as one family of "there's more over here"
                  pointers, but this one expands in place instead of navigating. */}
              {quietedCount > 0 && (
                <div
                  role="button"
                  tabIndex={0}
                  aria-expanded={false}
                  onClick={() => setOthersExpanded(true)}
                  onKeyDown={(e) => {
                    if (e.key === 'Enter') setOthersExpanded(true);
                  }}
                  title="Show the agents that aren't waiting on you"
                  style={{
                    margin: '2px 12px 0',
                    padding: '6px 6px 5px',
                    display: 'flex',
                    alignItems: 'center',
                    gap: 6,
                    fontSize: '0.66rem',
                    fontFamily: 'var(--wks-font-mono)',
                    color: 'var(--wks-text-faint)',
                    cursor: 'pointer',
                    userSelect: 'none',
                    transition: 'color 0.12s',
                  }}
                  onMouseEnter={(e) => {
                    (e.currentTarget as HTMLElement).style.color = 'var(--wks-text-secondary)';
                  }}
                  onMouseLeave={(e) => {
                    (e.currentTarget as HTMLElement).style.color = 'var(--wks-text-faint)';
                  }}
                >
                  <ChevronRight size={11} strokeWidth={2} style={{ flexShrink: 0 }} />
                  <span>
                    {quietedCount} other{quietedCount === 1 ? '' : 's'}
                  </span>
                  {quietedWorking > 0 && (
                    <span style={{ marginLeft: 'auto' }}>{quietedWorking} working</span>
                  )}
                </div>
              )}
              {/* Collapse the reveal again — only offered once expanded, so the
                  quiet state stays a single row. */}
              {uiManifest.feed === 'active-and-blocked' && othersExpanded && (
                <div
                  role="button"
                  tabIndex={0}
                  aria-expanded={true}
                  onClick={() => setOthersExpanded(false)}
                  onKeyDown={(e) => {
                    if (e.key === 'Enter') setOthersExpanded(false);
                  }}
                  title="Quiet the agents that aren't waiting on you"
                  style={{
                    margin: '2px 12px 0',
                    padding: '6px 6px 5px',
                    display: 'flex',
                    alignItems: 'center',
                    gap: 6,
                    fontSize: '0.66rem',
                    fontFamily: 'var(--wks-font-mono)',
                    color: 'var(--wks-text-faint)',
                    cursor: 'pointer',
                    userSelect: 'none',
                  }}
                >
                  <ChevronLeft size={11} strokeWidth={2} style={{ flexShrink: 0 }} />
                  <span>Show less</span>
                </div>
              )}
              {/* Footer strip — the quiet pointers out of the live feed: past
                  work, and the app's own settings. Pinned to the feed bottom
                  (margin-top:auto collapses to 0 when the feed overflows) so
                  live cards always keep the top. The border belongs to the
                  strip, not the rows, so one rule separates it from the feed
                  however many rows are showing. */}
              {(onOpenSettings || (onOpenHistory && historyCount > 0)) && (
                <div
                  style={{
                    margin: 'auto 12px 0',
                    paddingTop: 2,
                    borderTop: '1px solid var(--wks-border-subtle)',
                  }}
                >
                  {onOpenHistory && historyCount > 0 && (
                    <FooterRow
                      icon={<History size={12} strokeWidth={1.75} style={{ flexShrink: 0 }} />}
                      label="History"
                      title="Browse and resume past sessions"
                      trailing={<span>{historyCount}</span>}
                      onClick={onOpenHistory}
                    />
                  )}
                  {onOpenSettings && (
                    <FooterRow
                      icon={<Settings size={12} strokeWidth={1.75} style={{ flexShrink: 0 }} />}
                      label="Settings"
                      title="App settings"
                      onClick={onOpenSettings}
                    />
                  )}
                </div>
              )}
            </>
          );
        })()}
      </div>

      {/* No footer: spawn, notifications and the mobile client moved into the
          header cluster, and the hub dot went with them — a healthy bus is not
          news, and `useHubReconnect` handles a dead one without being watched.
          The feed now owns every pixel below the header. */}

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

export default SideBar;
