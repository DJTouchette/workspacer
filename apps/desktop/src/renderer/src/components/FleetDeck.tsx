import { findAgentChatPane } from '../hooks/useAgentManager';
import { FleetChatDestination } from './claude/RetainedSessionChat';
import { waitForSessionChatController } from '../hooks/useSessionChatController';
import { useFleetAgentMenu } from './FleetAgentMenu';
import { SmallButton } from './settings/primitives';
import React, { useEffect, useMemo, useRef, useState } from 'react';
import {
  X,
  Minimize2,
  ExternalLink,
  Search,
  Radar,
  CornerDownLeft,
  History,
  ChevronRight,
  Compass,
} from 'lucide-react';
import type { AgentWorkspace } from '../types/pane';
import type { ClaudeSessionSnapshot } from '../types/claudeSession';
import { useAttention } from '../contexts/AttentionContext';
import { AgentCard } from './AgentCard';
import { Surface } from './Surface';
import { InspectorCard } from './claude/InspectorCard';
import { AgentLogo } from './agentLogos';
import { requestInspector } from '../lib/watchBus';
import { useConfig } from '../hooks/useConfig';
import { DEFAULT_SHORTCUTS } from '../hooks/configDefaults';
import {
  eventMatchesCombo,
  digitFromRangeEvent,
  formatBinding,
  resolveLeader,
} from '../lib/shortcuts';
import { isLayerArmed } from '../lib/layerArmed';
import { orderFleetTimeline, FleetActivityTime } from './FleetTimeline';
import './FleetTimeline.css';

const STYLE_ID = 'fleet-deck-keyframes';
function ensureFleetKeyframes() {
  if (typeof document === 'undefined' || document.getElementById(STYLE_ID)) return;
  const s = document.createElement('style');
  s.id = STYLE_ID;
  // Pulse for blocked agents + a single keyboard-focus ring for the whole deck.
  // Inline styles can't express :focus-visible, so the deck opts in via the
  // `.fleet-root` class and everything focusable inside gets a consistent ring
  // (buttons/rows) or accent halo (text fields) — the deck had none before.
  s.textContent = `
    @keyframes fleetPulse { 0%,100% { box-shadow: 0 0 0 1px currentColor; } 50% { box-shadow: 0 0 0 3px currentColor, 0 0 18px currentColor; } }
    .fleet-root button:focus-visible,
    .fleet-root [role="button"]:focus-visible,
    .fleet-root tr:focus-visible {
      outline: 2px solid var(--wks-accent);
      outline-offset: 2px;
      border-radius: var(--wks-radius-sm);
    }
    .fleet-root input:focus-visible,
    .fleet-root textarea:focus-visible {
      outline: none;
      border-color: var(--wks-accent);
      box-shadow: 0 0 0 3px var(--wks-accent-glow);
    }
  `;
  document.head.appendChild(s);
}

interface Props {
  onOpenRecentAgents?: () => void;
  onTerminateAgent?: (id: string) => Promise<void>;
  onEnsureAgentChat?: (id: string) => void;
  /** Inset so the deck sits inside the content area (right of sidebar, below navbar). */
  top: number;
  left: number;
}

/**
 * An agent card flipped in place into its live Inspector — the shared
 * {@link InspectorCard} fed the same `snapshotBySession` entry the collapsed card
 * uses, so it stays live for any agent (not just the piloted one). Sits in its
 * own full-width area below its timeline row; collapse or "open as pane"
 * from the header. The inspector body scrolls within its bounded height.
 *
 * Surface `raised` = fill only: the old 1.5px accent border is now the accent
 * `tone` rail, so the card separates itself on one channel, not three.
 */
