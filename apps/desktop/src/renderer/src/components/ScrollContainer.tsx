import React, {
  useRef,
  useState,
  useCallback,
  useEffect,
  useImperativeHandle,
  forwardRef,
  Suspense,
} from 'react';
import Pane from './Pane';
import { EmptyState } from './PaneMessage';
import ErrorBoundary from './ErrorBoundary';
import { PaneConfig, PaneType, TabConfig, AgentWorkspace, AgentProvider } from '../types/pane';
import { useConfig } from '../hooks/useConfig';
import { tilingColumns } from '../lib/layoutUtils';
import { useIsSmallScreen } from '../hooks/useMediaQuery';

// Lazy-load pane types that aren't needed on initial render
const TerminalPane = React.lazy(() => import('../panes/TerminalPane'));
const ClaudePane = React.lazy(() => import('../panes/ClaudePane'));
const BrowserPane = React.lazy(() => import('../panes/BrowserPane'));
const PluginPane = React.lazy(() => import('../panes/PluginPane'));
const NotesPane = React.lazy(() => import('../panes/NotesPane'));
const SettingsPane = React.lazy(() => import('../panes/SettingsPane'));
const ReviewPane = React.lazy(() => import('../panes/ReviewPane'));
const PluginsManagerPane = React.lazy(() => import('../panes/PluginsManagerPane'));
const OverviewPane = React.lazy(() => import('../panes/OverviewPane'));
const LibraryPane = React.lazy(() => import('../panes/LibraryPane'));
const AskPane = React.lazy(() => import('../panes/AskPane'));
const AgentWatchPane = React.lazy(() => import('../panes/AgentWatchPane'));
const AgentsPane = React.lazy(() => import('../panes/AgentsPane'));
const InspectorPane = React.lazy(() => import('../panes/InspectorPane'));
const MarkdownPreviewPane = React.lazy(() => import('../panes/MarkdownPreviewPane'));
const ContextPane = React.lazy(() => import('../panes/ContextPane'));

