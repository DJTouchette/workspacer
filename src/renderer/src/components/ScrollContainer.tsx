import React, { useRef, useState, useCallback, useEffect, useImperativeHandle, forwardRef, Suspense } from 'react';
import Pane from './Pane';
import { PaneConfig, PaneType, TabConfig, CanvasRect, ViewMode } from '../types/pane';
import TerminalPane from '../panes/TerminalPane';
import ClaudePane from '../panes/ClaudePane';
import { useConfig } from '../hooks/useConfig';

// Lazy-load pane types that aren't needed on initial render
const BrowserPane = React.lazy(() => import('../panes/BrowserPane'));
const NotesPane = React.lazy(() => import('../panes/NotesPane'));
const AgentPane = React.lazy(() => import('../panes/AgentPane'));
const SettingsPane = React.lazy(() => import('../panes/SettingsPane'));
const ReviewPane = React.lazy(() => import('../panes/ReviewPane'));
const PluginsManagerPane = React.lazy(() => import('../panes/PluginsManagerPane'));
const OverviewPane = React.lazy(() => import('../panes/OverviewPane'));
const LibraryPane = React.lazy(() => import('../panes/LibraryPane'));
const AnalyticsPane = React.lazy(() => import('../panes/AnalyticsPane'));

// --- Spatial-canvas constants -------------------------------------------------
const CARD_HEADER_H = 26;        // drag-handle strip atop each spatial card
const ZOOM_MIN = 0.25;
const ZOOM_MAX = 2.5;
// Default grid slot for a card that has never been placed.
const DEF_COLS = 3;
const DEF_W = 560;
const DEF_H = 400;
const DEF_GAP = 28;

function defaultCanvas(index: number): CanvasRect {
  const col = index % DEF_COLS;
  const row = Math.floor(index / DEF_COLS);
  return {
    x: 40 + col * (DEF_W + DEF_GAP),
    y: 40 + row * (DEF_H + DEF_GAP),
    w: DEF_W,
    h: DEF_H,
  };
}

const clamp = (v: number, lo: number, hi: number) => Math.max(lo, Math.min(hi, v));

// --- Timeline (vertical activity feed) constants ------------------------------
const TL_GUTTER = 96;   // left time-label column
const TL_TOP = 28;      // top padding
const TL_RIGHT = 28;    // right margin
const TL_CARD_H = 360;  // fixed card height
const TL_GAP = 24;      // vertical gap between cards
const TL_MIN_W = 360;

// Tabs sorted newest-activity-first for the timeline. Stable: tabs sharing a
// timestamp (or both missing one) keep their existing order.
function timelineSorted(tabs: TabConfig[]): TabConfig[] {
  return tabs
    .map((tab, index) => ({ tab, index }))
    .sort((a, b) => {
      const ta = a.tab.lastActiveAt ?? -Infinity;
      const tb = b.tab.lastActiveAt ?? -Infinity;
      if (ta !== tb) return tb - ta;
      return a.index - b.index;
    })
    .map((e) => e.tab);
}

function formatRelative(ts?: number): string {
  if (!ts) return '—';
  const diff = Date.now() - ts;
  if (diff < 0) return 'now';
  const s = Math.floor(diff / 1000);
  if (s < 45) return 'now';
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m ago`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h}h ago`;
  const d = Math.floor(h / 24);
  if (d < 7) return `${d}d ago`;
  try { return new Date(ts).toLocaleDateString(); } catch { return '—'; }
}

const PaneFallback = () => (
  <div style={{ width: '100%', height: '100%', display: 'flex', alignItems: 'center', justifyContent: 'center', backgroundColor: 'var(--wks-bg-base)', color: 'var(--wks-text-muted)', fontSize: '0.8rem' }}>
    Loading…
  </div>
);