const ExpandedAgentCard: React.FC<{
  agent: AgentWorkspace;
  snapshot: ClaudeSessionSnapshot | undefined;
  onCollapse: () => void;
  onOpenAsPane: () => void;
}> = ({ agent, snapshot, onCollapse, onOpenAsPane }) => (
  <Surface
    elevation="raised"
    radius="lg"
    tone="var(--wks-accent)"
    onClick={(e) => e.stopPropagation()}
    style={{
      display: 'flex',
      flexDirection: 'column',
      minHeight: 0,
      overflow: 'hidden',
    }}
  >
    <div
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: 8,
        padding: '10px 12px',
        borderBottom: '1px solid var(--wks-glass-border)',
        flexShrink: 0,
      }}
    >
      {agent.manager ? (
        <Compass size={13} strokeWidth={2} style={{ flexShrink: 0 }} />
      ) : (
        <AgentLogo
          provider={agent.provider ?? 'claude'}
          size={14}
          style={{ color: 'var(--wks-text-tertiary)', flexShrink: 0 }}
        />
      )}
      <span
        style={{
          fontSize: '0.9rem',
          fontWeight: 700,
          color: 'var(--wks-text-primary)',
          overflow: 'hidden',
          textOverflow: 'ellipsis',
          whiteSpace: 'nowrap',
          minWidth: 0,
        }}
      >
        {agent.name}
      </span>
      <div style={{ flex: 1 }} />
      <button
        onClick={onOpenAsPane}
        title="Open this inspector as its own pane"
        style={expandBtn}
        onMouseEnter={(e) => {
          (e.currentTarget as HTMLElement).style.color = 'var(--wks-text-primary)';
          (e.currentTarget as HTMLElement).style.borderColor = 'var(--wks-accent)';
        }}
        onMouseLeave={(e) => {
          (e.currentTarget as HTMLElement).style.color = 'var(--wks-text-faint)';
          (e.currentTarget as HTMLElement).style.borderColor = 'var(--wks-glass-border)';
        }}
      >
        <ExternalLink size={13} strokeWidth={2} />
      </button>
      <button
        onClick={onCollapse}
        title="Collapse (Esc)"
        style={expandBtn}
        onMouseEnter={(e) => {
          (e.currentTarget as HTMLElement).style.color = 'var(--wks-text-primary)';
          (e.currentTarget as HTMLElement).style.borderColor = 'var(--wks-accent)';
        }}
        onMouseLeave={(e) => {
          (e.currentTarget as HTMLElement).style.color = 'var(--wks-text-faint)';
          (e.currentTarget as HTMLElement).style.borderColor = 'var(--wks-glass-border)';
        }}
      >
        <Minimize2 size={13} strokeWidth={2} />
      </button>
    </div>
    <div style={{ flex: 1, minHeight: 0 }}>
      <InspectorCard snapshot={snapshot} sessionId={agent.sessionId} />
    </div>
  </Surface>
);

const expandBtn: React.CSSProperties = {
  display: 'inline-flex',
  alignItems: 'center',
  justifyContent: 'center',
  flexShrink: 0,
  width: 24,
  height: 24,
  padding: 0,
  borderRadius: 'var(--wks-radius-md)',
  border: '1px solid var(--wks-glass-border)',
  background: 'transparent',
  color: 'var(--wks-text-faint)',
  cursor: 'pointer',
  transition: 'color 0.12s, border-color 0.12s',
};

/**
 * The Fleet runbook: metadata-defined manager anchors above workers ordered by
 * recorded last activity. Rendered OVER the still-mounted per-agent workspaces,
 * so entering/leaving the deck never remounts a
 * pane. Card activation selects the same retained GUI inside the Fleet shell;
 * Inspector is a separate action and does not replace chat navigation.
 */