/** POSIX single-quote a path so it's safe as a terminal-editor argument. */
function shellQuote(p: string): string {
  return `'${p.replace(/'/g, `'\\''`)}'`;
}

const PaneFallback = () => (
  <div
    style={{
      width: '100%',
      height: '100%',
      display: 'flex',
      alignItems: 'center',
      justifyContent: 'center',
      backgroundColor: 'var(--wks-bg-base)',
      color: 'var(--wks-text-muted)',
      fontSize: '0.8rem',
    }}
  >
    Loading…
  </div>
);

interface ScrollContainerProps {
  /** Agent workspace that owns this container. */
  ownerAgentId?: string;
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
  onAddTab?: (
    type: PaneType,
    shell?: string,
    label?: string,
    cwd?: string,
    profileId?: string,
    resumeSessionId?: string,
    attachSessionId?: string,
  ) => void;
  /** Split the given tab by appending a new pane of `type` (in-pane split button). */
  onSplit?: (tabId: string, type: PaneType) => void;
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
  /** Owning agent's live working tree (worktree entered mid-session), if any. */
  agentLiveCwd?: string;
  /** Fallback project root for the Library pane (the app's cwd). */
  appCwd?: string;
  /** Full agent list — passed down to the Ask pane so it can display all agents. */
  allAgents?: AgentWorkspace[];
  /** Spawn a supervisor agent from a question — forwarded to AskPane. */
  spawnSupervisor?: (opts: {
    question?: string;
    parentId?: string;
    provider?: AgentProvider;
  }) => Promise<string>;
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
  onAddTab?: (
    type: PaneType,
    shell?: string,
    label?: string,
    cwd?: string,
    profileId?: string,
    resumeSessionId?: string,
    attachSessionId?: string,
  ) => void;
  ptyMapping?: Record<string, string>;
  workspaceAgents?: { sessionId?: string }[];
  appCwd?: string;
  /** The owning agent's CURRENT working tree when it differs from its home cwd
   *  (e.g. a git worktree entered mid-session). Agent-scoped plugin panes
   *  re-scope to it live. */
  agentLiveCwd?: string;
  /** Full agent list for the Ask pane. */
  allAgents?: AgentWorkspace[];
  /** Spawn a supervisor — for the Ask pane. */
  spawnSupervisor?: (opts: {
    question?: string;
    parentId?: string;
    provider?: AgentProvider;
  }) => Promise<string>;
  /** Jump to agent by id — for the Ask pane. */
  onJumpToAgent?: (agentId: string) => void;
  /** Agent workspace that owns this pane tree. */
  ownerAgentId?: string;
}

function renderPaneContent(pane: PaneConfig, isActive: boolean, callbacks: PaneCallbacks) {
  switch (pane.type) {
    case 'terminal':
      return (
        <Suspense fallback={<PaneFallback />}>
          <TerminalPane
            paneId={pane.id}
            title={pane.title}
            isActive={isActive}
            shell={pane.shell}
            cwd={pane.cwd}
            initialCommand={pane.initialCommand}
            onPtyReady={callbacks.onPtyReady}
          />
        </Suspense>
      );
    case 'claude':
      return (
        <Suspense fallback={<PaneFallback />}>
          <ClaudePane
            paneId={pane.id}
            title={pane.title}
            isActive={isActive}
            cwd={pane.cwd}
            profileId={pane.profileId}
            provider={pane.provider}
            transport={pane.transport}
            resumeSessionId={pane.resumeSessionId}
            attachSessionId={pane.attachSessionId}
            expectHistory={pane.expectHistory}
            initialPrompt={pane.initialPrompt}
            onPtyReady={callbacks.onPtyReady}
          />
        </Suspense>
      );
    case 'browser':
      return (
        <Suspense fallback={<PaneFallback />}>
          <BrowserPane
            paneId={pane.id}
            title={pane.title}
            isActive={isActive}
            initialUrl={pane.url}
            appMode={pane.appMode}
            hibernated={pane.hibernated}
            onUrlChange={(url) => callbacks.onUrlChange?.(pane.id, url)}
          />
        </Suspense>
      );
    case 'notes':
      return (
        <Suspense fallback={<PaneFallback />}>
          <NotesPane
            title={pane.title}
            notes={pane.notes}
            onNotesChange={(notes) => callbacks.onNotesChange?.(pane.id, notes)}
          />
        </Suspense>
      );
    case 'editor':
      // The in-app editor is now the sandboxed editor *plugin* (opened as a
      // 'plugin' pane via openFileInEditor). This 'editor' pane type only renders
      // the 'terminal' engine — the user's $EDITOR in a PTY. A leftover
      // codemirror 'editor' pane (e.g. from an old saved session) points the user
      // at the plugin instead of rendering the removed in-app editor.
      if (callbacks.editorEngine === 'terminal') {
        const editorCmd = callbacks.editorTerminalCommand || 'nvim';
        const cmd = pane.filePath
          ? `${editorCmd} ${shellQuote(pane.filePath)}`
          : pane.cwd
            ? `${editorCmd} .`
            : undefined;
        return (
          <Suspense fallback={<PaneFallback />}>
            <TerminalPane
              paneId={pane.id}
              title={pane.title}
              isActive={isActive}
              cwd={pane.cwd}
              initialCommand={cmd}
              onPtyReady={callbacks.onPtyReady}
            />
          </Suspense>
        );
      }
      return (
        <div
          style={{
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            height: '100%',
            color: 'var(--wks-text-faint)',
            fontSize: '0.8rem',
            textAlign: 'center',
            padding: 24,
          }}
        >
          The editor is now a plugin. Reopen it from the command palette (Open Editor).
        </div>
      );
    case 'settings':
      return (
        <Suspense fallback={<PaneFallback />}>
          <SettingsPane title={pane.title} />
        </Suspense>
      );
    case 'review':
      return (
        <Suspense fallback={<PaneFallback />}>
          <ReviewPane
            paneId={pane.id}
            title={pane.title}
            isActive={isActive}
            cwd={pane.cwd}
            onReturnToAgent={
              callbacks.ownerAgentId && callbacks.onJumpToAgent
                ? () => {
                    if (callbacks.ownerAgentId) callbacks.onJumpToAgent?.(callbacks.ownerAgentId);
                  }
                : undefined
            }
          />
        </Suspense>
      );
    case 'plugin':
      // A plugin-injected pane: a webview onto the plugin's own UI. PluginPane
      // mints/revokes an agent-cwd-scoped bus token around it (see PluginPane).
      return (
        <Suspense fallback={<PaneFallback />}>
          <PluginPane
            paneId={pane.id}
            title={pane.title}
            isActive={isActive}
            url={pane.url || 'about:blank'}
            hibernated={pane.hibernated}
            pluginId={pane.pluginId}
            // Agent-scoped panes follow the agent's LIVE tree (a worktree
            // entered mid-session) so the pane token re-mints for it; global
            // panes (no cwd) stay unscoped.
            cwd={pane.cwd ? callbacks.agentLiveCwd || pane.cwd : undefined}
          />
        </Suspense>
      );
    case 'plugins':
      return (
        <Suspense fallback={<PaneFallback />}>
          <PluginsManagerPane title={pane.title} />
        </Suspense>
      );
    case 'overview':
      return (
        <Suspense fallback={<PaneFallback />}>
          <OverviewPane title={pane.title} agents={callbacks.workspaceAgents} />
        </Suspense>
      );
    case 'library':
      return (
        <Suspense fallback={<PaneFallback />}>
          <LibraryPane title={pane.title} cwd={pane.cwd || callbacks.appCwd} />
        </Suspense>
      );
    case 'analytics':
      // The built-in Analytics pane moved to the catalog's Analytics plugin
      // (djtouchette.analytics). Legacy panes from saved sessions land here.
      return (
        <EmptyState
          title="Analytics moved to a plugin"
          hint={
            <>
              Install it from the command palette: <b>Install Plugin…</b> →{' '}
              <code>DJTouchette/workspacer-plugin-analytics</code>. Once installed, “Analytics”
              opens the plugin pane automatically.
            </>
          }
        />
      );
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
    case 'agentwatch':
      return (
        <Suspense fallback={<PaneFallback />}>
          <AgentWatchPane
            title={pane.title}
            isActive={isActive}
            watchSessionId={pane.watchSessionId}
            watchKind={pane.watchKind}
            watchId={pane.watchId}
          />
        </Suspense>
      );
    case 'agents':
      return (
        <Suspense fallback={<PaneFallback />}>
          <AgentsPane isActive={isActive} />
        </Suspense>
      );
    case 'inspector':
      return (
        <Suspense fallback={<PaneFallback />}>
          <InspectorPane
            title={pane.title}
            isActive={isActive}
            inspectorSessionId={pane.inspectorSessionId}
            inspectorAgentName={pane.inspectorAgentName}
          />
        </Suspense>
      );
    case 'mdpreview':
      return (
        <Suspense fallback={<PaneFallback />}>
          <MarkdownPreviewPane
            title={pane.title}
            previewPath={pane.previewPath}
            previewCwd={pane.previewCwd}
          />
        </Suspense>
      );
    case 'context':
      return (
        <Suspense fallback={<PaneFallback />}>
          <ContextPane
            title={pane.title}
            isActive={isActive}
            contextSessionId={pane.contextSessionId}
            contextAgentName={pane.contextAgentName}
            contextFocus={pane.contextFocus}
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
  onSplit,
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
  /** Split this tab by appending a pane of the chosen type (pane split button). */
  onSplit?: (type: PaneType) => void;
}) {
  const single = panes.length === 1;
  const count = panes.length;
  // Phones: side-by-side columns would give each pane a sliver of a ~375px
  // viewport, so split tabs stack vertically instead — one full-width pane
  // per row.
  const isSmallScreen = useIsSmallScreen();
  const cols = isSmallScreen ? 1 : tilingColumns(count);
  const rows = Math.ceil(count / cols);

  const layouts: Array<{ col: number; row: number; colSpan: number; rowSpan: number }> = [];

  if (count === 1) {
    layouts.push({ col: 0, row: 0, colSpan: 1, rowSpan: 1 });
  } else if (isSmallScreen) {
    for (let i = 0; i < count; i++) {
      layouts.push({ col: 0, row: i, colSpan: 1, rowSpan: 1 });
    }
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
  // Near-flush split gutter: the Pane card carries a 1px margin of its own, so
  // the grid cells sit edge-to-edge and adjacent panes are separated only by
  // that hairline + their borders.
  const gap = 0;

  return (
    <>
      {panes.map((pane, idx) => {
        const layout = layouts[idx];
        if (!layout) return null;
        // Liveness (drives throttling/focus) requires the agent to be on
        // screen; only the focused tab (single) or focused pane (multi) is live.
        const liveActive = agentActive && (single ? isActiveTab : pane.id === activePaneId);
        // Visual focus highlight: single-pane tracks the active tab; multi-pane
        // highlights the active pane of the active tab.
        const isActive = single ? isActiveTab : pane.id === activePaneId && isActiveTab;
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
              onSplit={onSplit}
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

const ScrollContainer = forwardRef<ScrollContainerRef, ScrollContainerProps>(
  (
    {
      tabs,
      activeTabId,
      onTabFocus,
      ownerAgentId,
      onPaneClose,
      onPaneFocus,
      onTabRename,
      onTabMove,
      onPtyReady,
      onUrlChange,
      onNotesChange,
      onNavigateToTab,
      onAddTab,
      onSplit,
      ptyMapping,
      renameSignal,
      agentActive = true,
      workspaceAgents,
      appCwd,
      agentLiveCwd,
      allAgents,
      spawnSupervisor,
      onJumpToAgent,
    },
    ref,
  ) => {
    const containerRef = useRef<HTMLDivElement>(null);
    const { config } = useConfig();
    const isSmallScreen = useIsSmallScreen();
    const peek = config.panes?.peek ?? 0;
    const gap = config.panes?.gap ?? 0;
    const editorEngine = config.editor?.engine ?? 'codemirror';
    const editorTerminalCommand = config.editor?.terminalCommand ?? 'nvim';

    const [tabWidth, setTabWidth] = useState(800);
    const [containerHeight, setContainerHeight] = useState(600);

    useEffect(() => {
      const container = containerRef.current;
      if (!container) return;

      const updateSize = () => {
        // Phones (the web share UI on a phone especially): one full-bleed tab
        // per screen. The desktop peek margins + 400px floor would force every
        // tab wider than the viewport and turn paging into sideways crawling.
        if (isSmallScreen) {
          setTabWidth(Math.max(1, container.clientWidth));
        } else {
          const w = container.clientWidth - 2 * peek - gap;
          setTabWidth(Math.max(400, w));
        }
        setContainerHeight(container.clientHeight - 16);
      };

      updateSize();
      const observer = new ResizeObserver(updateSize);
      observer.observe(container);
      return () => observer.disconnect();
    }, [peek, gap, isSmallScreen]);

    const scrollToTab = useCallback((id: string) => {
      // A just-opened tab (e.g. from the command palette) may not be committed to
      // the DOM on the first frame, so its element/index isn't found yet — which
      // left the strip highlighting the new tab while the view stayed on the old
      // pane. Retry across a few frames until the target exists.
      let attempts = 0;
      const attempt = () => {
        const container = containerRef.current;
        if (!container) return;

        const el = container.querySelector(`[data-tab-id="${id}"]`) as HTMLElement | null;
        if (!el) {
          if (attempts++ < 12) requestAnimationFrame(attempt);
          return;
        }

        const containerRect = container.getBoundingClientRect();
        const tabRect = el.getBoundingClientRect();
        const scrollLeft = el.offsetLeft - containerRect.width / 2 + tabRect.width / 2;
        container.scrollTo({ left: scrollLeft, behavior: 'instant' });
      };
      attempt();
    }, []);

    useImperativeHandle(ref, () => ({ scrollToTab }), [scrollToTab]);

    // Detect which tab is most visible after scroll ends.
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

    const handleTabMove = useCallback(
      (tabId: string, delta: number) => {
        if (!onTabMove) return;
        const idx = tabs.findIndex((t) => t.id === tabId);
        if (idx < 0) return;
        onTabMove(tabId, idx + delta);
      },
      [tabs, onTabMove],
    );

    return (
      <div
        ref={containerRef}
        className="scroll-container"
        style={{
          position: 'relative',
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
            agentLiveCwd,
            allAgents,
            spawnSupervisor,
            onJumpToAgent,
            ownerAgentId,
          };

          const cardStyle: React.CSSProperties = {
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
              {/* Invisible positioning box hosting the pane subtree. */}
              <div
                key="pane-box"
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
                  onSplit={onSplit ? (type) => onSplit(tab.id, type) : undefined}
                />
              </div>
            </div>
          );
        })}
      </div>
    );
  },
);

ScrollContainer.displayName = 'ScrollContainer';

export default ScrollContainer;
