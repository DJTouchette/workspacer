import React, { useRef, useState, useCallback, useEffect, useImperativeHandle, forwardRef, Suspense } from 'react';
import Pane from './Pane';
import { PaneConfig, PaneType, TabConfig } from '../types/pane';
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
        // screen; the single-pane visual highlight tracks the active tab.
        const liveActive = agentActive && (single ? isActiveTab : pane.id === activePaneId);
        const isActive = single ? isActiveTab : liveActive;
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

const ScrollContainer = forwardRef<ScrollContainerRef, ScrollContainerProps>(
  ({ tabs, activeTabId, onTabFocus, onPaneClose, onPaneFocus, onTabRename, onTabMove, onPtyReady, onUrlChange, onNavigateToTab, onAddTab, ptyMapping, renameSignal, agentActive = true, workspaceAgents, appCwd }, ref) => {
    const containerRef = useRef<HTMLDivElement>(null);
    const { config } = useConfig();
    const peek = config.panes?.peek ?? 80;
    const gap = config.panes?.gap ?? 16;

    const [tabWidth, setTabWidth] = useState(800);
    const [containerHeight, setContainerHeight] = useState(600);

    useEffect(() => {
      const container = containerRef.current;
      if (!container) return;

      const updateSize = () => {
        const w = container.clientWidth - 2 * peek - gap;
        setTabWidth(Math.max(400, w));
        setContainerHeight(container.clientHeight - 16);
      };

      updateSize();
      const observer = new ResizeObserver(updateSize);
      observer.observe(container);
      return () => observer.disconnect();
    }, [peek, gap]);

    const scrollToTab = useCallback((id: string) => {
      const container = containerRef.current;
      if (!container) return;
      const tabEl = container.querySelector(`[data-tab-id="${id}"]`) as HTMLElement | null;
      if (!tabEl) return;

      const containerRect = container.getBoundingClientRect();
      const tabRect = tabEl.getBoundingClientRect();
      const scrollLeft = tabEl.offsetLeft - containerRect.width / 2 + tabRect.width / 2;
      container.scrollTo({ left: scrollLeft, behavior: 'instant' });
    }, []);

    useImperativeHandle(ref, () => ({ scrollToTab }), [scrollToTab]);

    // Detect which tab is most visible after scroll ends
    useEffect(() => {
      const container = containerRef.current;
      if (!container) return;

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
    }, [tabs, activeTabId, onTabFocus]);

    const handleTabMove = useCallback((tabId: string, delta: number) => {
      if (!onTabMove) return;
      const idx = tabs.findIndex((t) => t.id === tabId);
      if (idx < 0) return;
      onTabMove(tabId, idx + delta);
    }, [tabs, onTabMove]);

    return (
      <div
        ref={containerRef}
        className="scroll-container"
        style={{
          display: 'flex',
          flexDirection: 'row',
          overflowX: 'auto',
          overflowY: 'hidden',
          height: '100%',
          scrollSnapType: 'x mandatory',
          scrollBehavior: 'auto',
          padding: '0',
          gap: '0px',
          alignItems: 'stretch',
        }}
      >
        {tabs.map((tab) => {
          const isActiveTab = tab.id === activeTabId;
          const singlePane = tab.panes.length === 1;

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

          return (
            <div
              key={tab.id}
              data-tab-id={tab.id}
              style={{
                scrollSnapAlign: 'center',
                flexShrink: 0,
                height: '100%',
                display: 'flex',
                alignItems: 'stretch',
                width: `${tabWidth}px`,
                minWidth: `${tabWidth}px`,
                // Let the browser skip layout/paint/animation for tabs scrolled
                // off-screen. The intrinsic size keeps the scrollbar stable so
                // skipped tabs still occupy their slot.
                contentVisibility: 'auto',
                containIntrinsicSize: `${tabWidth}px ${containerHeight}px`,
              }}
            >
              {/* Always render through TilingLayout — single- and multi-pane
                 tabs share one structure so splitting/closing a pane never
                 re-parents (and thus never remounts → kills the PTY of) an
                 existing pane. This wrapper is just an invisible positioning
                 box; each pane carries its own border (focused one accented). */}
              <div
                style={{
                  flex: 1,
                  minWidth: 0,
                  height: '100%',
                  position: 'relative',
                }}
                onClick={() => onTabFocus(tab.id)}
              >
                <TilingLayout
                  panes={tab.panes}
                  activePaneId={tab.activePaneId}
                  agentActive={agentActive}
                  containerWidth={tabWidth}
                  containerHeight={containerHeight}
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
            </div>
          );
        })}
      </div>
    );
  }
);

ScrollContainer.displayName = 'ScrollContainer';

export default ScrollContainer;