const FleetDeck: React.FC<Props> = ({
  top,
  left,
  onOpenRecentAgents,
  onTerminateAgent,
  onEnsureAgentChat,
}) => {
  ensureFleetKeyframes();
  const {
    agents,
    snapshotBySession,
    counts,
    setViewLevel,
    topByAgent,
    spawnAgent,
    approve,
    answer,
    openAgent,
  } = useAttention();

  const agentsRef = useRef(agents);
  agentsRef.current = agents;
  const prepareChat = (agent: AgentWorkspace) => {
    if (!agent.sessionId) return Promise.resolve(undefined);
    onEnsureAgentChat?.(agent.id);
    return waitForSessionChatController(agent.sessionId, () => {
      const current = agentsRef.current.find(
        (a) => a.id === agent.id && a.sessionId === agent.sessionId,
      );
      return current ? findAgentChatPane(current)?.id : undefined;
    });
  };

  const [controlUnavailable, setControlUnavailable] = useState<string | undefined>(
    'Checking agent control connection…',
  );
  const agentMenu = useFleetAgentMenu({
    agents,
    snapshotBySession,
    onTerminate: onTerminateAgent,
    disabledReason: controlUnavailable,
  });
  useEffect(() => {
    let disposed = false;
    let eventSeen = false;
    const apply = (connected: boolean) => {
      if (!disposed)
        setControlUnavailable(
          connected ? undefined : 'Hub connection is offline — reconnect to terminate agents',
        );
    };
    const off = window.electronAPI.onHubStatus?.((status) => {
      eventSeen = true;
      apply(status.connected);
    });
    const request = window.electronAPI.getHubStatus?.();
    if (request)
      void request
        .then((status) => {
          if (!eventSeen) apply(!!status?.connected);
        })
        .catch(() => apply(false));
    else setControlUnavailable('Agent control is unavailable');
    return () => {
      disposed = true;
      off?.();
    };
  }, []);

  const realAgents = useMemo(() => agents.filter((a) => !a.global), [agents]);

  // Deck-scoped keybindings (fleet-*), remappable in Settings → Keybindings.
  // Defaults merged under user overrides so a partial saved map still binds.
  const { config } = useConfig();
  const sc = useMemo(
    () => ({ ...DEFAULT_SHORTCUTS, ...(config.keybindings?.shortcuts ?? {}) }),
    [config.keybindings?.shortcuts],
  );
  // The command layer's leader, resolved exactly like App resolves kbPrefix
  // (leaderOverride beats the platform-resolved prefix) — the deck's capture
  // handler must recognize it to let it through (see onKey below).
  const kbPrefix = useMemo(
    () =>
      config.keybindings?.commandLayer?.leaderOverride?.trim() ||
      resolveLeader(config.keybindings?.prefix ?? 'ctrl+space'),
    [config.keybindings?.commandLayer?.leaderOverride, config.keybindings?.prefix],
  );

  // Type-to-filter by name or provider. Applied before sort, so cards, list, and
  // keyboard nav all operate on the filtered set; header counts stay whole-fleet.
  const [query, setQuery] = useState('');

  // Staleness needs a clock even when no snapshots arrive (that IS the stale
  // case) — a slow tick re-evaluates the list rows' warning tint.
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const t = setInterval(() => setNow(Date.now()), 30_000);
    return () => clearInterval(t);
  }, []);

  // Managers are a stable metadata-defined anchor. Only workers are time ordered.
  const displayOrder = useMemo(
    () => orderFleetTimeline(realAgents, snapshotBySession, query),
    [realAgents, snapshotBySession, query],
  );

  const working = realAgents.filter((a) => {
    const s = a.sessionId ? snapshotBySession[a.sessionId]?.ambientState : undefined;
    return s === 'thinking' || s === 'streaming' || s === 'background';
  }).length;

  const scrollRef = useRef<HTMLDivElement>(null);

  // Timeline selection follows `displayOrder`, with approve/answer
  // acting on the selected agent's top attention item — kept entirely within
  // the deck.
  const [selectedId, setSelectedId] = useState<string | null>(null);
  // The card flipped in place into its live InspectorCard (null = none).
  const [expandedId, setExpandedId] = useState<string | null>(null);
  const [chatId, setChatId] = useState<string | null>(null);
  const chatAgent = realAgents.find((a) => a.id === chatId);
  const openChat = (id: string) => {
    onEnsureAgentChat?.(id);
    setSelectedId(id);
    setChatId(id);
    setExpandedId(null);
  };
  const backToFleet = () => {
    setChatId(null);
    requestAnimationFrame(() =>
      document.querySelector<HTMLElement>(`[data-fleet-agent="${chatId}"]`)?.focus(),
    );
  };
  useEffect(() => {
    if (chatId && !realAgents.some((a) => a.id === chatId)) setChatId(null);
  }, [realAgents, chatId]);
  // Keep selection valid as the fleet re-sorts / agents come and go.
  useEffect(() => {
    if (displayOrder.length === 0) {
      if (selectedId !== null) setSelectedId(null);
      return;
    }
    if (!selectedId || !displayOrder.some((a) => a.id === selectedId))
      setSelectedId(displayOrder[0].id);
  }, [displayOrder, selectedId]);
  // Drop an expansion if its agent leaves the (filtered) fleet.
  useEffect(() => {
    if (expandedId && !displayOrder.some((a) => a.id === expandedId)) setExpandedId(null);
  }, [displayOrder, expandedId]);

  // Open the selected/expanded agent's inspector as its own pane, leaving the
  // deck (the pane lands in the currently-piloted workspace, like a watch pane).
  const openInspectorPane = (agent: (typeof realAgents)[number]) => {
    if (!agent.sessionId) {
      openAgent(agent.id);
      return;
    }
    requestInspector({ sessionId: agent.sessionId, agentName: agent.name });
    setViewLevel('piloting');
  };

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      // tmux doctrine: the command layer always wins. Both this handler and the
      // window chord dispatcher listen in the capture phase, so their order is
      // whichever effect re-registered last — never let the deck's stops decide
      // the race. While the leader is armed every keystroke is a chord step
      // (the deck's y/n/i/digits would otherwise eat it), and the leader press
      // itself must reach the dispatcher to arm at all.
      if (isLayerArmed()) return;
      if (eventMatchesCombo(e, kbPrefix)) return;
      const t = e.target as HTMLElement | null;
      if (t instanceof Element && t.closest('[data-fleet-action], [data-fleet-chat]')) return;
      if (t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.isContentEditable)) return;
      if (displayOrder.length === 0) return;
      const stop = () => {
        e.preventDefault();
        e.stopPropagation();
      };
      const idx = selectedId ? displayOrder.findIndex((a) => a.id === selectedId) : -1;

      // Escape collapses an in-place expansion before the deck's own Esc (exit
      // fleet) can fire — this handler runs in the capture phase, so stopping
      // propagation keeps the App-level Esc from also unwinding the deck.
      if (e.key === 'Escape' && chatId && !expandedId) {
        stop();
        backToFleet();
        return;
      }
      if (e.key === 'Escape' && expandedId) {
        stop();
        setExpandedId(null);
        return;
      }
      // 'i' flips the focused card in place into its live InspectorCard (toggle).
      if (e.key === 'i' && idx >= 0 && !chatId) {
        stop();
        setExpandedId((cur) => (cur === displayOrder[idx].id ? null : displayOrder[idx].id));
        return;
      }

      if (t instanceof Element && t.closest('button')) return;

      // Keep both existing navigation binding families usable in the timeline.
      const select = (n: number) => {
        const id = displayOrder[Math.max(0, Math.min(displayOrder.length - 1, n))].id;
        setSelectedId(id);
        // Move actual focus too: Shift+F10 must address the row navigation selected.
        Array.from(scrollRef.current?.querySelectorAll<HTMLElement>('[data-fleet-agent]') ?? [])
          .find((element) => element.dataset.fleetAgent === id)
          ?.focus({ preventScroll: true });
      };
      if (
        ['ArrowDown', 'ArrowRight'].includes(e.key) ||
        eventMatchesCombo(e, sc['fleet-list-down']) ||
        eventMatchesCombo(e, sc['fleet-cards-down']) ||
        eventMatchesCombo(e, sc['fleet-cards-right'])
      ) {
        stop();
        select(idx + 1);
        return;
      }
      if (
        ['ArrowUp', 'ArrowLeft'].includes(e.key) ||
        eventMatchesCombo(e, sc['fleet-list-up']) ||
        eventMatchesCombo(e, sc['fleet-cards-up']) ||
        eventMatchesCombo(e, sc['fleet-cards-left'])
      ) {
        stop();
        select(idx - 1);
        return;
      }

      if (idx < 0) return;
      const top = topByAgent.get(displayOrder[idx].id);
      if (!top) {
        if (eventMatchesCombo(e, sc['fleet-open'])) {
          stop();
          openChat(displayOrder[idx].id);
        }
        return;
      }
      if (eventMatchesCombo(e, sc['fleet-open'])) {
        stop();
        openChat(top.agentId);
        return;
      }
      if (top.payload.type === 'approval') {
        if (eventMatchesCombo(e, sc['fleet-approve-yes'])) {
          stop();
          approve(top, 'yes');
          return;
        }
        if (eventMatchesCombo(e, sc['fleet-approve-no'])) {
          stop();
          approve(top, 'no');
          return;
        }
      }
      if (top.payload.type === 'question') {
        const n = digitFromRangeEvent(e, sc['fleet-answer']);
        if (n !== null && n <= (top.payload.questions[0]?.options.length ?? 0)) {
          stop();
          answer(top, { option: n });
          return;
        }
      }
    };
    window.addEventListener('keydown', onKey, true);
    return () => window.removeEventListener('keydown', onKey, true);
  }, [
    displayOrder,
    selectedId,
    expandedId,
    chatId,
    topByAgent,
    approve,
    answer,
    openAgent,
    sc,
    kbPrefix,
  ]);

  useEffect(() => {
    if (!selectedId || chatId) return;
    const row = Array.from(
      scrollRef.current?.querySelectorAll<HTMLElement>('[data-fleet-agent]') ?? [],
    ).find((el) => el.dataset.fleetAgent === selectedId);
    row?.scrollIntoView?.({ block: 'nearest' });
  }, [selectedId, chatId]);

  const renderAgent = (agent: AgentWorkspace) => {
    const snapshot = agent.sessionId ? snapshotBySession[agent.sessionId] : undefined;
    return (
      <div
        key={agent.id}
        data-fleet-agent={agent.id}
        data-fleet-row={agent.id}
        className={`fleet-timeline-row${agent.manager ? ' fleet-manager-anchor' : ''}`}
        tabIndex={0}
        aria-label={agent.name}
        onFocus={(e) => {
          if (e.target === e.currentTarget) setSelectedId(agent.id);
        }}
        onMouseDown={(e) => {
          if (e.button !== 0) e.preventDefault();
        }}
        onClick={(e) => {
          if (!(e.target as Element).closest('button, input, textarea')) openChat(agent.id);
        }}
        {...agentMenu.handlers(agent.id)}
      >
        <div className="fleet-timeline-time">
          {agent.manager ? (
            <span className="fleet-manager-label">
              <Compass size={12} /> Manager
            </span>
          ) : null}
          <FleetActivityTime timestamp={snapshot?.lastActivity} now={now} />
        </div>
        <div className="fleet-timeline-entry">
          <AgentCard
            agent={agent}
            snapshot={snapshot}
            timeline
            onOpen={() => openChat(agent.id)}
            actions={agentMenu.overflow(agent)}
            prepareChat={() => prepareChat(agent)}
            onInspect={() => setExpandedId(agent.id)}
          />
          {expandedId === agent.id && (
            <div style={{ height: 440, display: 'flex', flexDirection: 'column' }}>
              <ExpandedAgentCard
                agent={agent}
                snapshot={snapshot}
                onCollapse={() => setExpandedId(null)}
                onOpenAsPane={() => openInspectorPane(agent)}
              />
            </div>
          )}
        </div>
      </div>
    );
  };

  const selectedAgent = displayOrder.find((a) => a.id === selectedId);

  return (
    <div
      className="fleet-root"
      style={{
        position: 'fixed',
        top,
        left,
        right: 0,
        bottom: 0,
        zIndex: 150,
        background: 'var(--wks-bg-base)',
        display: 'flex',
        flexDirection: 'column',
      }}
    >
      {agentMenu.menu}
      {/* Overview chrome yields all available width and height to selected chat. */}
      {!chatAgent && (
        <div
          className="fleet-toolbar"
          style={{
            display: 'flex',
            alignItems: 'center',
            gap: 12,
            padding: '14px 22px 12px',
            borderBottom: '1px solid var(--wks-border-subtle)',
          }}
        >
          <div style={{ display: 'flex', alignItems: 'center', gap: 14, minWidth: 0 }}>
            <div
              style={{
                display: 'inline-flex',
                alignItems: 'center',
                gap: 8,
                fontSize: '1.05rem',
                fontWeight: 700,
                letterSpacing: '-0.01em',
                color: 'var(--wks-text-primary)',
              }}
            >
              <Radar size={17} strokeWidth={2.2} style={{ color: 'var(--wks-accent)' }} />
              Fleet
            </div>
            {/* Scannable status chips — dot + count, colour-keyed by state. */}
            <div
              style={{
                display: 'inline-flex',
                alignItems: 'center',
                gap: 12,
                fontSize: '0.72rem',
                fontVariantNumeric: 'tabular-nums',
                color: 'var(--wks-text-secondary)',
                whiteSpace: 'nowrap',
              }}
            >
              <StatChip color="var(--wks-text-tertiary)" glow={false}>
                {realAgents.length} agent{realAgents.length === 1 ? '' : 's'}
              </StatChip>
              <StatChip color="var(--wks-busy)" glow={working > 0}>
                {working} working
              </StatChip>
              <StatChip color="var(--wks-warning)" glow={counts.needsYou > 0}>
                {counts.needsYou} need{counts.needsYou === 1 ? 's' : ''} you
              </StatChip>
            </div>
          </div>
          <div style={{ flex: 1 }} />
          {/* Filter with a leading search glyph */}
          <div style={{ position: 'relative', display: 'inline-flex', alignItems: 'center' }}>
            <Search
              size={13}
              strokeWidth={2.2}
              style={{
                position: 'absolute',
                left: 9,
                color: 'var(--wks-text-faint)',
                pointerEvents: 'none',
              }}
            />
            <input
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="Filter agents…"
              spellCheck={false}
              style={{
                width: 168,
                fontSize: '0.72rem',
                fontFamily: 'inherit',
                padding: '6px 10px 6px 28px',
                borderRadius: 'var(--wks-radius-md)',
                // Border OR fill, never both — the field reads on --wks-bg-base
                // from its edge alone.
                border: '1px solid var(--wks-border-subtle)',
                background: 'transparent',
                color: 'var(--wks-text-primary)',
                transition: 'border-color 0.12s, box-shadow 0.12s',
              }}
            />
          </div>
          <button
            onClick={spawnAgent}
            title="Dispatch a new agent"
            style={{
              display: 'inline-flex',
              alignItems: 'center',
              gap: 6,
              fontSize: '0.72rem',
              fontFamily: 'inherit',
              fontWeight: 700,
              cursor: 'pointer',
              border: 'none',
              borderRadius: 'var(--wks-radius-md)',
              padding: '6px 13px',
              background: 'var(--wks-accent)',
              color: 'var(--wks-text-on-accent)',
              boxShadow: '0 1px 3px var(--wks-shadow)',
              transition: 'filter 0.12s',
            }}
            onMouseEnter={(e) => {
              (e.currentTarget as HTMLElement).style.filter = 'brightness(1.08)';
            }}
            onMouseLeave={(e) => {
              (e.currentTarget as HTMLElement).style.filter = '';
            }}
          >
            <span style={{ fontSize: '0.95rem', lineHeight: 1 }}>+</span> Dispatch agent
          </button>
          <button
            onClick={() => setViewLevel('piloting')}
            title="Back to agent (Esc)"
            style={{
              display: 'inline-flex',
              alignItems: 'center',
              gap: 6,
              fontSize: '0.72rem',
              fontFamily: 'inherit',
              fontWeight: 600,
              cursor: 'pointer',
              // Border only — it sits on --wks-bg-base beside the filled Dispatch
              // button, and the contrast between them is the hierarchy.
              border: '1px solid var(--wks-glass-border)',
              borderRadius: 'var(--wks-radius-md)',
              padding: '6px 12px',
              background: 'transparent',
              color: 'var(--wks-text-secondary)',
              transition: 'border-color 0.12s, color 0.12s',
            }}
            onMouseEnter={(e) => {
              (e.currentTarget as HTMLElement).style.color = 'var(--wks-text-primary)';
              (e.currentTarget as HTMLElement).style.borderColor = 'var(--wks-border)';
            }}
            onMouseLeave={(e) => {
              (e.currentTarget as HTMLElement).style.color = 'var(--wks-text-secondary)';
              (e.currentTarget as HTMLElement).style.borderColor = 'var(--wks-glass-border)';
            }}
          >
            <X size={13} strokeWidth={2} /> Exit fleet <kbd style={kbdStyle}>Esc</kbd>
          </button>
        </div>
      )}

      {chatAgent && (
        <div className="fleet-chat-layout">
          <main
            style={{ display: 'flex', flexDirection: 'column', flex: 1, minHeight: 0, minWidth: 0 }}
          >
            <div
              style={{
                display: 'flex',
                flexWrap: 'wrap',
                alignItems: 'center',
                gap: 8,
                padding: '8px 12px',
              }}
            >
              <SmallButton label="Back to fleet" onClick={backToFleet} />
              <strong style={{ flex: 1, minWidth: 0, overflowWrap: 'anywhere' }}>
                {chatAgent.name}
              </strong>
            </div>
            <FleetChatDestination
              sessionId={chatAgent.sessionId ?? chatAgent.lastSessionId ?? null}
              paneId={findAgentChatPane(chatAgent)?.id}
            />
            {!chatAgent.sessionId && <p style={{ padding: 12 }}>Session stopped.</p>}
          </main>
        </div>
      )}
      {!chatAgent && (
        <div style={{ display: 'none' }}>
          <FleetChatDestination sessionId={null} />
        </div>
      )}
      <div
        style={{
          display: chatAgent ? 'none' : 'flex',
          flexDirection: 'column',
          flex: 1,
          minHeight: 0,
        }}
      >
        <div ref={scrollRef} className="fleet-timeline-scroll">
          <div className="fleet-timeline-content">
            <header className="fleet-timeline-intro">
              <h1>Runbook timeline</h1>
              <p>Workers by last activity, newest first. Open a conversation to follow the work.</p>
            </header>
            <section aria-label="Fleet managers">
              {displayOrder.filter((agent) => agent.manager).map(renderAgent)}
            </section>
            <section aria-label="Worker timeline" className="fleet-worker-timeline">
              {displayOrder.filter((agent) => !agent.manager).map(renderAgent)}
              {!displayOrder.some((agent) => !agent.manager) && (
                <p className="fleet-timeline-empty">
                  {realAgents.length === 0
                    ? 'No agents in the fleet. Dispatch an agent to start.'
                    : query.trim()
                      ? 'No workers match this filter.'
                      : 'No workers in the fleet.'}
                </p>
              )}
            </section>
            {onOpenRecentAgents && (
              <button className="fleet-recent" onClick={onOpenRecentAgents}>
                <History size={20} />
                <span>
                  <strong>Recent agents</strong>
                  <span>Review earlier tasks and session history</span>
                </span>
                <ChevronRight size={16} />
              </button>
            )}
          </div>
        </div>
      </div>
      {/* Footer console — persistent, contextual keyboard affordances + the
          currently-selected agent. Moved out of the cramped header so hints stay
          visible without stealing header width. */}
      {!chatAgent && (
        <div
          style={{
            flexShrink: 0,
            display: 'flex',
            alignItems: 'center',
            gap: 16,
            padding: '7px 22px',
            borderTop: '1px solid var(--wks-border-subtle)',
            background: 'var(--wks-bg-surface)',
            fontSize: '0.66rem',
            color: 'var(--wks-text-faint)',
            whiteSpace: 'nowrap',
            overflow: 'hidden',
          }}
        >
          <Hint>
            <kbd style={kbdStyle}>{formatBinding(sc['fleet-list-down'] ?? '')}</kbd>
            <kbd style={kbdStyle}>{formatBinding(sc['fleet-list-up'] ?? '')}</kbd>
            <span>move</span>
          </Hint>
          <Hint>
            <kbd style={kbdStyle}>i</kbd>
            <span>inspect</span>
          </Hint>
          <Hint>
            <kbd style={kbdStyle}>{formatBinding(sc['fleet-approve-yes'] ?? '')}</kbd>
            <kbd style={kbdStyle}>{formatBinding(sc['fleet-approve-no'] ?? '')}</kbd>
            <span>approve</span>
          </Hint>
          <Hint>
            <kbd style={kbdStyle}>{formatBinding(sc['fleet-answer'] ?? '')}</kbd>
            <span>answer</span>
          </Hint>
          <div style={{ flex: 1, minWidth: 8 }} />
          {selectedAgent && (
            <span
              style={{
                display: 'inline-flex',
                alignItems: 'center',
                gap: 6,
                color: 'var(--wks-text-secondary)',
                minWidth: 0,
              }}
            >
              <span style={{ color: 'var(--wks-text-faint)' }}>Selected</span>
              <span
                style={{
                  fontWeight: 700,
                  color: 'var(--wks-text-primary)',
                  overflow: 'hidden',
                  textOverflow: 'ellipsis',
                }}
              >
                {selectedAgent.name}
              </span>
              <span style={{ display: 'inline-flex', alignItems: 'center', gap: 4 }}>
                <CornerDownLeft size={11} strokeWidth={2.2} /> chat
              </span>
            </span>
          )}
        </div>
      )}
    </div>
  );
};