interface ScrollContainerProps {
  tabs: TabConfig[];
  activeTabId: string;
  onTabFocus: (tabId: string) => void;
  onPaneClose: (tabId: string, paneId: string) => void;
  onPaneFocus: (tabId: string, paneId: string) => void;
  onTabRename?: (tabId: string, title: string) => void;
  onTabMove?: (tabId: string, toIndex: number) => void;
  onPtyReady?: (paneId: string, ptySessionId: string) => void;
  onUrlChange?: (tabId: string, paneId: string, url: string) => void;
  onNavigateToTab?: (tabId: string) => void;
  onAddTab?: (type: PaneType, shell?: string, label?: string, cwd?: string, profileId?: string, resumeSessionId?: string, attachSessionId?: string) => void;
  /** paneId → ptySessionId. For Claude panes, ptySessionId === Claude session id. */
  ptyMapping?: Record<string, string>;
  renameSignal?: number;
  /** Global layout paradigm. 'tabs' = horizontal strip; 'spatial' = canvas. */
  viewMode?: ViewMode;
  /** Persist a tab card's spatial placement (only fires in 'spatial' mode). */
  onTabCanvasChange?: (tabId: string, canvas: CanvasRect) => void;
  /**
   * Whether this container's agent is the one currently shown. Every agent's
   * container stays mounted (so terminals/scrollback survive agent switches);
   * inactive ones are hidden. When false, no pane counts as active, so off-
   * screen panes throttle their updates and skip terminal refits.
   */
  agentActive?: boolean;
  /** workspacer's own agents (for the Overview pane to scope stats to them,
   *  not every Claude session claudemon tracks machine-wide). */
  workspaceAgents?: { sessionId?: string }[];
  /** Fallback project root for the Library pane (the app's cwd). */
  appCwd?: string;
}

export interface ScrollContainerRef {
  scrollToTab: (id: string) => void;
}

interface PaneCallbacks {
  onPtyReady?: (paneId: string, ptySessionId: string) => void;
  onUrlChange?: (paneId: string, url: string) => void;
  tabs?: TabConfig[];
  onNavigateToTab?: (tabId: string) => void;
  onAddTab?: (type: PaneType, shell?: string, label?: string, cwd?: string, profileId?: string, resumeSessionId?: string, attachSessionId?: string) => void;
  ptyMapping?: Record<string, string>;
  workspaceAgents?: { sessionId?: string }[];
  appCwd?: string;
}

function renderPaneContent(pane: PaneConfig, isActive: boolean, callbacks: PaneCallbacks) {
  switch (pane.type) {
    case 'terminal':
      return <TerminalPane paneId={pane.id} title={pane.title} isActive={isActive} shell={pane.shell} cwd={pane.cwd} initialCommand={pane.initialCommand} onPtyReady={callbacks.onPtyReady} />;
    case 'claude':
      return <ClaudePane paneId={pane.id} title={pane.title} isActive={isActive} cwd={pane.cwd} profileId={pane.profileId} resumeSessionId={pane.resumeSessionId} attachSessionId={pane.attachSessionId} initialPrompt={pane.initialPrompt} onPtyReady={callbacks.onPtyReady} />;
    case 'browser':
      return (
        <Suspense fallback={<PaneFallback />}>
          <BrowserPane paneId={pane.id} title={pane.title} isActive={isActive} initialUrl={pane.url} appMode={pane.appMode} hibernated={pane.hibernated} onUrlChange={(url) => callbacks.onUrlChange?.(pane.id, url)} />
        </Suspense>
      );
    case 'notes':
      return <Suspense fallback={<PaneFallback />}><NotesPane title={pane.title} /></Suspense>;
    case 'agent':
      return <Suspense fallback={<PaneFallback />}><AgentPane title={pane.title} /></Suspense>;
    case 'settings':
      return <Suspense fallback={<PaneFallback />}><SettingsPane title={pane.title} /></Suspense>;
    case 'review':
      return (
        <Suspense fallback={<PaneFallback />}>
          <ReviewPane paneId={pane.id} title={pane.title} isActive={isActive} cwd={pane.cwd} />
        </Suspense>
      );
    case 'plugin':
      // A plugin-injected pane: a webview onto the plugin sidecar's own UI.
      return (
        <Suspense fallback={<PaneFallback />}>
          <BrowserPane paneId={pane.id} title={pane.title} isActive={isActive} initialUrl={pane.url || 'about:blank'} appMode={true} hibernated={pane.hibernated} onUrlChange={() => {}} />
        </Suspense>
      );
    case 'plugins':
      return <Suspense fallback={<PaneFallback />}><PluginsManagerPane title={pane.title} /></Suspense>;
    case 'overview':
      return <Suspense fallback={<PaneFallback />}><OverviewPane title={pane.title} agents={callbacks.workspaceAgents} /></Suspense>;
    case 'library':
      return <Suspense fallback={<PaneFallback />}><LibraryPane title={pane.title} cwd={pane.cwd || callbacks.appCwd} /></Suspense>;
    case 'analytics':
      return <Suspense fallback={<PaneFallback />}><AnalyticsPane title={pane.title} /></Suspense>;
    default:
      return <div>Unknown pane type</div>;
  }
}

