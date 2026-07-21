import React, { useEffect, useMemo, useRef, useState } from 'react';
import {
  X,
  Minimize2,
  ExternalLink,
  Search,
  Radar,
  CornerDownLeft,
  AlertTriangle,
  ChevronDown,
  ChevronUp,
  Compass,
} from 'lucide-react';
import { useVirtualizer } from '@tanstack/react-virtual';
import type { AgentWorkspace } from '../types/pane';
import type { ClaudeSessionSnapshot } from '../types/claudeSession';
import { useAttention } from '../contexts/AttentionContext';
import { AgentCard } from './AgentCard';
import { InspectorCard } from './claude/InspectorCard';
import { AgentLogo } from './agentLogos';
import { requestInspector } from '../lib/watchBus';
import { StatusGlyph } from './statusGlyph';
import { shortModelLabel } from '../lib/modelLabel';
import { agentAttentionScore } from '../lib/attentionRouter';
import {
  deriveSessionStats,
  planProgress,
  fmtUSD,
  ctxColor,
  isSnapshotStale,
} from '../lib/sessionStats';
import { useConfig } from '../hooks/useConfig';
import { DEFAULT_SHORTCUTS } from '../hooks/configDefaults';
import { eventMatchesCombo, digitFromRangeEvent, formatBinding } from '../lib/shortcuts';

const CARD_MIN = 360; // matches the old minmax(360px) grid
const GRID_GAP = 18;
const GRID_PAD_X = 22; // horizontal padding each side of the scroll area

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

/** Compact relative time for the list's "Active" column. */
function relTime(ts: number | undefined): string {
  if (!ts) return '—';
  const s = Math.max(0, Math.round((Date.now() - ts) / 1000));
  if (s < 60) return `${s}s`;
  const m = Math.round(s / 60);
  if (m < 60) return `${m}m`;
  const h = Math.round(m / 60);
  if (h < 24) return `${h}h`;
  return `${Math.round(h / 24)}d`;
}

interface Props {
  /** Inset so the deck sits inside the content area (right of sidebar, below navbar). */
  top: number;
  left: number;
}

/**
 * An agent card flipped in place into its live Inspector — the shared
 * {@link InspectorCard} fed the same `snapshotBySession` entry the collapsed card
 * uses, so it stays live for any agent (not just the piloted one). Sits in the
 * same grid cell as the collapsed AgentCard; collapse or "open as pane" from the
 * header. Height is fixed so the card's inner tab body scrolls.
 */
