import React, { useRef, useState, useCallback, useEffect, useImperativeHandle, forwardRef, Suspense } from 'react';
import Pane from './Pane';
import { PaneConfig, TabConfig } from '../types/pane';
import TerminalPane from '../panes/TerminalPane';
import ClaudePane from '../panes/ClaudePane';
import { useConfig } from '../hooks/useConfig';

// Lazy-load pane types that aren't needed on initial render
const BrowserPane = React.lazy(() => import('../panes/BrowserPane'));
const NotesPane = React.lazy(() => import('../panes/NotesPane'));
const AgentPane = React.lazy(() => import('../panes/AgentPane'));
const SettingsPane = React.lazy(() => import('../panes/SettingsPane'));
const DashboardPane = React.lazy(() => import('../panes/DashboardPane'));
const TrackerPane = React.lazy(() => import('../panes/TrackerPane'));
const DevOpsPane = React.lazy(() => import('../panes/DevOpsPane'));

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
  renameSignal?: number;
}

export interface ScrollContainerRef {
  scrollToTab: (id: string) => void;
}

interface PaneCallbacks {
  onPtyReady?: (paneId: string, ptySessionId: string) => void;
  onUrlChange?: (paneId: string, url: string) => void;
  tabs?: TabConfig[];
  onNavigateToTab?: (tabId: string) => void;
}

function renderPaneContent(pane: PaneConfig, isActive: boolean, callbacks: PaneCallbacks) {
  switch (pane.type) {
    case 'terminal':
      return <TerminalPane paneId={pane.id} title={pane.title} isActive={isActive} shell={pane.shell} cwd={pane.cwd} onPtyReady={callbacks.onPtyReady} />;
    case 'claude':
      return <ClaudePane paneId={pane.id} title={pane.title} isActive={isActive} cwd={pane.cwd} onPtyReady={callbacks.onPtyReady} />;
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
    case 'dashboard':
      return (
        <Suspense fallback={<PaneFallback />}>
          <DashboardPane title={pane.title} tabs={callbacks.tabs ?? []} onNavigateToTab={callbacks.onNavigateToTab ?? (() => {})} />
        </Suspense>
      );
    case 'tracker':
      return (
        <Suspense fallback={<PaneFallback />}>
          <TrackerPane paneId={pane.id} title={pane.title} isActive={isActive} />
        </Suspense>
      );
    case 'devops':
      return (
        <Suspense fallback={<PaneFallback />}>
          <DevOpsPane paneId={pane.id} title={pane.title} isActive={isActive} />
        </Suspense>
      );
    default:
      return <div>Unknown pane type</div>;
  }
}

// Auto-tiling layout for multiple panes within a tab
function TilingLayout({
  panes,
  activePaneId,
  containerWidth,
  containerHeight,
  onPaneClose,
  onPaneFocus,
  callbacks,
}: {
  panes: PaneConfig[];
  activePaneId: string;
  containerWidth: number;
  containerHeight: number;
  onPaneClose: (paneId: string) => void;
  onPaneFocus: (paneId: string) => void;
  callbacks: PaneCallbacks;
}) {
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
        const isActive = pane.id === activePaneId;
        const left = layout.col * cellWidth + gap;
        const top = layout.row * cellHeight + gap;
        const width = layout.colSpan * cellWidth - gap * 2;
        const height = layout.rowSpan * cellHeight - gap * 2;

        return (
          <div
            key={pane.id}
            style={{
              position: 'absolute',
              left: `${left}px`,
              top: `${top}px`,
              width: `${width}px`,
              height: `${height}px`,
            }}
          >
            <Pane
              id={pane.id}
              type={pane.type}
              title={pane.title}
              isActive={isActive}
              onClose={() => onPaneClose(pane.id)}
              onFocus={() => onPaneFocus(pane.id)}
            >
              {renderPaneContent(pane, isActive, callbacks)}
            </Pane>
          </div>
        );
      })}
    </>
  );
}

const ScrollContainer = forwardRef<ScrollContainerRef, ScrollContainerProps>(
  ({ tabs, activeTabId, onTabFocus, onPaneClose, onPaneFocus, onTabRename, onTabMove, onPtyReady, onUrlChange, onNavigateToTab, renameSignal }, ref) => {
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
              }}
            >
              {singlePane ? (
                // Single pane tab — render like before
                <Pane
                  id={tab.panes[0].id}
                  type={tab.panes[0].type}
                  title={tab.title}
                  isActive={isActiveTab}
                  onClose={() => onPaneClose(tab.id, tab.panes[0].id)}
                  onFocus={() => onTabFocus(tab.id)}
                  onMove={onTabMove ? (_, delta) => handleTabMove(tab.id, delta) : undefined}
                  onRename={onTabRename ? (_, title) => onTabRename(tab.id, title) : undefined}
                  renameSignal={isActiveTab ? renameSignal : undefined}
                >
                  {renderPaneContent(tab.panes[0], isActiveTab, paneCallbacks)}
                </Pane>
              ) : (
                // Multi-pane tab — tiling layout inside a container
                <div
                  style={{
                    flex: 1,
                    minWidth: 0,
                    height: 'calc(100% - 8px)',
                    margin: '4px 8px',
                    borderRadius: '8px',
                    overflow: 'hidden',
                    border: isActiveTab
                      ? '1px solid var(--wks-accent)'
                      : '1px solid var(--wks-border)',
                    boxShadow: isActiveTab ? '0 0 12px var(--wks-accent-glow)' : 'none',
                    position: 'relative',
                    backgroundColor: 'var(--wks-bg-base)',
                  }}
                  onClick={() => onTabFocus(tab.id)}
                >
                  <TilingLayout
                    panes={tab.panes}
                    activePaneId={tab.activePaneId}
                    containerWidth={tabWidth - 18}
                    containerHeight={containerHeight - 8}
                    onPaneClose={(paneId) => onPaneClose(tab.id, paneId)}
                    onPaneFocus={(paneId) => onPaneFocus(tab.id, paneId)}
                    callbacks={paneCallbacks}
                  />
                </div>
              )}
            </div>
          );
        })}
      </div>
    );
  }
);

ScrollContainer.displayName = 'ScrollContainer';

export default ScrollContainer;