// Auto-tiling layout for the panes within a tab. Handles 1..N panes through a
// single structure so that adding/removing a pane never re-parents (and thus
// never remounts → kills the PTY of) an existing pane.
function TilingLayout({
  panes,
  activePaneId,
  agentActive,
  forceLive,
  containerWidth,
  containerHeight,
  onPaneClose,
  onPaneFocus,
  callbacks,
  isActiveTab,
  tabTitle,
  onTabFocus,
  onTabMove,
  onTabRename,
  renameSignal,
}: {
  panes: PaneConfig[];
  activePaneId: string;
  agentActive: boolean;
  /** Spatial mode: every visible card's panes should stay live (rendered/refit),
   *  not just the focused tab's. */
  forceLive: boolean;
  containerWidth: number;
  containerHeight: number;
  onPaneClose: (paneId: string) => void;
  onPaneFocus: (paneId: string) => void;
  callbacks: PaneCallbacks;
  // Tab-level props used only when the tab has a single pane (headerless,
  // labelled by the tab itself and renamable/movable like the old layout).
  isActiveTab: boolean;
  tabTitle: string;
  onTabFocus: () => void;
  onTabMove?: (delta: number) => void;
  onTabRename?: (title: string) => void;
  renameSignal?: number;
}) {
  const single = panes.length === 1;
  const count = panes.length;
  const cols = count <= 1 ? 1 : count <= 2 ? 2 : count <= 4 ? 2 : count <= 6 ? 3 : Math.ceil(Math.sqrt(count));
  const rows = Math.ceil(count / cols);

  const layouts: Array<{ col: number; row: number; colSpan: number; rowSpan: number }> = [];

  if (count === 1) {
    layouts.push({ col: 0, row: 0, colSpan: 1, rowSpan: 1 });
  } else if (count === 2) {
    layouts.push({ col: 0, row: 0, colSpan: 1, rowSpan: 1 });
    layouts.push({ col: 1, row: 0, colSpan: 1, rowSpan: 1 });
  } else if (count === 3) {
    layouts.push({ col: 0, row: 0, colSpan: 1, rowSpan: 2 });
    layouts.push({ col: 1, row: 0, colSpan: 1, rowSpan: 1 });
    layouts.push({ col: 1, row: 1, colSpan: 1, rowSpan: 1 });
  } else {
    for (let i = 0; i < count; i++) {
      layouts.push({ col: i % cols, row: Math.floor(i / cols), colSpan: 1, rowSpan: 1 });
    }
    const lastRowStart = Math.floor((count - 1) / cols) * cols;
    const lastRowCount = count - lastRowStart;
    if (lastRowCount < cols) {
      layouts[count - 1].colSpan = cols - lastRowCount + 1;
    }
  }

  const cellWidth = containerWidth / cols;
  const cellHeight = containerHeight / rows;
  const gap = 2;

  return (
    <>
      {panes.map((pane, idx) => {
        const layout = layouts[idx];
        if (!layout) return null;
        // Liveness (drives throttling/focus) requires the agent to be on
        // screen; in spatial mode every visible card stays live, otherwise only
        // the focused tab (single) or focused pane (multi) is live.
        const liveActive = agentActive && (forceLive ? true : (single ? isActiveTab : pane.id === activePaneId));
        // Visual focus highlight: single-pane tracks the active tab; multi-pane
        // highlights the active pane of the active tab.
        const isActive = single ? isActiveTab : (pane.id === activePaneId && isActiveTab);
        // Single pane fills the whole tab area (and stays headerless, labelled
        // by the tab). Multi-pane uses the computed grid cell.
        const cellStyle: React.CSSProperties = single
          ? { position: 'absolute', inset: 0 }
          : {
              position: 'absolute',
              left: `${layout.col * cellWidth + gap}px`,
              top: `${layout.row * cellHeight + gap}px`,
              width: `${layout.colSpan * cellWidth - gap * 2}px`,
              height: `${layout.rowSpan * cellHeight - gap * 2}px`,
            };

        return (
          <div key={pane.id} style={cellStyle}>
            <Pane
              id={pane.id}
              type={pane.type}
              title={single ? tabTitle : pane.title}
              isActive={isActive}
              onClose={() => onPaneClose(pane.id)}
              onFocus={single ? onTabFocus : () => onPaneFocus(pane.id)}
              onMove={single && onTabMove ? (_, delta) => onTabMove(delta) : undefined}
              onRename={single && onTabRename ? (_, title) => onTabRename(title) : undefined}
              renameSignal={single ? renameSignal : undefined}
              hideHeader={single}
              hideActiveBorder={single}
            >
              {renderPaneContent(pane, liveActive, callbacks)}
            </Pane>
          </div>
        );
      })}
    </>
  );
}

