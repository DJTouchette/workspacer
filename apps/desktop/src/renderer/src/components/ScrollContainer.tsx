import React, { useRef, useState, useCallback, useEffect, useImperativeHandle, forwardRef, Suspense } from 'react';
import Pane from './Pane';
import ErrorBoundary from './ErrorBoundary';
import { PaneConfig, PaneType, TabConfig, CanvasRect, ViewMode, AgentWorkspace } from '../types/pane';
import { useConfig } from '../hooks/useConfig';
import { tilingColumns } from '../lib/layoutUtils';

// Lazy-load pane types that aren't needed on initial render
const TerminalPane = React.lazy(() => import('../panes/TerminalPane'));
const ClaudePane = React.lazy(() => import('../panes/ClaudePane'));
const BrowserPane = React.lazy(() => import('../panes/BrowserPane'));
const PluginPane = React.lazy(() => import('../panes/PluginPane'));
const NotesPane = React.lazy(() => import('../panes/NotesPane'));
const EditorPane = React.lazy(() => import('../panes/EditorPane'));
const SettingsPane = React.lazy(() => import('../panes/SettingsPane'));
const ReviewPane = React.lazy(() => import('../panes/ReviewPane'));
const PluginsManagerPane = React.lazy(() => import('../panes/PluginsManagerPane'));
const OverviewPane = React.lazy(() => import('../panes/OverviewPane'));
const LibraryPane = React.lazy(() => import('../panes/LibraryPane'));
const AnalyticsPane = React.lazy(() => import('../panes/AnalyticsPane'));
const AskPane = React.lazy(() => import('../panes/AskPane'));

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

// Spatial-canvas grid: snap card position/size to this world-unit grid so cards
// line up instead of landing on fractional pixels after a drag.
const SNAP = 20;
const snap = (v: number) => Math.round(v / SNAP) * SNAP;