const ExpandedAgentCard: React.FC<{
  agent: AgentWorkspace;
  snapshot: ClaudeSessionSnapshot | undefined;
  onCollapse: () => void;
  onOpenAsPane: () => void;
}> = ({ agent, snapshot, onCollapse, onOpenAsPane }) => (
  <div
    onClick={(e) => e.stopPropagation()}
    style={{
      display: 'flex',
      flexDirection: 'column',
      height: 440,
      borderRadius: 'var(--wks-radius-lg)',
      overflow: 'hidden',
      background: 'var(--wks-bg-surface)',
      border: '1.5px solid var(--wks-accent)',
      boxShadow: '0 4px 16px var(--wks-shadow)',
    }}
  >
    <div
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: 8,
        padding: '9px 12px',
        borderBottom: '1px solid var(--wks-glass-border)',
        flexShrink: 0,
      }}
    >
      {agent.kind === 'supervisor' ? (
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
      <InspectorCard snapshot={snapshot} />
    </div>
  </div>
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
 * The Fleet — an advanced cross-agent radar. Every agent is a live
 * telemetry-face card, arranged by the Attention Router so the ones that need
 * you float to the front and pulse. Rendered as an overlay OVER the still-
 * mounted per-agent workspaces, so entering/leaving the deck never remounts a
 * pane: the agents keep running underneath, and clicking a card simply reveals
 * the one you picked (setActiveAgentId + viewLevel='piloting').
 */
const FleetDeck: React.FC<Props> = ({ top, left }) => {
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

  const realAgents = useMemo(() => agents.filter((a) => !a.global), [agents]);

  // Deck-scoped keybindings (fleet-*), remappable in Settings → Keybindings.
  // Defaults merged under user overrides so a partial saved map still binds.
  const { config } = useConfig();
  const sc = useMemo(
    () => ({ ...DEFAULT_SHORTCUTS, ...(config.keybindings?.shortcuts ?? {}) }),
    [config.keybindings?.shortcuts],
  );

  // Cards (default) vs a dense List table — mirrors the overview toggle.
  // Persisted so the deck reopens in the layout you last used.
  const [fleetView, setFleetView] = useState<'cards' | 'list'>(() => {
    try {
      return localStorage.getItem('wks-fleet-view') === 'list' ? 'list' : 'cards';
    } catch {
      return 'cards';
    }
  });
  const pickView = (v: 'cards' | 'list') => {
    setFleetView(v);
    try {
      localStorage.setItem('wks-fleet-view', v);
    } catch {
      /* private mode */
    }
  };
  const listScrollRef = useRef<HTMLDivElement>(null);

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

  // Agent → most-urgent open item, shared with the SideBar via the attention
  // feed (topByAgent) so both surfaces buoy cards by the same rule.
  const sorted = useMemo(() => {
    const q = query.trim().toLowerCase();
    const matches = (a: (typeof realAgents)[number]) =>
      !q || a.name.toLowerCase().includes(q) || (a.provider ?? 'claude').toLowerCase().includes(q);
    return realAgents.filter(matches).sort((a, b) => {
      const sa = agentAttentionScore(
        a.sessionId ? snapshotBySession[a.sessionId]?.ambientState : undefined,
        topByAgent.get(a.id)?.priority ?? 0,
      );
      const sb = agentAttentionScore(
        b.sessionId ? snapshotBySession[b.sessionId]?.ambientState : undefined,
        topByAgent.get(b.id)?.priority ?? 0,
      );
      return sb - sa;
    });
  }, [realAgents, snapshotBySession, topByAgent, query]);

  // List-view column sort. 'attn' keeps the needy-first order (the default); the
  // other keys sort by live stats. Cards always use the attention order.
  const [listSort, setListSort] = useState<{
    key: 'attn' | 'name' | 'ctx' | 'cost' | 'act';
    dir: 1 | -1;
  }>({ key: 'attn', dir: -1 });
  const toggleSort = (key: typeof listSort.key) =>
    setListSort((s) =>
      s.key === key ? { key, dir: s.dir === 1 ? -1 : 1 } : { key, dir: key === 'name' ? 1 : -1 },
    );
  const listRows = useMemo(() => {
    if (listSort.key === 'attn') return sorted;
    const keyOf = (a: (typeof sorted)[number]): number | string => {
      const snap = a.sessionId ? snapshotBySession[a.sessionId] : undefined;
      const st = deriveSessionStats(snap);
      switch (listSort.key) {
        case 'name':
          return a.name.toLowerCase();
        case 'ctx':
          return st.ctxPct ?? -1;
        case 'cost':
          return st.costUSD ?? -1;
        case 'act':
          return snap?.lastActivity ?? 0;
        default:
          return 0;
      }
    };
    return [...sorted].sort((a, b) => {
      const ka = keyOf(a);
      const kb = keyOf(b);
      const c =
        typeof ka === 'string' ? ka.localeCompare(kb as string) : (ka as number) - (kb as number);
      return c * listSort.dir;
    });
  }, [sorted, listSort, snapshotBySession]);
  // The order the user is actually looking at — keyboard nav + selection follow it.
  const displayOrder = fleetView === 'list' ? listRows : sorted;

  // Clickable, sortable list-column header. Click toggles direction; the active
  // column shows a caret.
  const SortBtn: React.FC<{ k: typeof listSort.key; label: string }> = ({ k, label }) => {
    const active = listSort.key === k;
    return (
      <button
        onClick={(e) => {
          e.stopPropagation();
          toggleSort(k);
        }}
        style={{
          background: 'none',
          border: 'none',
          padding: 0,
          font: 'inherit',
          cursor: 'pointer',
          color: active ? 'var(--wks-text-secondary)' : 'inherit',
          fontWeight: active ? 700 : 'inherit',
          display: 'inline-flex',
          alignItems: 'center',
          gap: 3,
        }}
      >
        {label}
        {active &&
          (listSort.dir === 1 ? (
            <ChevronUp size={10} strokeWidth={2.25} />
          ) : (
            <ChevronDown size={10} strokeWidth={2.25} />
          ))}
      </button>
    );
  };

  const working = realAgents.filter((a) => {
    const s = a.sessionId ? snapshotBySession[a.sessionId]?.ambientState : undefined;
    return s === 'thinking' || s === 'streaming' || s === 'background';
  }).length;

  // Windowed grid measurement (also feeds keyboard grid-nav): track the content
  // width so we can pack cards into rows of `cols` and move selection by row.
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

  // Card selection (needy-first order == `displayOrder`), with approve/answer
  // acting on the selected agent's top attention item — kept entirely within
  // the deck.
  const [selectedId, setSelectedId] = useState<string | null>(null);
  // The card flipped in place into its live InspectorCard (null = none).
  const [expandedId, setExpandedId] = useState<string | null>(null);
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
      const t = e.target as HTMLElement | null;
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
      if (e.key === 'Escape' && expandedId) {
        stop();
        setExpandedId(null);
        return;
      }
      // 'i' flips the focused card in place into its live InspectorCard (toggle).
      if (e.key === 'i' && idx >= 0) {
        stop();
        setExpandedId((cur) => (cur === displayOrder[idx].id ? null : displayOrder[idx].id));
        return;
      }

      // Movement adapts to the active fleet view: the Cards grid navigates
      // spatially (down = the card BELOW, one row of `cols` later), the List
      // linearly. Each view has its own bindings; arrows are fixed fallbacks.
      const select = (n: number) =>
        setSelectedId(displayOrder[Math.max(0, Math.min(displayOrder.length - 1, n))].id);
      if (fleetView === 'cards') {
        if (idx < 0) {
          // Nothing selected yet: any movement key just lands on the first card.
          if (
            ['ArrowDown', 'ArrowUp', 'ArrowLeft', 'ArrowRight'].includes(e.key) ||
            eventMatchesCombo(e, sc['fleet-cards-down']) ||
            eventMatchesCombo(e, sc['fleet-cards-up']) ||
            eventMatchesCombo(e, sc['fleet-cards-left']) ||
            eventMatchesCombo(e, sc['fleet-cards-right'])
          ) {
            stop();
            select(0);
            return;
          }
        } else {
          // Row moves are true vertical moves: below the grid they clamp into
          // the (possibly partial) last row; at the edge rows they no-op rather
          // than sliding sideways.
          const lastRowStart = Math.floor((displayOrder.length - 1) / cols) * cols;
          if (eventMatchesCombo(e, sc['fleet-cards-down']) || e.key === 'ArrowDown') {
            stop();
            if (idx < lastRowStart) select(idx + cols);
            return;
          }
          if (eventMatchesCombo(e, sc['fleet-cards-up']) || e.key === 'ArrowUp') {
            stop();
            if (idx >= cols) select(idx - cols);
            return;
          }
          if (eventMatchesCombo(e, sc['fleet-cards-left']) || e.key === 'ArrowLeft') {
            stop();
            select(idx - 1);
            return;
          }
          if (eventMatchesCombo(e, sc['fleet-cards-right']) || e.key === 'ArrowRight') {
            stop();
            select(idx + 1);
            return;
          }
        }
      } else {
        if (eventMatchesCombo(e, sc['fleet-list-down']) || e.key === 'ArrowDown') {
          stop();
          select((idx < 0 ? -1 : idx) + 1);
          return;
        }
        if (eventMatchesCombo(e, sc['fleet-list-up']) || e.key === 'ArrowUp') {
          stop();
          select((idx < 0 ? 1 : idx) - 1);
          return;
        }
      }

      if (idx < 0) return;
      const top = topByAgent.get(displayOrder[idx].id);
      if (!top) {
        if (eventMatchesCombo(e, sc['fleet-open'])) {
          stop();
          openAgent(displayOrder[idx].id);
        }
        return;
      }
      if (eventMatchesCombo(e, sc['fleet-open'])) {
        stop();
        openAgent(top.agentId);
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
    topByAgent,
    approve,
    answer,
    openAgent,
    sc,
    fleetView,
    cols,
  ]);

  // In list mode, keep the j/k-selected row visible as it moves.
  useEffect(() => {
    if (fleetView !== 'list' || !selectedId) return;
    listScrollRef.current
      ?.querySelector(`[data-fleet-row="${selectedId}"]`)
      ?.scrollIntoView({ block: 'nearest' });
  }, [selectedId, fleetView]);

  // Windowed grid: virtualize the packed rows — only on-screen cards (plus
  // overscan) are in the DOM, so a 50+-agent fleet stays smooth.
  const rowVirtualizer = useVirtualizer({
    count: Math.ceil(displayOrder.length / cols),
    getScrollElement: () => scrollRef.current,
    estimateSize: () => 290, // ~260 card min-height (chips + files line) + row gap; rows self-measure
    overscan: 2,
  });

  // In card mode, keep the j/k-selected card visible by scrolling its row into
  // view (the list mode has its own scroll effect above).
  useEffect(() => {
    if (fleetView !== 'cards' || !selectedId) return;
    const i = displayOrder.findIndex((a) => a.id === selectedId);
    if (i >= 0) rowVirtualizer.scrollToIndex(Math.floor(i / cols), { align: 'auto' });
  }, [selectedId, fleetView, displayOrder, cols, rowVirtualizer]);

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
      {/* Deck header */}
      <div
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
              fontSize: '1.1rem',
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
              padding: '5px 9px 5px 28px',
              borderRadius: 'var(--wks-radius-md)',
              border: '1px solid var(--wks-border-subtle)',
              background: 'var(--wks-bg-surface)',
              color: 'var(--wks-text-primary)',
              transition: 'border-color 0.12s, box-shadow 0.12s',
            }}
          />
        </div>
        {/* Cards / List toggle */}
        <div
          style={{
            display: 'flex',
            background: 'var(--wks-bg-surface)',
            border: '1px solid var(--wks-border-subtle)',
            borderRadius: 'var(--wks-radius-md)',
            padding: 3,
          }}
        >
          <SegBtn active={fleetView === 'cards'} onClick={() => pickView('cards')}>
            Cards
          </SegBtn>
          <SegBtn active={fleetView === 'list'} onClick={() => pickView('list')}>
            List
          </SegBtn>
        </div>
        <button
          onClick={spawnAgent}
          title="Spawn a new agent"
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
          <span style={{ fontSize: '0.95rem', lineHeight: 1 }}>+</span> Spawn agent
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
            border: '1px solid var(--wks-glass-border)',
            borderRadius: 'var(--wks-radius-md)',
            padding: '5px 12px',
            background: 'var(--wks-bg-surface)',
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

      {/* Content: empty state · dense list · windowed card grid */}
      {realAgents.length === 0 ? (
        <div style={CONTENT_SCROLL}>
          <div
            style={{
              marginTop: 80,
              display: 'flex',
              flexDirection: 'column',
              alignItems: 'center',
              textAlign: 'center',
              color: 'var(--wks-text-faint)',
            }}
          >
            <div
              style={{
                display: 'flex',
                alignItems: 'center',
                justifyContent: 'center',
                width: 56,
                height: 56,
                borderRadius: '50%',
                marginBottom: 18,
                background: 'var(--wks-bg-surface)',
                border: '1px solid var(--wks-border-subtle)',
                color: 'var(--wks-text-tertiary)',
              }}
            >
              <Radar size={26} strokeWidth={1.8} />
            </div>
            <div
              style={{ fontSize: '0.9rem', fontWeight: 600, color: 'var(--wks-text-secondary)' }}
            >
              No agents in the fleet
            </div>
            <div style={{ fontSize: '0.78rem', marginTop: 6, maxWidth: 300 }}>
              Spawn an agent and it'll appear here as a live card.
            </div>
            <button
              onClick={spawnAgent}
              style={{
                marginTop: 18,
                fontSize: '0.8rem',
                fontFamily: 'inherit',
                fontWeight: 700,
                cursor: 'pointer',
                background: 'var(--wks-accent)',
                color: 'var(--wks-text-on-accent)',
                border: 'none',
                borderRadius: 'var(--wks-radius-md)',
                padding: '8px 16px',
                boxShadow: '0 1px 3px var(--wks-shadow)',
              }}
            >
              + Spawn agent
            </button>
          </div>
        </div>
      ) : fleetView === 'list' ? (
        <div ref={listScrollRef} style={CONTENT_SCROLL}>
          <table style={{ width: '100%', borderCollapse: 'collapse', fontSize: '0.74rem' }}>
            <thead>
              <tr style={{ color: 'var(--wks-text-faint)', textAlign: 'left' }}>
                <th style={lth}>
                  <SortBtn k="name" label="Agent" />
                </th>
                <th style={lth}>Status</th>
                <th style={lth}>Model</th>
                <th style={lthNum}>
                  <SortBtn k="ctx" label="Context" />
                </th>
                <th style={lthNum}>
                  <SortBtn k="cost" label="Cost" />
                </th>
                <th style={lthNum}>Plan</th>
                <th style={lthNum}>
                  <SortBtn k="act" label="Active" />
                </th>
              </tr>
            </thead>
            <tbody>
              {displayOrder.map((agent) => {
                const snap = agent.sessionId ? snapshotBySession[agent.sessionId] : undefined;
                const vis = listStateVisual(agent.sessionId ? snap?.ambientState : undefined);
                const stats = deriveSessionStats(snap);
                const plan = planProgress(snap?.plan);
                const sel = selectedId === agent.id;
                return (
                  <tr
                    key={agent.id}
                    data-fleet-row={agent.id}
                    onMouseDown={() => setSelectedId(agent.id)}
                    onClick={() => openAgent(agent.id)}
                    title={`${agent.name} — ${vis.label}\n${agent.cwd ?? ''}`}
                    style={{
                      cursor: 'pointer',
                      borderTop: '1px solid var(--wks-border-subtle)',
                      background: sel ? 'var(--wks-bg-selected)' : 'transparent',
                    }}
                    onMouseEnter={(e) => {
                      if (!sel)
                        (e.currentTarget as HTMLElement).style.background = 'var(--wks-bg-hover)';
                    }}
                    onMouseLeave={(e) => {
                      if (!sel) (e.currentTarget as HTMLElement).style.background = 'transparent';
                    }}
                  >
                    <td
                      style={sel ? { ...ltd, boxShadow: `inset 3px 0 0 var(--wks-accent)` } : ltd}
                    >
                      <span
                        style={{
                          display: 'inline-flex',
                          alignItems: 'center',
                          gap: 8,
                          minWidth: 0,
                        }}
                      >
                        <span
                          style={{
                            width: 8,
                            height: 8,
                            borderRadius: '50%',
                            flexShrink: 0,
                            background: vis.color,
                            boxShadow: vis.glow ? `0 0 8px ${vis.color}` : 'none',
                          }}
                        />
                        <span
                          style={{
                            fontWeight: 600,
                            color: 'var(--wks-text-primary)',
                            overflow: 'hidden',
                            textOverflow: 'ellipsis',
                            whiteSpace: 'nowrap',
                          }}
                        >
                          {agent.kind === 'supervisor' && (
                            <Compass
                              size={11}
                              strokeWidth={2}
                              style={{ flexShrink: 0, marginRight: 4, verticalAlign: '-1px' }}
                            />
                          )}
                          {agent.name}
                        </span>
                      </span>
                    </td>
                    <td style={ltd}>
                      <span
                        style={{
                          display: 'inline-flex',
                          alignItems: 'center',
                          gap: 4,
                          fontSize: '0.66rem',
                          fontWeight: 600,
                          color: vis.color,
                          border: `1px solid ${vis.color}`,
                          borderRadius: 'var(--wks-radius-pill)',
                          padding: '1px 9px',
                          whiteSpace: 'nowrap',
                        }}
                      >
                        <StatusGlyph
                          state={agent.sessionId ? snap?.ambientState : undefined}
                          size={12}
                          strokeWidth={2.2}
                          accent="currentColor"
                        />
                        {vis.label}
                      </span>
                    </td>
                    <td style={{ ...ltd, color: 'var(--wks-text-secondary)' }}>
                      {stats.model ? shortModelLabel(stats.model) : '—'}
                    </td>
                    <td style={ltdNum}>
                      {stats.ctxPct !== undefined ? (
                        <span style={{ color: ctxColor(stats.ctxPct), fontWeight: 600 }}>
                          {Math.round(stats.ctxPct)}%
                        </span>
                      ) : (
                        '—'
                      )}
                    </td>
                    <td style={{ ...ltdNum, color: 'var(--wks-accent)' }}>
                      {stats.costUSD !== undefined ? fmtUSD(stats.costUSD) : '—'}
                    </td>
                    <td style={ltdNum}>
                      {plan ? (
                        <span
                          title={plan.active?.activeForm ?? plan.active?.content ?? 'Plan progress'}
                          style={{
                            color:
                              plan.done >= plan.total
                                ? 'var(--wks-success)'
                                : 'var(--wks-text-secondary)',
                            fontWeight: 600,
                          }}
                        >
                          {plan.done}/{plan.total}
                        </span>
                      ) : (
                        '—'
                      )}
                    </td>
                    {isSnapshotStale(snap?.ambientState, snap?.lastActivity, now) ? (
                      <td
                        style={{ ...ltdNum, color: 'var(--wks-warning)', fontWeight: 700 }}
                        title="Says it's working but nothing has arrived — the stream may have stalled."
                      >
                        <span style={{ display: 'inline-flex', alignItems: 'center', gap: 3 }}>
                          <AlertTriangle size={10} strokeWidth={2.25} />
                          {relTime(snap?.lastActivity)}
                        </span>
                      </td>
                    ) : (
                      <td style={ltdNum}>{relTime(snap?.lastActivity)}</td>
                    )}
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      ) : (
        <div ref={scrollRef} style={CONTENT_SCROLL}>
          <div
            style={{ position: 'relative', width: '100%', height: rowVirtualizer.getTotalSize() }}
          >
            {rowVirtualizer.getVirtualItems().map((vr) => {
              const start = vr.index * cols;
              const rowAgents = displayOrder.slice(start, start + cols);
              return (
                <div
                  key={vr.key}
                  data-index={vr.index}
                  ref={rowVirtualizer.measureElement}
                  style={{
                    position: 'absolute',
                    top: 0,
                    left: 0,
                    width: '100%',
                    transform: `translateY(${vr.start}px)`,
                    display: 'grid',
                    gridTemplateColumns: `repeat(${cols}, minmax(0, 1fr))`,
                    gap: GRID_GAP,
                    paddingBottom: GRID_GAP,
                    alignItems: 'start',
                  }}
                >
                  {rowAgents.map((agent) => (
                    <div
                      key={agent.id}
                      onMouseDown={() => setSelectedId(agent.id)}
                      style={{
                        borderRadius: 'var(--wks-radius-lg)',
                        outline:
                          selectedId === agent.id
                            ? '2px solid var(--wks-accent)'
                            : '2px solid transparent',
                        outlineOffset: 2,
                        transition: 'outline-color 0.12s',
                      }}
                    >
                      {expandedId === agent.id ? (
                        <ExpandedAgentCard
                          agent={agent}
                          snapshot={
                            agent.sessionId ? snapshotBySession[agent.sessionId] : undefined
                          }
                          onCollapse={() => setExpandedId(null)}
                          onOpenAsPane={() => openInspectorPane(agent)}
                        />
                      ) : (
                        <AgentCard
                          agent={agent}
                          snapshot={
                            agent.sessionId ? snapshotBySession[agent.sessionId] : undefined
                          }
                          onInspect={() => setExpandedId(agent.id)}
                        />
                      )}
                    </div>
                  ))}
                </div>
              );
            })}
          </div>
        </div>
      )}

      {/* Footer console — persistent, contextual keyboard affordances + the
          currently-selected agent. Moved out of the cramped header so hints stay
          visible without stealing header width. */}
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
          {fleetView === 'cards' ? (
            <>
              <kbd style={kbdStyle}>{formatBinding(sc['fleet-cards-left'] ?? '')}</kbd>
              <kbd style={kbdStyle}>{formatBinding(sc['fleet-cards-down'] ?? '')}</kbd>
              <kbd style={kbdStyle}>{formatBinding(sc['fleet-cards-up'] ?? '')}</kbd>
              <kbd style={kbdStyle}>{formatBinding(sc['fleet-cards-right'] ?? '')}</kbd>
            </>
          ) : (
            <>
              <kbd style={kbdStyle}>{formatBinding(sc['fleet-list-down'] ?? '')}</kbd>
              <kbd style={kbdStyle}>{formatBinding(sc['fleet-list-up'] ?? '')}</kbd>
            </>
          )}
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
              <CornerDownLeft size={11} strokeWidth={2.2} /> open
            </span>
          </span>
        )}
      </div>
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

const kbdStyle: React.CSSProperties = {
  display: 'inline-flex',
  alignItems: 'center',
  justifyContent: 'center',
  minWidth: 15,
  height: 15,
  fontSize: '0.64rem',
  lineHeight: 1,
  color: 'var(--wks-text-secondary)',
  border: '1px solid var(--wks-glass-border)',
  borderBottomWidth: 2,
  borderRadius: 'var(--wks-radius-sm)',
  background: 'var(--wks-bg-elevated)',
  padding: '0 4px',
  fontFamily: 'var(--wks-font-mono)',
};

const CONTENT_SCROLL: React.CSSProperties = {
  flex: 1,
  overflowY: 'auto',
  padding: '6px 22px 28px',
};

/** Segmented Cards/List toggle button (mockup style). */
const SegBtn: React.FC<{ active: boolean; onClick: () => void; children: React.ReactNode }> = ({
  active,
  onClick,
  children,
}) => (
  <button
    onClick={onClick}
    style={{
      border: 'none',
      borderRadius: 'var(--wks-radius-sm)',
      padding: '5px 13px',
      cursor: 'pointer',
      fontFamily: 'inherit',
      fontSize: '0.72rem',
      fontWeight: 600,
      background: active ? 'var(--wks-bg-selected)' : 'transparent',
      color: active ? 'var(--wks-text-primary)' : 'var(--wks-text-faint)',
      boxShadow: active ? '0 1px 2px var(--wks-shadow)' : 'none',
      transition: 'color 0.12s',
    }}
  >
    {children}
  </button>
);

/** Status dot/pill colour + label for the list row. `busy` (thinking/streaming)
 *  uses the dedicated busy token so it matches the cards and sidebar. */
function listStateVisual(s: string | undefined): { color: string; label: string; glow: boolean } {
  switch (s) {
    case 'waiting_approval':
      return { color: 'var(--wks-warning)', label: 'Needs approval', glow: true };
    case 'waiting_input':
      return { color: 'var(--wks-warning)', label: 'Waiting', glow: true };
    case 'thinking':
      return {
        color: 'var(--wks-busy)',
        label: 'Thinking',
        glow: true,
      };
    case 'streaming':
      return { color: 'var(--wks-busy)', label: 'Working', glow: true };
    case 'background':
      return {
        color: 'var(--wks-busy)',
        label: 'Background work',
        glow: false,
      };
    case 'idle':
      return { color: 'var(--wks-success)', label: 'Idle', glow: false };
    default:
      return { color: 'var(--wks-text-faint)', label: 'Stopped', glow: false };
  }
}

const lth: React.CSSProperties = {
  position: 'sticky',
  top: 0,
  zIndex: 2,
  background: 'var(--wks-bg-base)',
  boxShadow: 'inset 0 -1px 0 var(--wks-border-subtle)',
  padding: '7px 10px',
  fontWeight: 700,
  textTransform: 'uppercase',
  letterSpacing: '0.06em',
  fontSize: '0.64rem',
};
const lthNum: React.CSSProperties = { ...lth, textAlign: 'right' };
const ltd: React.CSSProperties = {
  padding: '8px 10px',
  whiteSpace: 'nowrap',
  overflow: 'hidden',
  textOverflow: 'ellipsis',
  maxWidth: 280,
};
const ltdNum: React.CSSProperties = {
  ...ltd,
  textAlign: 'right',
  fontVariantNumeric: 'tabular-nums',
  color: 'var(--wks-text-secondary)',
};

export default FleetDeck;