// Tracks an in-progress canvas interaction (panning the view, or moving/resizing
// a card) via window-level listeners so the drag continues outside the element.
interface Interaction {
  kind: 'pan' | 'move' | 'resize';
  tabId?: string;
  startClientX: number;
  startClientY: number;
  origin: CanvasRect | { x: number; y: number }; // pan: {x,y}; card: full rect
  zoom: number;
}

const ScrollContainer = forwardRef<ScrollContainerRef, ScrollContainerProps>(
  ({ tabs, activeTabId, onTabFocus, onPaneClose, onPaneFocus, onTabRename, onTabMove, onPtyReady, onUrlChange, onNavigateToTab, onAddTab, ptyMapping, renameSignal, viewMode = 'tabs', onTabCanvasChange, agentActive = true, workspaceAgents, appCwd }, ref) => {
    const containerRef = useRef<HTMLDivElement>(null);
    const { config } = useConfig();
    const peek = config.panes?.peek ?? 80;
    const gap = config.panes?.gap ?? 16;
    const spatial = viewMode === 'spatial';
    const timeline = viewMode === 'timeline';

    const [tabWidth, setTabWidth] = useState(800);
    const [containerHeight, setContainerHeight] = useState(600);
    const [containerWidth, setContainerWidth] = useState(1000);

    // --- Spatial view state ---------------------------------------------------
    const [pan, setPan] = useState({ x: 0, y: 0 });
    const [zoom, setZoom] = useState(1);
    // Live (un-persisted) rect of the card currently being dragged/resized, so it
    // tracks the cursor smoothly before we commit on mouse-up.
    const [liveRect, setLiveRect] = useState<{ tabId: string; rect: CanvasRect } | null>(null);
    const interactionRef = useRef<Interaction | null>(null);

    useEffect(() => {
      const container = containerRef.current;
      if (!container) return;

      const updateSize = () => {
        const w = container.clientWidth - 2 * peek - gap;
        setTabWidth(Math.max(400, w));
        setContainerWidth(container.clientWidth);
        setContainerHeight(container.clientHeight - 16);
      };

      updateSize();
      const observer = new ResizeObserver(updateSize);
      observer.observe(container);
      return () => observer.disconnect();
    }, [peek, gap]);

    // Resolve a tab's spatial rect: live drag override → persisted → default slot.
    const rectFor = useCallback((tab: TabConfig, index: number): CanvasRect => {
      if (liveRect && liveRect.tabId === tab.id) return liveRect.rect;
      return tab.canvas ?? defaultCanvas(index);
    }, [liveRect]);

    const scrollToTab = useCallback((id: string) => {
      const container = containerRef.current;
      if (!container) return;

      if (spatial) {
        // Pan the canvas so the target card is centred in the viewport.
        const index = tabs.findIndex((t) => t.id === id);
        if (index < 0) return;
        const tab = tabs[index];
        const rect = tab.canvas ?? defaultCanvas(index);
        const cx = container.clientWidth / 2;
        const cy = container.clientHeight / 2;
        setPan({
          x: cx - (rect.x + rect.w / 2) * zoom,
          y: cy - (rect.y + rect.h / 2) * zoom,
        });
        return;
      }

      if (timeline) {
        const order = timelineSorted(tabs);
        const pos = order.findIndex((t) => t.id === id);
        if (pos < 0) return;
        const y = TL_TOP + pos * (TL_CARD_H + TL_GAP);
        container.scrollTo({ top: Math.max(0, y - 24), behavior: 'instant' });
        return;
      }

      const tabEl = container.querySelector(`[data-tab-id="${id}"]`) as HTMLElement | null;
      if (!tabEl) return;
      const containerRect = container.getBoundingClientRect();
      const tabRect = tabEl.getBoundingClientRect();
      const scrollLeft = tabEl.offsetLeft - containerRect.width / 2 + tabRect.width / 2;
      container.scrollTo({ left: scrollLeft, behavior: 'instant' });
    }, [spatial, timeline, tabs, zoom]);

    useImperativeHandle(ref, () => ({ scrollToTab }), [scrollToTab]);

    // Detect which tab is most visible after scroll ends (tabs mode only).
    useEffect(() => {
      const container = containerRef.current;
      if (!container || viewMode !== 'tabs') return;

      let scrollTimeout: ReturnType<typeof setTimeout>;

      const handleScrollEnd = () => {
        clearTimeout(scrollTimeout);
        scrollTimeout = setTimeout(() => {
          const containerCenter = container.scrollLeft + container.clientWidth / 2;
          let closestId = tabs[0]?.id;
          let closestDist = Infinity;

          for (const tab of tabs) {
            const el = container.querySelector(`[data-tab-id="${tab.id}"]`) as HTMLElement | null;
            if (!el) continue;
            const tabCenter = el.offsetLeft + el.offsetWidth / 2;
            const dist = Math.abs(containerCenter - tabCenter);
            if (dist < closestDist) {
              closestDist = dist;
              closestId = tab.id;
            }
          }

          if (closestId && closestId !== activeTabId) {
            onTabFocus(closestId);
          }
        }, 100);
      };

      container.addEventListener('scroll', handleScrollEnd, { passive: true });
      return () => {
        container.removeEventListener('scroll', handleScrollEnd);
        clearTimeout(scrollTimeout);
      };
    }, [tabs, activeTabId, onTabFocus, viewMode]);

    // Ctrl/zoom wheel on the canvas (spatial mode). Attached natively so we can
    // preventDefault the page-zoom / scroll.
    useEffect(() => {
      const container = containerRef.current;
      if (!container || !spatial) return;

      const onWheel = (e: WheelEvent) => {
        e.preventDefault();
        const rect = container.getBoundingClientRect();
        const mx = e.clientX - rect.left;
        const my = e.clientY - rect.top;
        const factor = Math.exp(-e.deltaY * 0.0015);
        setZoom((z) => {
          const nz = clamp(z * factor, ZOOM_MIN, ZOOM_MAX);
          const ratio = nz / z;
          // Keep the world point under the cursor fixed on screen.
          setPan((p) => ({
            x: mx - (mx - p.x) * ratio,
            y: my - (my - p.y) * ratio,
          }));
          return nz;
        });
      };

      container.addEventListener('wheel', onWheel, { passive: false });
      return () => container.removeEventListener('wheel', onWheel);
    }, [spatial]);

    // Window-level drag handling for pan / card-move / card-resize.
    useEffect(() => {
      const onMove = (e: MouseEvent) => {
        const it = interactionRef.current;
        if (!it) return;
        const dx = e.clientX - it.startClientX;
        const dy = e.clientY - it.startClientY;

        if (it.kind === 'pan') {
          const o = it.origin as { x: number; y: number };
          setPan({ x: o.x + dx, y: o.y + dy });
          return;
        }

        const o = it.origin as CanvasRect;
        const wdx = dx / it.zoom;
        const wdy = dy / it.zoom;
        if (it.kind === 'move') {
          setLiveRect({ tabId: it.tabId!, rect: { ...o, x: o.x + wdx, y: o.y + wdy } });
        } else {
          setLiveRect({
            tabId: it.tabId!,
            rect: { ...o, w: Math.max(280, o.w + wdx), h: Math.max(180, o.h + wdy) },
          });
        }
      };

      const onUp = () => {
        const it = interactionRef.current;
        interactionRef.current = null;
        if (it && (it.kind === 'move' || it.kind === 'resize')) {
          setLiveRect((lr) => {
            if (lr && lr.tabId === it.tabId) onTabCanvasChange?.(it.tabId!, lr.rect);
            return null;
          });
        }
        document.body.style.cursor = '';
        document.body.style.userSelect = '';
      };

      window.addEventListener('mousemove', onMove);
      window.addEventListener('mouseup', onUp);
      return () => {
        window.removeEventListener('mousemove', onMove);
        window.removeEventListener('mouseup', onUp);
      };
    }, [onTabCanvasChange]);

    const beginPan = useCallback((e: React.MouseEvent) => {
      if (!spatial || e.button !== 0) return;
      interactionRef.current = { kind: 'pan', startClientX: e.clientX, startClientY: e.clientY, origin: { ...pan }, zoom };
      document.body.style.cursor = 'grabbing';
      document.body.style.userSelect = 'none';
    }, [spatial, pan, zoom]);

    const beginCardMove = useCallback((e: React.MouseEvent, tab: TabConfig, index: number) => {
      if (e.button !== 0) return;
      e.stopPropagation();
      onTabFocus(tab.id);
      interactionRef.current = { kind: 'move', tabId: tab.id, startClientX: e.clientX, startClientY: e.clientY, origin: rectFor(tab, index), zoom };
      document.body.style.userSelect = 'none';
    }, [rectFor, zoom, onTabFocus]);

    const beginCardResize = useCallback((e: React.MouseEvent, tab: TabConfig, index: number) => {
      if (e.button !== 0) return;
      e.stopPropagation();
      interactionRef.current = { kind: 'resize', tabId: tab.id, startClientX: e.clientX, startClientY: e.clientY, origin: rectFor(tab, index), zoom };
      document.body.style.cursor = 'nwse-resize';
      document.body.style.userSelect = 'none';
    }, [rectFor, zoom]);

    const handleTabMove = useCallback((tabId: string, delta: number) => {
      if (!onTabMove) return;
      const idx = tabs.findIndex((t) => t.id === tabId);
      if (idx < 0) return;
      onTabMove(tabId, idx + delta);
    }, [tabs, onTabMove]);

    // --- Timeline geometry (vertical activity feed) ---------------------------
    const tlIndex = new Map<string, number>();
    if (timeline) timelineSorted(tabs).forEach((t, i) => tlIndex.set(t.id, i));
    const tlCardW = Math.max(TL_MIN_W, containerWidth - TL_GUTTER - TL_RIGHT);
    const tlTotalH = TL_TOP + tabs.length * (TL_CARD_H + TL_GAP);

    return (
      <div
        ref={containerRef}
        className="scroll-container"
        onMouseDown={spatial ? beginPan : undefined}
        style={{
          position: 'relative',
          display: viewMode === 'tabs' ? 'flex' : 'block',
          flexDirection: 'row',
          overflowX: viewMode === 'tabs' ? 'auto' : 'hidden',
          overflowY: timeline ? 'auto' : 'hidden',
          height: '100%',
          scrollSnapType: viewMode === 'tabs' ? 'x mandatory' : undefined,
          scrollBehavior: 'auto',
          padding: '0',
          gap: '0px',
          alignItems: 'stretch',
          cursor: spatial ? 'grab' : undefined,
        }}
      >
        {/* Spatial-canvas background grid (also the pan target). Always rendered
            so the pane-host wrapper below keeps a stable position in the tree;
            inert in tabs mode. */}
        <div
          key="canvas-bg"
          aria-hidden
          style={{
            position: 'absolute',
            inset: 0,
            display: spatial ? 'block' : 'none',
            backgroundColor: 'var(--wks-bg-base)',
            backgroundImage:
              'radial-gradient(var(--wks-border) 1px, transparent 1px)',
            backgroundSize: `${24 * zoom}px ${24 * zoom}px`,
            backgroundPosition: `${pan.x}px ${pan.y}px`,
            zIndex: 0,
          }}
        />

        {/* Pane host. In tabs mode it's the flex strip; in spatial mode it's the
            pan/zoom-transformed world. Keyed + always present so toggling the
            mode only re-styles it — the pane subtree never re-parents (which
            would remount → kill terminals/webviews). */}
        <div
          key="pane-host"
          style={
            spatial
              ? {
                  position: 'absolute',
                  top: 0,
                  left: 0,
                  width: 0,
                  height: 0,
                  transform: `translate(${pan.x}px, ${pan.y}px) scale(${zoom})`,
                  transformOrigin: '0 0',
                  zIndex: 1,
                }
              : timeline
              ? // Vertical feed: a tall relative block the container scrolls
                // through; cards are absolutely positioned within it by recency.
                {
                  position: 'relative',
                  width: '100%',
                  height: `${tlTotalH}px`,
                  zIndex: 1,
                }
              : // In tabs mode the wrapper itself must not participate in layout
                // (a single flex child would shrink instead of letting the cards
                // overflow & scroll). `display: contents` makes the cards behave
                // as direct flex children of the scroll container, exactly as
                // before — while keeping this element (and the pane subtree under
                // it) at a stable tree position so toggling modes never remounts.
                { display: 'contents' }
          }
        >
          {tabs.map((tab, index) => {
            const isActiveTab = tab.id === activeTabId;
            const rect = spatial ? rectFor(tab, index) : null;
            const showHeader = spatial || timeline;
            const tlPos = timeline ? (tlIndex.get(tab.id) ?? index) : 0;
            const tlY = TL_TOP + tlPos * (TL_CARD_H + TL_GAP);

            const paneCallbacks: PaneCallbacks = {
              onPtyReady,
              onUrlChange: onUrlChange
                ? (paneId: string, url: string) => onUrlChange(tab.id, paneId, url)
                : undefined,
              tabs,
              onNavigateToTab: onNavigateToTab ?? onTabFocus,
              onAddTab,
              ptyMapping,
              workspaceAgents,
              appCwd,
            };

            // Per-card inner dimensions handed to the tiling layout.
            const innerW = spatial ? rect!.w : timeline ? tlCardW : tabWidth;
            const innerH = spatial
              ? rect!.h - CARD_HEADER_H
              : timeline
              ? TL_CARD_H - CARD_HEADER_H
              : containerHeight;

            const floatingCard: React.CSSProperties = {
              display: 'flex',
              flexDirection: 'column',
              borderRadius: '8px',
              overflow: 'hidden',
              backgroundColor: 'var(--wks-bg-surface)',
              border: isActiveTab ? '1px solid var(--wks-accent)' : '1px solid var(--wks-glass-border)',
              boxShadow: isActiveTab ? '0 8px 28px var(--wks-shadow)' : '0 4px 14px var(--wks-shadow)',
            };
            const cardStyle: React.CSSProperties = spatial
              ? {
                  ...floatingCard,
                  position: 'absolute',
                  left: `${rect!.x}px`,
                  top: `${rect!.y}px`,
                  width: `${rect!.w}px`,
                  height: `${rect!.h}px`,
                }
              : timeline
              ? {
                  ...floatingCard,
                  position: 'absolute',
                  left: `${TL_GUTTER}px`,
                  top: `${tlY}px`,
                  width: `${tlCardW}px`,
                  height: `${TL_CARD_H}px`,
                  contentVisibility: 'auto',
                  containIntrinsicSize: `${tlCardW}px ${TL_CARD_H}px`,
                }
              : {
                  scrollSnapAlign: 'center',
                  flexShrink: 0,
                  height: '100%',
                  display: 'flex',
                  alignItems: 'stretch',
                  width: `${tabWidth}px`,
                  minWidth: `${tabWidth}px`,
                  // Let the browser skip layout/paint for off-screen tabs.
                  contentVisibility: 'auto',
                  containIntrinsicSize: `${tabWidth}px ${containerHeight}px`,
                };

            return (
              <div key={tab.id} data-tab-id={tab.id} style={cardStyle}>
                {/* Drag-handle strip — only shown (and interactive) in spatial
                    mode. Always rendered so the pane-host subtree below keeps a
                    stable position across mode toggles. */}
                <div
                  key="card-header"
                  onMouseDown={spatial ? (e) => beginCardMove(e, tab, index) : undefined}
                  onDoubleClick={showHeader && onTabRename ? () => {
                    const name = window.prompt('Rename tab', tab.title);
                    if (name != null && name.trim()) onTabRename(tab.id, name.trim());
                  } : undefined}
                  style={{
                    display: showHeader ? 'flex' : 'none',
                    alignItems: 'center',
                    gap: 6,
                    height: `${CARD_HEADER_H}px`,
                    flexShrink: 0,
                    padding: '0 8px',
                    cursor: spatial ? 'move' : 'default',
                    fontSize: '0.72rem',
                    fontWeight: isActiveTab ? 600 : 400,
                    color: isActiveTab ? 'var(--wks-text-primary)' : 'var(--wks-text-muted)',
                    backgroundColor: 'var(--wks-glass-strong)',
                    borderBottom: '1px solid var(--wks-glass-border)',
                    userSelect: 'none',
                  }}
                >
                  <span style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                    {tab.title}
                  </span>
                  {tab.panes.length > 1 && (
                    <span style={{ fontSize: '0.55rem', opacity: 0.6 }}>{tab.panes.length}</span>
                  )}
                </div>

                {/* Invisible positioning box; the pane subtree lives here in
                    BOTH modes (only its size/position changes). */}
                <div
                  key="pane-box"
                  style={{
                    flex: 1,
                    minWidth: 0,
                    height: showHeader ? undefined : '100%',
                    position: 'relative',
                  }}
                  onClick={() => onTabFocus(tab.id)}
                >
                  <TilingLayout
                    panes={tab.panes}
                    activePaneId={tab.activePaneId}
                    agentActive={agentActive}
                    forceLive={spatial || timeline}
                    containerWidth={innerW}
                    containerHeight={innerH}
                    onPaneClose={(paneId) => onPaneClose(tab.id, paneId)}
                    onPaneFocus={(paneId) => onPaneFocus(tab.id, paneId)}
                    callbacks={paneCallbacks}
                    isActiveTab={isActiveTab}
                    tabTitle={tab.title}
                    onTabFocus={() => onTabFocus(tab.id)}
                    onTabMove={onTabMove ? (delta) => handleTabMove(tab.id, delta) : undefined}
                    onTabRename={onTabRename ? (title) => onTabRename(tab.id, title) : undefined}
                    renameSignal={isActiveTab ? renameSignal : undefined}
                  />
                </div>

                {/* Resize handle (spatial only). */}
                {spatial && (
                  <div
                    onMouseDown={(e) => beginCardResize(e, tab, index)}
                    style={{
                      position: 'absolute',
                      right: 0,
                      bottom: 0,
                      width: 16,
                      height: 16,
                      cursor: 'nwse-resize',
                      zIndex: 2,
                      background:
                        'linear-gradient(135deg, transparent 50%, var(--wks-text-faint) 50%, var(--wks-text-faint) 60%, transparent 60%)',
                      opacity: 0.6,
                    }}
                  />
                )}
              </div>
            );
          })}

          {/* Timeline rail + per-card time labels in the left gutter. Keyed
              siblings of the cards — they never disturb card (pane) identity. */}
          {timeline && (
            <div
              key="tl-rail"
              aria-hidden
              style={{
                position: 'absolute',
                left: `${TL_GUTTER - 18}px`,
                top: `${TL_TOP}px`,
                width: '2px',
                height: `${Math.max(0, tlTotalH - TL_TOP - TL_GAP)}px`,
                backgroundColor: 'var(--wks-glass-border)',
                zIndex: 0,
              }}
            />
          )}
          {timeline && tabs.map((tab) => {
            const pos = tlIndex.get(tab.id) ?? 0;
            const y = TL_TOP + pos * (TL_CARD_H + TL_GAP);
            const isActiveTab = tab.id === activeTabId;
            return (
              <div
                key={`tl-label-${tab.id}`}
                onClick={() => { onTabFocus(tab.id); scrollToTab(tab.id); }}
                title={tab.lastActiveAt ? new Date(tab.lastActiveAt).toLocaleString() : 'no recorded activity'}
                style={{
                  position: 'absolute',
                  left: 0,
                  top: `${y}px`,
                  width: `${TL_GUTTER - 26}px`,
                  height: `${CARD_HEADER_H}px`,
                  display: 'flex',
                  flexDirection: 'column',
                  alignItems: 'flex-end',
                  justifyContent: 'center',
                  textAlign: 'right',
                  cursor: 'pointer',
                  zIndex: 1,
                }}
              >
                <span style={{ fontSize: '0.62rem', fontWeight: 600, color: isActiveTab ? 'var(--wks-accent)' : 'var(--wks-text-muted)', lineHeight: 1.1 }}>
                  {formatRelative(tab.lastActiveAt)}
                </span>
              </div>
            );
          })}

          {/* Rail dots, drawn last so they sit atop the rail line. */}
          {timeline && tabs.map((tab) => {
            const pos = tlIndex.get(tab.id) ?? 0;
            const y = TL_TOP + pos * (TL_CARD_H + TL_GAP);
            const isActiveTab = tab.id === activeTabId;
            return (
              <div
                key={`tl-dot-${tab.id}`}
                aria-hidden
                style={{
                  position: 'absolute',
                  left: `${TL_GUTTER - 18 - 3}px`,
                  top: `${y + CARD_HEADER_H / 2 - 4}px`,
                  width: '8px',
                  height: '8px',
                  borderRadius: '50%',
                  backgroundColor: isActiveTab ? 'var(--wks-accent)' : 'var(--wks-text-faint)',
                  border: '2px solid var(--wks-bg-base)',
                  boxSizing: 'content-box',
                  zIndex: 1,
                }}
              />
            );
          })}
        </div>
      </div>
    );
  }
);

ScrollContainer.displayName = 'ScrollContainer';

export default ScrollContainer;