/** POSIX single-quote a path so it's safe as a terminal-editor argument. */
function shellQuote(p: string): string {
  return `'${p.replace(/'/g, `'\\''`)}'`;
}

// --- Stacked feed (vertical, Instagram-style) constants -----------------------
const STK_TOP = 16;     // top/bottom padding
const STK_GAP = 20;     // vertical gap between cards
const STK_SIDE = 16;    // min horizontal margin around the centered column
const STK_MAX_W = 1600; // max card width
const STK_MIN_W = 360;  // min card width before it just fills the space

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
  onNotesChange?: (tabId: string, paneId: string, notes: string) => void;
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
  /** Full agent list — passed down to the Ask pane so it can display all agents. */
  allAgents?: AgentWorkspace[];
  /** Spawn a supervisor agent from a question — forwarded to AskPane. */
  spawnSupervisor?: (opts: { question: string; parentId?: string }) => Promise<string>;
  /** Jump to a specific agent by id — forwarded to AskPane. */
  onJumpToAgent?: (agentId: string) => void;
}

export interface ScrollContainerRef {
  scrollToTab: (id: string) => void;
}

interface PaneCallbacks {
  onPtyReady?: (paneId: string, ptySessionId: string) => void;
  onUrlChange?: (paneId: string, url: string) => void;
  onNotesChange?: (paneId: string, notes: string) => void;
  /** Editor-pane engine + terminal command, from config.editor. */
  editorEngine?: 'codemirror' | 'terminal';
  editorTerminalCommand?: string;
  tabs?: TabConfig[];
  onNavigateToTab?: (tabId: string) => void;
  onAddTab?: (type: PaneType, shell?: string, label?: string, cwd?: string, profileId?: string, resumeSessionId?: string, attachSessionId?: string) => void;
  ptyMapping?: Record<string, string>;
  workspaceAgents?: { sessionId?: string }[];
  appCwd?: string;
  /** Full agent list for the Ask pane. */
  allAgents?: AgentWorkspace[];
  /** Spawn a supervisor — for the Ask pane. */
  spawnSupervisor?: (opts: { question: string; parentId?: string }) => Promise<string>;
  /** Jump to agent by id — for the Ask pane. */
  onJumpToAgent?: (agentId: string) => void;
}

function renderPaneContent(pane: PaneConfig, isActive: boolean, callbacks: PaneCallbacks) {
  switch (pane.type) {
    case 'terminal':
      return (
        <Suspense fallback={<PaneFallback />}>
          <TerminalPane paneId={pane.id} title={pane.title} isActive={isActive} shell={pane.shell} cwd={pane.cwd} initialCommand={pane.initialCommand} onPtyReady={callbacks.onPtyReady} />
        </Suspense>
      );
    case 'claude':
      return (
        <Suspense fallback={<PaneFallback />}>
          <ClaudePane paneId={pane.id} title={pane.title} isActive={isActive} cwd={pane.cwd} profileId={pane.profileId} resumeSessionId={pane.resumeSessionId} attachSessionId={pane.attachSessionId} initialPrompt={pane.initialPrompt} onPtyReady={callbacks.onPtyReady} />
        </Suspense>
      );
    case 'browser':
      return (
        <Suspense fallback={<PaneFallback />}>
          <BrowserPane paneId={pane.id} title={pane.title} isActive={isActive} initialUrl={pane.url} appMode={pane.appMode} hibernated={pane.hibernated} onUrlChange={(url) => callbacks.onUrlChange?.(pane.id, url)} />
        </Suspense>
      );
    case 'notes':
      return (
        <Suspense fallback={<PaneFallback />}>
          <NotesPane title={pane.title} notes={pane.notes} onNotesChange={(notes) => callbacks.onNotesChange?.(pane.id, notes)} />
        </Suspense>
      );
    case 'editor':
      // The 'terminal' engine just runs the user's editor in a PTY pane; the
      // 'codemirror' engine is the in-app editor. Chosen live from config.
      if (callbacks.editorEngine === 'terminal') {
        const editorCmd = callbacks.editorTerminalCommand || 'nvim';
        const cmd = pane.filePath
          ? `${editorCmd} ${shellQuote(pane.filePath)}`
          : pane.cwd ? `${editorCmd} .` : undefined;
        return (
          <Suspense fallback={<PaneFallback />}>
            <TerminalPane paneId={pane.id} title={pane.title} isActive={isActive} cwd={pane.cwd} initialCommand={cmd} onPtyReady={callbacks.onPtyReady} />
          </Suspense>
        );
      }
      return (
        <Suspense fallback={<PaneFallback />}>
          <EditorPane paneId={pane.id} title={pane.title} isActive={isActive} filePath={pane.filePath} cwd={pane.cwd} />
        </Suspense>
      );
    case 'settings':
      return <Suspense fallback={<PaneFallback />}><SettingsPane title={pane.title} /></Suspense>;
    case 'review':
      return (
        <Suspense fallback={<PaneFallback />}>
          <ReviewPane paneId={pane.id} title={pane.title} isActive={isActive} cwd={pane.cwd} />
        </Suspense>
      );
    case 'plugin':
      // A plugin-injected pane: a webview onto the plugin's own UI. PluginPane
      // mints/revokes an agent-cwd-scoped bus token around it (see PluginPane).
      return (
        <Suspense fallback={<PaneFallback />}>
          <PluginPane paneId={pane.id} title={pane.title} isActive={isActive} url={pane.url || 'about:blank'} hibernated={pane.hibernated} pluginId={pane.pluginId} cwd={pane.cwd} />
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
    case 'ask':
      return (
        <Suspense fallback={<PaneFallback />}>
          <AskPane
            agents={callbacks.allAgents ?? []}
            spawnSupervisor={callbacks.spawnSupervisor ?? (() => Promise.resolve(''))}
            onJumpToAgent={callbacks.onJumpToAgent ?? (() => {})}
            scopeAgentId={pane.scopeAgentId}
          />
        </Suspense>
      );
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
  const cols = tilingColumns(count);
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
              flush={single}
            >
              <ErrorBoundary label={pane.title || pane.type} resetKeys={[pane.id]}>
                {renderPaneContent(pane, liveActive, callbacks)}
              </ErrorBoundary>
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
  ({ tabs, activeTabId, onTabFocus, onPaneClose, onPaneFocus, onTabRename, onTabMove, onPtyReady, onUrlChange, onNotesChange, onNavigateToTab, onAddTab, ptyMapping, renameSignal, viewMode = 'tabs', onTabCanvasChange, agentActive = true, workspaceAgents, appCwd, allAgents, spawnSupervisor, onJumpToAgent }, ref) => {
    const containerRef = useRef<HTMLDivElement>(null);
    const { config } = useConfig();
    const peek = config.panes?.peek ?? 80;
    const gap = config.panes?.gap ?? 16;
    const editorEngine = config.editor?.engine ?? 'codemirror';
    const editorTerminalCommand = config.editor?.terminalCommand ?? 'nvim';
    const spatial = viewMode === 'spatial';
    const stacked = viewMode === 'stacked';

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

      if (stacked) {
        const el = container.querySelector(`[data-tab-id="${id}"]`) as HTMLElement | null;
        if (el) container.scrollTo({ top: Math.max(0, el.offsetTop - STK_TOP), behavior: 'instant' });
        return;
      }

      const tabEl = container.querySelector(`[data-tab-id="${id}"]`) as HTMLElement | null;
      if (!tabEl) return;
      const containerRect = container.getBoundingClientRect();
      const tabRect = tabEl.getBoundingClientRect();
      const scrollLeft = tabEl.offsetLeft - containerRect.width / 2 + tabRect.width / 2;
      container.scrollTo({ left: scrollLeft, behavior: 'instant' });
    }, [spatial, stacked, tabs, zoom]);

    useImperativeHandle(ref, () => ({ scrollToTab }), [scrollToTab]);

    // Stacked feed: wrap the scroll so it loops (Instagram-style). At the bottom
    // a downward scroll jumps to the top; at the top an upward scroll jumps to
    // the bottom. Live panes can't be duplicated, so the seam is a jump, not a
    // seamless wrap. Wheel/trackpad only — scrollbar drag stays linear.
    useEffect(() => {
      const container = containerRef.current;
      if (!container || !stacked) return;
      const onWheel = (e: WheelEvent) => {
        const max = container.scrollHeight - container.clientHeight;
        if (max <= 0) return;
        const atTop = container.scrollTop <= 0;
        const atBottom = container.scrollTop >= max - 1;
        if (e.deltaY > 0 && atBottom) {
          e.preventDefault();
          container.scrollTo({ top: 0, behavior: 'auto' });
        } else if (e.deltaY < 0 && atTop) {
          e.preventDefault();
          container.scrollTo({ top: max, behavior: 'auto' });
        }
      };
      container.addEventListener('wheel', onWheel, { passive: false });
      return () => container.removeEventListener('wheel', onWheel);
    }, [stacked, tabs.length]);

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
          setLiveRect({ tabId: it.tabId!, rect: { ...o, x: snap(o.x + wdx), y: snap(o.y + wdy) } });
        } else {
          setLiveRect({
            tabId: it.tabId!,
            rect: { ...o, w: Math.max(280, snap(o.w + wdx)), h: Math.max(180, snap(o.h + wdy)) },
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

    // --- Stacked feed geometry (vertical, natural order) ----------------------
    // Cards are near-viewport tall and snap, so the feed pages one at a time.
    const stkCardW = Math.min(STK_MAX_W, Math.max(STK_MIN_W, containerWidth - 2 * STK_SIDE));
    const stkLeft = Math.max(STK_SIDE, Math.round((containerWidth - stkCardW) / 2));
    const stkCardH = Math.max(360, containerHeight - 2 * STK_TOP);
    const stkTotalH = STK_TOP + tabs.length * (stkCardH + STK_GAP);

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
          overflowY: stacked ? 'auto' : 'hidden',
          height: '100%',
          scrollSnapType: viewMode === 'tabs' ? 'x mandatory' : stacked ? 'y mandatory' : undefined,
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
              : stacked
              ? // Vertical feed: a tall relative block the container scrolls
                // through; cards are absolutely positioned within it in order.
                {
                  position: 'relative',
                  width: '100%',
                  height: `${stkTotalH}px`,
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
            // Only spatial keeps the card header — it's the drag handle there.
            // Stacked (and tabs) show the pane content with no title bar.
            const showHeader = spatial;
            const stkY = STK_TOP + index * (stkCardH + STK_GAP);

            const paneCallbacks: PaneCallbacks = {
              onPtyReady,
              onUrlChange: onUrlChange
                ? (paneId: string, url: string) => onUrlChange(tab.id, paneId, url)
                : undefined,
              onNotesChange: onNotesChange
                ? (paneId: string, notes: string) => onNotesChange(tab.id, paneId, notes)
                : undefined,
              editorEngine,
              editorTerminalCommand,
              tabs,
              onNavigateToTab: onNavigateToTab ?? onTabFocus,
              onAddTab,
              ptyMapping,
              workspaceAgents,
              appCwd,
              allAgents,
              spawnSupervisor,
              onJumpToAgent,
            };

            // Per-card inner dimensions handed to the tiling layout.
            const innerW = spatial ? rect!.w : stacked ? stkCardW : tabWidth;
            const innerH = spatial
              ? rect!.h - CARD_HEADER_H
              : stacked
              ? stkCardH // no header in stacked → content fills the whole card
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
              : stacked
              ? {
                  // No card chrome in stacked — just the pane content (theme-aware
                  // rounded corners, no background/border/shadow frame).
                  position: 'absolute',
                  left: `${stkLeft}px`,
                  top: `${stkY}px`,
                  width: `${stkCardW}px`,
                  height: `${stkCardH}px`,
                  display: 'flex',
                  flexDirection: 'column',
                  overflow: 'hidden',
                  borderRadius: 'var(--wks-radius-lg)',
                  scrollSnapAlign: 'center',
                  contentVisibility: 'auto',
                  containIntrinsicSize: `${stkCardW}px ${stkCardH}px`,
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
                    justifyContent: 'center',
                    height: `${CARD_HEADER_H}px`,
                    flexShrink: 0,
                    cursor: spatial ? 'move' : 'default',
                    backgroundColor: 'var(--wks-glass-strong)',
                    borderBottom: '1px solid var(--wks-glass-border)',
                    userSelect: 'none',
                  }}
                  title={spatial ? 'Drag to move · double-click to rename' : undefined}
                >
                  {/* Title-less drag handle (spatial only) — a subtle grip, no
                      "Claude/Terminal" label. */}
                  <div style={{ width: 26, height: 3, borderRadius: 2, backgroundColor: 'var(--wks-text-faint)', opacity: 0.45 }} />
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
                    forceLive={spatial || stacked}
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

        </div>
      </div>
    );
  }
);

ScrollContainer.displayName = 'ScrollContainer';

export default ScrollContainer;