/** Footer hint group: keys + a label, evenly spaced. */
const Hint: React.FC<{ children: React.ReactNode }> = ({ children }) => (
  <span style={{ display: 'inline-flex', alignItems: 'center', gap: 4 }}>{children}</span>
);

/** Header status chip — a state-coloured dot + count. Glows when the count is
 *  live (>0) so "2 working" / "1 needs you" read at a glance. */
const StatChip: React.FC<{ color: string; glow: boolean; children: React.ReactNode }> = ({
  color,
  glow,
  children,
}) => (
  <span style={{ display: 'inline-flex', alignItems: 'center', gap: 6 }}>
    <span
      style={{
        width: 7,
        height: 7,
        borderRadius: '50%',
        background: color,
        boxShadow: glow ? `0 0 7px ${color}` : 'none',
      }}
    />
    <span style={{ color: glow ? 'var(--wks-text-secondary)' : 'var(--wks-text-faint)' }}>
      {children}
    </span>
  </span>
);

/** Keycap: a tint, no outline. Ten of these sit in the footer at once and the
 *  "Exit fleet" one sits *inside* a bordered button, so the old border + fill +
 *  2px faux-3px bottom edge was three separation channels each — fill alone is
 *  the whole treatment now. The tint (not `--wks-bg-elevated`, which is within a
 *  couple of RGB steps of the footer's fill in the dark themes) keeps the cap
 *  legible on every surface it lands on. */
const kbdStyle: React.CSSProperties = {
  display: 'inline-flex',
  alignItems: 'center',
  justifyContent: 'center',
  minWidth: 16,
  height: 16,
  fontSize: '0.6rem',
  lineHeight: 1,
  color: 'var(--wks-text-secondary)',
  border: 'none',
  borderRadius: 'var(--wks-radius-sm)',
  background: 'color-mix(in srgb, var(--wks-text-primary) 7%, transparent)',
  padding: '0 4px',
  fontFamily: 'var(--wks-font-mono)',
};

export default FleetDeck;
