import { useRef, useCallback, useState, useEffect, useMemo, lazy, Suspense, memo } from 'react';
import { ChevronRight } from 'lucide-react';
import './App.css';
import NavBar from './components/NavBar';
import SideBar, { SIDEBAR_WIDTH, SIDEBAR_RAIL_WIDTH } from './components/SideBar';
import ErrorBoundary from './components/ErrorBoundary';
import { HomeSpace } from './components/HomeSpace';
import Onboarding from './components/Onboarding';
import { presetConfigPatch } from './lib/keybindingPresets';
import { resolveLeader } from './lib/shortcuts';
import { resolveNavHeight } from './lib/layoutUtils';
import PluginInstallDialog from './components/PluginInstallDialog';
import { usePlugins } from './hooks/usePlugins';
import { useUiEventBus } from './hooks/useUiEventBus';
import { REVIEW_REQUEST_FILE_EVENT, openReviewFile, type ReviewFileTarget } from './lib/reviewBus';
import {
  AGENT_WATCH_EVENT,
  SESSION_WATCH_EVENT,
  AGENT_HANDOFF_EVENT,
  INSPECTOR_OPEN_EVENT,
  CONTEXT_OPEN_EVENT,
  type AgentWatchTarget,
  type SessionWatchTarget,
  type HandoffTarget,
  type InspectorTarget,
  type ContextTarget,
} from './lib/watchBus';
import { requestSettingsSection } from './lib/settingsBus';
import type { UpdateStatus } from './types/electron';
import { EDITOR_OPEN_FILE_EVENT } from './lib/editorBus';
import { MARKDOWN_PREVIEW_EVENT, type MarkdownPreviewTarget } from './lib/previewBus';
import { BROWSER_OPEN_EVENT, type BrowserOpenTarget } from './lib/browserBus';
import { useUiCommands } from './hooks/useUiCommands';
import type { PluginPane } from './types/plugin';
import SpawnAgentDialog from './components/SpawnAgentDialog';
// Lazy-loaded so qrcode.react (pulled in by RemoteShareDialog) stays off the
// startup bundle — it only mounts when the user opens the share panel.
const RemoteShareDialog = lazy(() => import('./components/RemoteShareDialog'));
import WebFolderPicker from './components/WebFolderPicker';
import SystemNotices from './components/SystemNotices';
import NotificationToasts from './components/notifications/NotificationToasts';
import { NotificationsProvider } from './contexts/NotificationsContext';
import ScrollContainer, { ScrollContainerRef } from './components/ScrollContainer';
import ShortcutOverlay from './components/ShortcutOverlay';
import ChordHint from './components/ChordHint';
import CommandPalette from './components/CommandPalette';
import LayoutsDialog from './components/LayoutsDialog';
import LibraryHost from './components/LibraryHost';
import LibrarySidePanel from './components/LibrarySidePanel';
import BottomTerminalPanel from './components/BottomTerminalPanel';
import InboxDrawer from './components/InboxDrawer';
import FleetDeck from './components/FleetDeck';
import { WorkflowOverlay } from './components/WorkflowOverlay';
import { AttentionProvider } from './contexts/AttentionContext';
import { useAttentionFeed, type AttentionFeed } from './hooks/useAttentionFeed';
import type { Layout, LayoutAgent } from './types/layout';
import { useLibrary } from './hooks/useLibrary';
import { useLayoutSync, type HydrationResult } from './hooks/useLayoutSync';
import { useHubReconnect } from './hooks/useHubReconnect';
import { useAgentManager, GLOBAL_WORKSPACE_ID } from './hooks/useAgentManager';
import type { PaneType, AgentWorkspace, AgentProvider, ViewLevel } from './types/pane';
import type { SessionAmbientState, ClaudeSessionSnapshot } from './types/claudeSession';
import { useKeyboardNav } from './hooks/useKeyboardNav';
import { useIsSmallScreen } from './hooks/useMediaQuery';
import { useConfig, DEFAULT_CONFIG } from './hooks/useConfig';
import { useUiMode } from './hooks/useUiMode';
import { useTheme } from './hooks/useTheme';
import { useSessionLifecycle } from './hooks/useSessionLifecycle';
import { useRecentSessions } from './hooks/useRecentSessions';
import { filterResumableSessions, recentSessionLabel } from './lib/recentSessionFilter';
import type { RecentAgentSession } from '../../main/shared/ipcTypes';
import { usePluginHotkeys } from './hooks/usePluginHotkeys';
import { buildPaneMenu } from './lib/paneMenu';
import { PaneMenuProvider, type PaneMenuContextValue } from './contexts/PaneMenuContext';
import { compactClaudeSnapshotForBackground } from './lib/compactClaudeSnapshot';
import { wasSessionTerminated } from './lib/terminatedSessions';
import { promoteSessionSnapshots } from './lib/promoteSessionSnapshots';

/** Normalize a workspace dir into a stable config key (slashes + no trailing /). */
function scriptKey(cwd: string): string {
  return cwd.replace(/\\/g, '/').replace(/\/+$/, '');
}

/**
 * Pure helper — normalizes a raw saved-session blob into a canonical
 * { agents, activeAgentId, name } shape that `loadAgentsFromSession` can
 * consume.  Handles all legacy session variants:
 *
 *  1. Modern   — data.agents is an array  → pass through as-is
 *  2. tabs-only  — data.tabs present but no agents → wrap into one agent
 *  3. panes-only — data.panes present but no tabs  → each pane becomes a tab
 *  4. neither  — null / empty data → empty session (no agents)
 *
 * `cwd` is the fallback working directory assigned to migrated agents when
 * the saved data doesn't carry one (typically the app's process cwd).
 *
 * Exported for unit tests only; callers inside the module use handleResumeSession.
 */
export function migrateSessionData(
  data: any,
  cwd: string,
): { agents: AgentWorkspace[]; activeAgentId: string; name: string } {
  if (data && Array.isArray(data.agents)) {
    // Modern format — pass through as-is.
    return {
      agents: data.agents,
      activeAgentId: data.activeAgentId || '',
      name: data.name || 'Default',
    };
  }
  if (data && (data.tabs?.length > 0 || data.panes?.length > 0)) {
    // Backward compat: old flat workspace → wrap its tabs into one agent.
    const oldTabs =
      data.tabs?.length > 0
        ? data.tabs
        : data.panes.map((p: any) => ({
            id: `tab-${p.id}`,
            title: p.title,
            panes: [p],
            activePaneId: p.id,
          }));
    const migrated: AgentWorkspace = {
      id: `agent-migrated-legacy`,
      name: data.name || 'Imported',
      cwd,
      tabs: oldTabs,
      activeTabId: data.activeTabId || oldTabs[0]?.id || '',
    };
    return { agents: [migrated], activeAgentId: migrated.id, name: data.name || 'Default' };
  }
  // Null / empty data → empty session.
  return { agents: [], activeAgentId: '', name: 'Default' };
}

/** Stable per-agent callbacks/props, bundled once in App so the memo below holds. */
interface AgentViewHandlers {
  onTabFocus: (tabId: string) => void;
  onPaneClose: (tabId: string, paneId: string) => void;
  onPaneFocus: (tabId: string, paneId: string) => void;
  onTabRename: (tabId: string, title: string) => void;
  onTabMove: (tabId: string, toIndex: number) => void;
  onPtyReady: (paneId: string, ptySessionId: string) => void;
  onUrlChange: (tabId: string, paneId: string, url: string) => void;
  onNavigateToTab: (tabId: string) => void;
  onAddTab: (
    type: PaneType,
    shell?: string,
    label?: string,
    cwd?: string,
    profileId?: string,
    resumeSessionId?: string,
    attachSessionId?: string,
  ) => void;
  onSplit: (tabId: string, type: PaneType) => void;
  onSplitPlugin: (tabId: string, pane: PluginPane) => void;
  spawnSupervisor: (opts: {
    question?: string;
    parentId?: string;
    provider?: AgentProvider;
  }) => Promise<string>;
  onJumpToAgent: (agentId: string) => void;
}

interface AgentWorkspaceViewProps {
  agent: AgentWorkspace;
  isActiveAgent: boolean;
  /** The agent's live working tree when it differs from its home cwd. */
  liveCwd?: string;
  scrollContainerRef: React.Ref<ScrollContainerRef>;
  ptyMapping: Record<string, string>;
  renameSignal: number;
  workspaceAgents: { sessionId?: string }[];
  appCwd: string;
  allAgents: AgentWorkspace[];
  handlers: AgentViewHandlers;
}

/**
 * One agent's mounted workspace. React.memo'd so a snapshot/state change scoped
 * to agent X reconciles only X's subtree instead of cascading into every other
 * mounted agent's ScrollContainer. The no-remount constraint is preserved: the
 * `display:none` wrapper lives here and stays mounted for inactive agents.
 *
 * For the memo to actually hold, every prop must be stable across unrelated
 * renders — App passes a single bundled `handlers` object plus memoized
 * arrays, so the only props that move for agent X are X's own `agent`/active
 * flag (and the genuinely-shared `allAgents`/`ptyMapping`).
 */
const AgentWorkspaceView = memo(function AgentWorkspaceView({
  agent,
  isActiveAgent,
  liveCwd,
  scrollContainerRef,
  ptyMapping,
  renameSignal,
  workspaceAgents,
  appCwd,
  allAgents,
  handlers,
}: AgentWorkspaceViewProps) {
  return (
    <div style={{ display: isActiveAgent ? 'block' : 'none', height: '100%' }}>
      <ErrorBoundary label="Workspace" variant="region" resetKeys={[agent.id]}>
        <ScrollContainer
          ref={isActiveAgent ? scrollContainerRef : undefined}
          ownerAgentId={agent.id}
          agentActive={isActiveAgent}
          tabs={agent.tabs}
          activeTabId={agent.activeTabId}
          onTabFocus={handlers.onTabFocus}
          onPaneClose={handlers.onPaneClose}
          onPaneFocus={handlers.onPaneFocus}
          onTabRename={handlers.onTabRename}
          onTabMove={handlers.onTabMove}
          onPtyReady={handlers.onPtyReady}
          onUrlChange={handlers.onUrlChange}
          onNavigateToTab={handlers.onNavigateToTab}
          onAddTab={handlers.onAddTab}
          onSplit={handlers.onSplit}
          onSplitPlugin={handlers.onSplitPlugin}
          ptyMapping={ptyMapping}
          renameSignal={renameSignal}
          workspaceAgents={workspaceAgents}
          appCwd={appCwd}
          agentLiveCwd={liveCwd}
          allAgents={allAgents}
          spawnSupervisor={handlers.spawnSupervisor}
          onJumpToAgent={handlers.onJumpToAgent}
        />
      </ErrorBoundary>
    </div>
  );
});

function App() {
  const { config, loaded: configLoaded, save: saveConfig } = useConfig();
  // App-wide UI mode (config.ui.mode): 'fleet' keeps the full mission-control
  // chrome; 'focus' strips down to the piloted agent. A lens, not a layout —
  // switching modes must never remount panes or touch sessions.
  const { manifest: uiManifest, toggle: toggleUiMode } = useUiMode();
  useTheme();

  // Shared-layout hydration gate (tmux-style mirror). Until the hub's layout
  // document is read we don't know whether to adopt a shared layout or run the
  // local session picker, so session restore waits on this:
  //   'pending'  — still reading the hub
  //   'adopted'  — the hub had a layout and auto-resume is on; we mirrored it
  //                and skip the picker
  //   'empty'    — nothing to adopt (no shared layout, or auto-resume is off);
  //                run normal session restore (which then seeds the hub via
  //                useLayoutSync's push)
  const [hubHydration, setHubHydration] = useState<HydrationResult>('pending');
  const {
    agents,
    activeAgentId,
    activeAgent,
    spawnAgent,
    spawnSupervisor,
    adoptAgent,
    respawnAgent,
    respawnAgentWithSettings,
    terminateAgent,
    renameAgent,
    reconcileAgents,
    stopAgentForSession,
    loadAgentsFromSession,
    openPaneIn,
    openAgentWatch,
    openInspector,
    openContext,
    openMarkdownPreview,
    setActiveAgentId,
    tabs,
    activeTabId,
    setActiveTabId,
    addTab,
    splitTab,
    removeTab,
    removePane,
    renameTab,
    moveTab,
    setActivePane,
    hibernatePane,
    wakePane,
    updatePaneUrl,
    getActiveTab,
  } = useAgentManager();

  const scrollContainerRef = useRef<ScrollContainerRef>(null);
  const [showHelp, setShowHelp] = useState(false);
  const [renameSignal, setRenameSignal] = useState(0);
  const [chordPath, setChordPath] = useState<string[] | null>(null);
  const [showCommandPalette, setShowCommandPalette] = useState(false);
  // The welcome card replayed from the palette (modal; independent of the
  // first-run onboardingDismissed flag).
  const [showWelcome, setShowWelcome] = useState(false);
  // In-app update status (main pushes transitions; 'unsupported' in dev/web).
  const [updateStatus, setUpdateStatus] = useState<UpdateStatus | null>(null);
  useEffect(() => {
    let cancelled = false;
    window.electronAPI
      .updatesGetStatus?.()
      .then((s) => {
        if (!cancelled && s) setUpdateStatus(s);
      })
      .catch(() => {});
    const off = window.electronAPI.onUpdateStatus?.((s) => setUpdateStatus(s));
    return () => {
      cancelled = true;
      off?.();
    };
  }, []);
  const [paletteMode, setPaletteMode] = useState<'tab' | 'split'>('tab');
  const [paletteRestrict, setPaletteRestrict] = useState<'library' | undefined>(undefined);
  const [showSpawnDialog, setShowSpawnDialog] = useState(false);
  // When the new-agent view is opened for a specific directory (e.g. a
  // dashboard favourite/recent), this holds its cwd so the dialog opens
  // pre-filled there instead of at the configured default. Cleared on close.
  const [spawnDialogCwd, setSpawnDialogCwd] = useState<string | null>(null);
  const [showLayouts, setShowLayouts] = useState(false);
  const [showRemote, setShowRemote] = useState(false);
  const [showLibraryPanel, setShowLibraryPanel] = useState(false);
  const [showBottomTerminal, setShowBottomTerminal] = useState(false);
  // On phone-sized viewports the sidebar starts collapsed and floats as an
  // overlay rather than reserving a column.
  const isSmallScreen = useIsSmallScreen();
  const [sidebarCollapsed, setSidebarCollapsed] = useState(isSmallScreen);
  // Auto-collapse when crossing into a small screen and auto-expand when
  // crossing back out. Manual toggles between breakpoint crossings are
  // preserved (we only react to the transition, not every render).
  const prevSmallScreen = useRef(isSmallScreen);
  useEffect(() => {
    if (isSmallScreen !== prevSmallScreen.current) {
      prevSmallScreen.current = isSmallScreen;
      setSidebarCollapsed(isSmallScreen);
    }
  }, [isSmallScreen]);
  // Focus mode (manifest.sidebar === 'rail') forces the icons-only rail on
  // desktop; the rail's expand affordance then shows the FULL sidebar as a
  // temporary overlay (scrim + floating panel) instead of re-reserving the
  // column. Small screens keep their existing overlay behavior in both modes.
  const sidebarRailForced = !isSmallScreen && uiManifest.sidebar === 'rail';
  const [focusSidebarOverlay, setFocusSidebarOverlay] = useState(false);
  // Leaving focus mode dismisses the overlay and reverts sidebarCollapsed to
  // the breakpoint default; entering it just dismisses any stale overlay.
  const prevRailForced = useRef(sidebarRailForced);
  useEffect(() => {
    if (sidebarRailForced === prevRailForced.current) return;
    prevRailForced.current = sidebarRailForced;
    setFocusSidebarOverlay(false);
    if (!sidebarRailForced) setSidebarCollapsed(isSmallScreen);
  }, [sidebarRailForced, isSmallScreen]);
  // The sidebar toggle (rail chevron, Ctrl+Shift+B, palette): in forced-rail
  // mode it opens/closes the temporary full-sidebar overlay; otherwise it
  // collapses/expands the panel as before.
  const toggleSidebar = useCallback(() => {
    if (prevRailForced.current) setFocusSidebarOverlay((v) => !v);
    else setSidebarCollapsed((v) => !v);
  }, []);
  // Layout offsets. On small screens the sidebar overlays the content, so we
  // never reserve space (navbar keeps a small inset for the floating toggle).
  // On desktop, collapsing shrinks the panel to a 74px monogram rail that still
  // reserves its column, rather than fully hiding.
  const sidebarOverlay = isSmallScreen;
  const railShown = sidebarCollapsed || sidebarRailForced;
  const contentLeft = sidebarOverlay ? 0 : railShown ? SIDEBAR_RAIL_WIDTH : SIDEBAR_WIDTH;
  const navLeft = sidebarOverlay ? 36 : railShown ? SIDEBAR_RAIL_WIDTH : SIDEBAR_WIDTH;

  // App working directory (used as the default cwd for the spawn dialog + the
  // Library's fallback project root).
  const appCwdRef = useRef<string>('');
  // Latest plugin panes, mirrored into a ref so openFileInEditor (defined above
  // usePlugins) can resolve the editor plugin at call time without reordering.
  const pluginPanesRef = useRef<PluginPane[]>([]);
  // Latest active agent in a ref. Deferred openers (e.g. the command palette,
  // which can hold a closure from when it was opened) read the *current* agent's
  // cwd from here, so a new terminal reliably lands in the selected agent's dir.
  const activeAgentRef = useRef(activeAgent);
  activeAgentRef.current = activeAgent;
  const [appCwd, setAppCwd] = useState('');
  // Neutral home for panes opened with no agent scope (~/.workspacer, created
  // on demand) — see handleAddTab's fallback chain.
  const scratchHomeRef = useRef<string>('');
  useEffect(() => {
    window.electronAPI
      .getSupervisorHome?.()
      .then((dir) => {
        scratchHomeRef.current = dir;
      })
      .catch(() => {});
  }, []);
  useEffect(() => {
    window.electronAPI
      .getCwd()
      .then((cwd) => {
        appCwdRef.current = cwd;
        setAppCwd(cwd);
      })
      .catch(() => {});
  }, []);

  // Session lifecycle — the single implicit session: boot restores the most
  // recent saved layout, saves continuously, no picker.
  const {
    sessionPhase,
    setSessionPhase,
    sessionName,
    ptyMapping,
    handlePtyReady,
    saveCurrentSession,
  } = useSessionLifecycle({
    // Hold session restore until the hub layout has been read. If the hub
    // already has a shared layout we adopt it instead (hubHydration === 'adopted'
    // never unblocks startup); only 'empty' falls through to local restore.
    configLoaded: configLoaded && hubHydration === 'empty',
    agents,
    activeAgentId,
    loadAgentsFromSession,
    reconcileAgents,
    appCwdRef,
  });

  // Sidebar RECENT list — daemon sessions (all providers, incl. archived) with
  // no card in the current layout. Clicking one respawns it as an agent.
  const allDaemonSessions = useRecentSessions(sessionPhase === 'active');
  const recentSessions = useMemo(
    () => filterResumableSessions(allDaemonSessions, agents, ptyMapping),
    [allDaemonSessions, agents, ptyMapping],
  );
  const handleResumeRecentSession = useCallback(
    (s: RecentAgentSession) => {
      const provider: AgentProvider = (['claude', 'codex', 'opencode', 'pi'] as const).includes(
        s.provider as AgentProvider,
      )
        ? (s.provider as AgentProvider)
        : 'claude';
      void spawnAgent({
        cwd: s.cwd,
        // Same label the RECENT row showed (explicit name, else the provider's
        // auto title) so the card doesn't rename itself on resume.
        name: recentSessionLabel(s),
        provider,
        // Transport and model only steer Claude spawns; managed providers
        // resolve their own settings from the resumed thread. A recorded 'pty'
        // is how every legacy row reads (the daemon's Transport default), not
        // a user choice — leave it undefined so the config default decides,
        // and only pin a session that genuinely ran stream.
        ...(provider === 'claude' && {
          transport: s.transport === 'stream' ? ('stream' as const) : undefined,
          model: s.model || undefined,
        }),
        resumeSessionId: s.sessionId,
      });
    },
    [spawnAgent],
  );

  // First-run welcome. The global Overview workspace always exists, so "brand
  // new user" means no *real* agent workspaces. Wait for the config (the
  // dismissed flag) and for session restore to settle, so an existing user's
  // agents never race the card into a flash.
  const firstRunWelcome =
    configLoaded &&
    sessionPhase === 'active' &&
    !config.onboardingDismissed &&
    !agents.some((a) => !a.global);
  const dismissWelcome = useCallback(() => {
    setShowWelcome(false);
    if (!config.onboardingDismissed) saveConfig({ onboardingDismissed: true });
  }, [config.onboardingDismissed, saveConfig]);

  // Mirror the workspace layout across clients (desktop ⇄ web). Reads the hub
  // doc on startup (driving hubHydration above), applies remote changes, and
  // pushes local changes back so every client converges — the tmux-style mirror.
  useLayoutSync({
    agents,
    activeAgentId,
    loadAgentsFromSession,
    sessionPhase,
    setSessionPhase,
    enabled: configLoaded,
    // With the single implicit session the app always resumes on boot, so the
    // hub's persisted layout is adopted unconditionally — it's just the shared
    // copy of the same layout the local session file holds.
    adoptSharedLayout: true,
    onHydration: setHubHydration,
  });

  // Library (reusable prompts + skills): global + the active project's items.
  const libraryCwd = activeAgent?.cwd || appCwd || undefined;
  const { items: libraryItems } = useLibrary(libraryCwd);
  // Toggle the right-side Library panel (bound to the 'library-picker' shortcut,
  // default Ctrl+L). Replaces the old restricted-command-palette quick-picker.
  const toggleLibraryPanel = useCallback(() => {
    setShowLibraryPanel((v) => !v);
  }, []);

  // Live agent status: sessionId -> ambient state, sourced from claudemon.
  // We also promote the FULL snapshot per session into snapshotBySession — the
  // shared substrate the Triage Inbox and Fleet Deck both project from. (App
  // already re-renders on every status update, so storing the snapshot here is
  // no extra render churn; it just stops throwing the rich payload away.)
  const [statusBySession, setStatusBySession] = useState<Record<string, SessionAmbientState>>({});
  const [snapshotBySession, setSnapshotBySession] = useState<Record<string, ClaudeSessionSnapshot>>(
    {},
  );
  // Daemon sessions that were already alive when Workspacer launched. claudemon
  // outlives the app, so these are leftovers from a previous run. They must NOT
  // be auto-adopted as orphan cards — the user reaches them only by explicitly
  // resuming a saved session. Auto-adoption is for sessions that appear *after*
  // launch (spawned via the MCP facade or by another agent). null until the
  // first session list resolves, so adoption waits rather than guessing empty.
  const preexistingSessionIdsRef = useRef<Set<string> | null>(null);
  // Pull the full session list and promote each snapshot. Runs at mount and
  // again on every hub reconnect — while the socket is down we miss
  // `onClaudeSessionUpdate` ticks, so without this re-pull a web tab shows stale
  // (or missing) sessions until a manual refresh.
  const refreshSessionSnapshots = useCallback(() => {
    window.electronAPI
      .getAllClaudeSessions()
      .then((sessions: any[]) => {
        // Skip terminated sessions AND daemon-reported `ended` ones: ended
        // sessions never tick again, so promoting one here would pin a dead
        // snapshot the live-update cleanup can never evict.
        const { statusBySession: map, snapshotBySession: snaps } =
          promoteSessionSnapshots(sessions);
        if (preexistingSessionIdsRef.current === null) {
          preexistingSessionIdsRef.current = new Set(sessions.map((s) => s.sessionId));
        }
        setStatusBySession(map);
        setSnapshotBySession(snaps);
      })
      .catch(() => {
        // No daemon / empty list: nothing pre-existed, so adoption can proceed.
        if (preexistingSessionIdsRef.current === null) {
          preexistingSessionIdsRef.current = new Set();
        }
      });
  }, []);
  useHubReconnect(refreshSessionSnapshots);
  useEffect(() => {
    refreshSessionSnapshots();
    const unsub = window.electronAPI.onClaudeSessionUpdate((sessionId: string, snapshot: any) => {
      // An ended session will never tick again, so drop its (full-transcript)
      // snapshot + status rather than leaving it pinned in memory forever.
      // Explicitly-terminated sessions get the same treatment even before the
      // daemon reports ended: their teardown ticks (final hooks / statusline)
      // would otherwise re-promote the snapshot and auto-adopt would resurrect
      // the card the user just closed.
      if (snapshot.status === 'ended' || wasSessionTerminated(sessionId)) {
        setStatusBySession((prev) => {
          if (!(sessionId in prev)) return prev;
          const { [sessionId]: _drop, ...rest } = prev;
          return rest;
        });
        setSnapshotBySession((prev) => {
          if (!(sessionId in prev)) return prev;
          const { [sessionId]: _drop, ...rest } = prev;
          return rest;
        });
        // Flip the owning agent to stopped so the card offers a respawn right
        // away. (No-op after an explicit terminate — the agent is already
        // gone by the time its session reports ended.)
        stopAgentForSession(sessionId);
        return;
      }
      setStatusBySession((prev) => ({ ...prev, [sessionId]: snapshot.ambientState }));
      setSnapshotBySession((prev) => ({
        ...prev,
        [sessionId]: compactClaudeSnapshotForBackground(snapshot),
      }));
    });
    return () => {
      unsub();
    };
  }, [refreshSessionSnapshots, stopAgentForSession]);

  // Drop a terminated agent's session snapshot/status from the promoted maps.
  // useAgentManager.terminateAgent removes the agent + closes the daemon session
  // but doesn't own these App-level maps, so without this they'd hold the dead
  // session's full transcript for the rest of the app's lifetime.
  const pruneSession = useCallback((sessionId: string | undefined) => {
    if (!sessionId) return;
    setStatusBySession((prev) => {
      if (!(sessionId in prev)) return prev;
      const { [sessionId]: _drop, ...rest } = prev;
      return rest;
    });
    setSnapshotBySession((prev) => {
      if (!(sessionId in prev)) return prev;
      const { [sessionId]: _drop, ...rest } = prev;
      return rest;
    });
  }, []);

  const handleTerminateAgent = useCallback(
    (agentId: string) => {
      const sid = agents.find((a) => a.id === agentId)?.sessionId;
      void terminateAgent(agentId);
      pruneSession(sid);
    },
    [agents, terminateAgent, pruneSession],
  );

  // Auto-adopt any live daemon session that has no AgentWorkspace yet (e.g. one
  // spawned externally via the MCP facade or by another agent). Gated on the
  // session-restore phase so we don't create duplicates for sessions that are
  // about to be loaded from the saved session file.
  const adoptedRef = useRef<Set<string>>(new Set());
  useEffect(() => {
    // Only adopt once a session is actually running (not while loading or while
    // the picker is up — adopting behind the picker is what surfaced leftover
    // daemon sessions as orphan cards on a fresh, un-resumed launch).
    if (sessionPhase !== 'active') return;
    // Wait until we know which sessions pre-existed this launch (see ref above).
    const preexisting = preexistingSessionIdsRef.current;
    if (preexisting === null) return;
    for (const [sessionId, snapshot] of Object.entries(snapshotBySession)) {
      // Skip ended sessions and already-adopted ones.
      if (snapshot.status === 'ended') continue;
      if (adoptedRef.current.has(sessionId)) continue;
      // Never re-adopt a session the user explicitly terminated — its dying
      // ticks can race the terminate and make it look live for a moment.
      if (wasSessionTerminated(sessionId)) continue;
      // Skip leftovers from a previous run — reachable only via explicit resume.
      if (preexisting.has(sessionId)) continue;
      // Skip if some agent already owns this session.
      if (agents.some((a) => a.sessionId === sessionId)) continue;
      // Mark as adopted before calling to avoid redundant calls from re-renders.
      adoptedRef.current.add(sessionId);
      adoptAgent({
        sessionId,
        cwd: snapshot.cwd,
        name: snapshot.label,
        parentSessionId: snapshot.parentSessionId,
        provider: snapshot.provider as AgentProvider | undefined,
        transport: snapshot.transport,
      });
    }
  }, [snapshotBySession, agents, adoptAgent, sessionPhase]);

  // Primary attention surface plus the cross-agent fleet surface.
  const [inboxOpen, setInboxOpen] = useState(false);
  const openInbox = useCallback(() => setInboxOpen(true), []);
  const closeInbox = useCallback(() => setInboxOpen(false), []);
  const toggleInbox = useCallback(() => setInboxOpen((v) => !v), []);

  // Altitude: 'piloting' (inside one agent) vs 'fleet' (the cross-agent surface).
  const viewLevel: ViewLevel = config.panes?.viewLevel === 'fleet' ? 'fleet' : 'piloting';
  // Focus mode unmounts the deck but leaves the persisted viewLevel alone — so
  // altitude consumers (attention auto-dismiss, the sidebar's deck state) must
  // see the altitude the user actually experiences: always 'piloting' when the
  // deck can't mount.
  const effectiveViewLevel: ViewLevel = uiManifest.fleetDeck ? viewLevel : 'piloting';
  const setViewLevel = useCallback(
    (next: ViewLevel) => {
      saveConfig({ panes: { ...config.panes, viewLevel: next } });
    },
    [config.panes, saveConfig],
  );
  const toggleFleet = useCallback(() => {
    // In focus mode the overview never mounts — instead of a dead key, the
    // toggle is an escape hatch: switch the UI mode to 'fleet' and open it.
    if (!uiManifest.fleetDeck) {
      saveConfig({
        ui: { ...config.ui, mode: 'fleet' },
        panes: { ...config.panes, viewLevel: 'fleet' },
      });
      return;
    }
    setViewLevel(viewLevel === 'fleet' ? 'piloting' : 'fleet');
  }, [uiManifest.fleetDeck, config.ui, config.panes, saveConfig, viewLevel, setViewLevel]);

  const handleUrlChange = useCallback(
    (tabId: string, paneId: string, url: string) => {
      updatePaneUrl(tabId, paneId, url);
    },
    [updatePaneUrl],
  );

  // Hibernation tracking
  const lastVisibleRef = useRef<Record<string, number>>({});
  const hibernateAfter = (config.browser?.hibernateAfter ?? 300) * 1000;

  useEffect(() => {
    if (!activeTabId) return;
    const now = Date.now();
    const tab = tabs.find((t) => t.id === activeTabId);
    if (tab) {
      for (const pane of tab.panes) {
        lastVisibleRef.current[pane.id] = now;
      }
    }
  }, [activeTabId, tabs]);

  // Auto-wake hibernated panes when their tab becomes active
  useEffect(() => {
    if (!activeTabId) return;
    const tab = tabs.find((t) => t.id === activeTabId);
    if (tab) {
      for (const pane of tab.panes) {
        if (pane.hibernated) wakePane(tab.id, pane.id);
      }
    }
  }, [activeTabId, tabs, wakePane]);

  // Hibernate timer (browser panes in inactive tabs)
  useEffect(() => {
    if (hibernateAfter <= 0 || sessionPhase !== 'active') return;
    const interval = setInterval(() => {
      const now = Date.now();
      for (const tab of tabs) {
        if (tab.id === activeTabId) continue;
        for (const pane of tab.panes) {
          if (pane.type !== 'browser' || pane.hibernated) continue;
          const lastSeen = lastVisibleRef.current[pane.id] ?? 0;
          if (lastSeen > 0 && now - lastSeen > hibernateAfter) {
            hibernatePane(tab.id, pane.id);
          }
        }
      }
    }, 15000);
    return () => clearInterval(interval);
  }, [tabs, activeTabId, hibernateAfter, hibernatePane, sessionPhase]);

  // --- Normal app logic ---

  const scrollToTab = useCallback((id: string) => {
    scrollContainerRef.current?.scrollToTab(id);
  }, []);

  const toggleHelp = useCallback(() => setShowHelp((prev) => !prev), []);
  const closeHelp = useCallback(() => setShowHelp(false), []);

  const insertPosition = config.panes.insertPosition || 'after';

  const addTabWithConfig = useCallback(
    (
      type: PaneType,
      title?: string,
      shell?: string,
      url?: string,
      appMode?: boolean,
      cwd?: string,
      profileId?: string,
      resumeSessionId?: string,
      attachSessionId?: string,
      initialCommand?: string,
      filePath?: string,
      provider?: AgentProvider,
    ) => {
      return addTab(
        type,
        title,
        insertPosition,
        shell,
        url,
        appMode,
        cwd,
        profileId,
        resumeSessionId,
        attachSessionId,
        initialCommand,
        filePath,
        provider,
      );
    },
    [addTab, insertPosition],
  );

  // Open the editor. The default (CodeMirror) engine is now the sandboxed editor
  // *plugin* (workspacer.editor): we open its webview pane rooted at the project
  // dir, optionally on a specific file. The 'terminal' engine is unchanged — it
  // runs the user's $EDITOR (e.g. nvim) in a PTY pane. Outside an agent with no
  // file we fall back to the OS file picker.
  const openFileInEditor = useCallback(
    async (filePath?: string) => {
      let target = filePath;
      if (!target && !activeAgent?.cwd) {
        const picked = await window.electronAPI.pickFiles();
        target = picked?.[0];
        if (!target) return;
      }
      const agentCwd = activeAgent && !activeAgent.global ? activeAgent.cwd : undefined;
      // Scope/tree root: the project dir when the file is under it (or no file),
      // else the file's own directory.
      const dir =
        agentCwd && (!target || target.startsWith(agentCwd))
          ? agentCwd
          : target
            ? target.replace(/[\\/][^\\/]*$/, '')
            : agentCwd;
      const title = target
        ? target.split(/[\\/]/).pop() || 'Editor'
        : dir
          ? dir.split(/[\\/]/).pop() || 'Editor'
          : 'Editor';

      // Terminal engine: open the user's editor in a PTY pane (the 'editor' pane
      // type renders a TerminalPane — see ScrollContainer).
      if ((config.editor?.engine ?? 'codemirror') === 'terminal') {
        const newId = addTabWithConfig(
          'editor',
          title,
          undefined,
          undefined,
          undefined,
          dir,
          undefined,
          undefined,
          undefined,
          undefined,
          target,
        );
        requestAnimationFrame(() => scrollToTab(newId));
        return;
      }

      // Default: the editor plugin. PluginPane mints a bus token scoped to `dir`.
      const editorPane = pluginPanesRef.current.find((p) => p.pluginId === 'workspacer.editor');
      if (!editorPane) {
        // The editor plugin isn't loaded (not installed, or the hub is down /
        // mid plugin-install). Ctrl+P must never be a silent no-op — fall back
        // to a native file pick + the OS default editor so the chord always
        // does something visible, even with plugin land on fire.
        console.warn(
          '[editor] the workspacer.editor plugin is not loaded; falling back to the system editor.',
        );
        const file = target ?? (await window.electronAPI.pickFiles())?.[0];
        if (file) void window.electronAPI.fileOpenExternal?.(file);
        return;
      }
      const params = new URLSearchParams();
      if (dir) params.set('cwd', dir);
      if (target) params.set('file', target);
      const sep = editorPane.url.includes('?') ? '&' : '?';
      const url = params.toString()
        ? `${editorPane.url}${sep}${params.toString()}`
        : editorPane.url;
      const wsId = activeAgent && !activeAgent.global ? activeAgent.id : GLOBAL_WORKSPACE_ID;
      const editorTabId = openPaneIn(wsId, 'plugin', title, url, dir, editorPane.pluginId);
      requestAnimationFrame(() => scrollToTab(editorTabId));
    },
    [activeAgent, config, addTabWithConfig, scrollToTab, openPaneIn],
  );

  // Open-in-editor requests (e.g. right-click in the Review pane's file tree).
  useEffect(() => {
    const handler = (e: Event) => {
      const target = (e as CustomEvent).detail as { path?: string } | undefined;
      if (target?.path) void openFileInEditor(target.path);
    };
    window.addEventListener(EDITOR_OPEN_FILE_EVENT, handler);
    return () => window.removeEventListener(EDITOR_OPEN_FILE_EVENT, handler);
  }, [openFileInEditor]);

  // Markdown-preview requests (FileLink left-click on a .md path). The opener
  // dedupes by file, so a repeat click focuses the existing preview pane.
  useEffect(() => {
    const handler = (e: Event) => {
      const t = (e as CustomEvent).detail as MarkdownPreviewTarget | undefined;
      if (!t?.path) return;
      const tabId = openMarkdownPreview({ path: t.path, cwd: t.cwd });
      if (tabId) requestAnimationFrame(() => scrollToTab(tabId));
    };
    window.addEventListener(MARKDOWN_PREVIEW_EVENT, handler);
    return () => window.removeEventListener(MARKDOWN_PREVIEW_EVENT, handler);
  }, [openMarkdownPreview, scrollToTab]);

  // Open-in-browser requests (e.g. FileLink's "Open in browser" on an .html
  // file). Opens a normal in-app browser tab — toolbar and all — pointed at the
  // target, rather than handing the file to the OS default handler.
  useEffect(() => {
    const handler = (e: Event) => {
      const t = (e as CustomEvent).detail as BrowserOpenTarget | undefined;
      if (!t?.url) return;
      const newId = addTab(
        'browser',
        t.title || 'Browser',
        insertPosition,
        undefined,
        t.url,
        false,
      );
      requestAnimationFrame(() => scrollToTab(newId));
    };
    window.addEventListener(BROWSER_OPEN_EVENT, handler);
    return () => window.removeEventListener(BROWSER_OPEN_EVENT, handler);
  }, [addTab, insertPosition, scrollToTab]);

  const openSettings = useCallback(() => {
    const existing = tabs.find((t) => t.panes.length === 1 && t.panes[0].type === 'settings');
    if (existing) {
      setActiveTabId(existing.id);
      scrollToTab(existing.id);
    } else {
      const newId = addTabWithConfig('settings', 'Settings');
      requestAnimationFrame(() => scrollToTab(newId));
    }
  }, [tabs, addTabWithConfig, setActiveTabId, scrollToTab]);

  // Open (or focus) a Review/changes pane for the active agent's work tree.
  const openReview = useCallback(() => {
    for (const tab of tabs) {
      const pane = tab.panes.find((p) => p.type === 'review');
      if (pane) {
        setActiveTabId(tab.id);
        setActivePane(tab.id, pane.id);
        scrollToTab(tab.id);
        return;
      }
    }
    const newId = addTabWithConfig(
      'review',
      'Review',
      undefined,
      undefined,
      undefined,
      activeAgent?.cwd,
    );
    requestAnimationFrame(() => scrollToTab(newId));
  }, [tabs, activeAgent, addTabWithConfig, setActiveTabId, setActivePane, scrollToTab]);

  // Resolve the leader to what actually works on this platform: on Linux the
  // stored ctrl+space is grabbed by fcitx/ibus, so it becomes a single Alt tap
  // (see resolveLeader). This feeds both the keyboard handler and every shortcut
  // display, so chords render as "Alt T" and fire correctly, with nothing
  // rewritten on disk.
  const kbPrefix = resolveLeader(config.keybindings?.prefix ?? 'ctrl+space');
  const kbChordHints = config.keybindings?.chordHints ?? true;
  // Defaults merged under any user overrides, so shortcut badges/labels always
  // render even when the saved config only carries a partial map.
  const resolvedShortcuts = useMemo(
    () => ({ ...DEFAULT_CONFIG.keybindings?.shortcuts, ...config.keybindings?.shortcuts }),
    [config.keybindings?.shortcuts],
  );

  const activeTab = getActiveTab();

  // --- Agent handlers (defined before useKeyboardNav so it can bind them) ---
  // Latest attention feed, read via a ref so handleSelectAgent (defined before
  // the feed) can clear an agent's items without depending on `attention`.
  const attentionRef = useRef<AttentionFeed | null>(null);
  const handleSelectAgent = useCallback(
    (id: string) => {
      // Opening an agent IS triaging it: clear that agent's inbox notifications
      // (sidebar dot/glyph + the "needs you" count) so they don't linger after
      // you've clicked through to deal with it. A genuinely new request resurfaces
      // later with a different signature. This is the single choke point for both
      // the sidebar click and the Inbox/Fleet "open agent" action.
      const att = attentionRef.current;
      if (att) {
        for (const it of att.items) {
          if (it.agentId === id) att.dismiss(it.signature);
        }
      }
      setActiveAgentId(id);
      // The deck is an altitude, not a mode: picking a specific agent from
      // anywhere (sidebar included) means "fly me there" — descend to piloting
      // so the fleet overlay doesn't keep covering the newly active workspace.
      if (viewLevel === 'fleet') setViewLevel('piloting');
      const agent = agents.find((a) => a.id === id);
      if (agent && !agent.sessionId) respawnAgent(id);
    },
    [agents, setActiveAgentId, respawnAgent, viewLevel, setViewLevel],
  );

  // Record a directory at the front of the Overview's recent list (deduped, capped).
  const recordRecentDir = useCallback(
    (cwd?: string) => {
      if (!cwd) return;
      const cur = config.directories?.recent ?? [];
      if (cur[0] === cwd) return;
      const recent = [cwd, ...cur.filter((d) => d !== cwd)].slice(0, 8);
      saveConfig({ directories: { recent, favourites: config.directories?.favourites ?? [] } });
    },
    [config.directories, saveConfig],
  );

  const handleSpawnAgent = useCallback(
    (opts: {
      cwd: string;
      name?: string;
      provider?: AgentProvider;
      /** Claude only: 'pty' | 'stream'. Omitted = the config default. */
      transport?: 'pty' | 'stream';
      profileId?: string;
      model?: string;
      effort?: string;
      permissionMode?: string;
      skipPermissions?: boolean;
      mcpItemIds?: string[];
      resumeSessionId?: string;
      worktree?: boolean;
    }) => {
      setShowSpawnDialog(false);
      setSpawnDialogCwd(null);
      const provider = opts.provider ?? 'claude';
      // Remember the harness/provider used so the next new-agent view reopens on
      // it (this is what makes a favourite launch restore your last choice).
      window.electronAPI.saveConfig({ agents: { defaultProvider: provider } }).catch(() => {});
      // Remember the picked model + permission choices so they stick next time
      // — but only for Claude, so spawning a Codex/OpenCode agent doesn't clobber
      // the saved Claude defaults (those options don't apply to other providers).
      if (provider === 'claude') {
        const defaultPermissionMode =
          opts.skipPermissions === true
            ? (opts.permissionMode ?? 'bypassPermissions')
            : opts.permissionMode === 'default'
              ? ''
              : (opts.permissionMode ?? '');
        window.electronAPI
          .saveConfig({
            claude: {
              defaultModel: opts.model ?? '',
              skipPermissionsDefault: opts.skipPermissions === true,
              // Remember the chosen permission mode too, so the next new agent
              // reopens on it instead of snapping back to the default.
              defaultPermissionMode,
              // Remember the transport (PTY vs headless stream) too, so the next
              // new-agent view reopens on the last harness used.
              ...(opts.transport ? { transport: opts.transport } : {}),
            },
          })
          .catch(() => {});
      }
      recordRecentDir(opts.cwd);
      void spawnAgent(opts);
    },
    [spawnAgent, recordRecentDir],
  );

  // --- Layout templates ---

  // Snapshot the current (non-global) agents as a reusable layout: directories
  // + their pane arrangement, stripped of live session ids.
  const captureLayout = useCallback((): LayoutAgent[] => {
    return agents
      .filter((a) => !a.global)
      .map((a) => ({
        name: a.name,
        cwd: a.cwd,
        model: a.model,
        tabs: a.tabs.map((t) => ({
          title: t.title,
          panes: t.panes
            .filter((p) => p.type !== 'settings')
            .map((p) => ({
              type: p.type,
              title: p.title,
              url: p.url,
              shell: p.shell,
              cwd: p.cwd,
              pluginId: p.pluginId,
            })),
        })),
      }));
  }, [agents]);

  const handleSaveLayout = useCallback(
    (name: string) => {
      window.electronAPI.layoutsSave({ name, agents: captureLayout() }).catch((err: any) => {
        console.error('[Layout] save failed:', err);
      });
    },
    [captureLayout],
  );

  // Restore a layout: spawn a fresh agent per saved directory, then reopen its
  // non-Claude panes (spawnAgent already creates the primary Claude tab).
  const handleRestoreLayout = useCallback(
    async (layout: Layout) => {
      for (const la of layout.agents) {
        recordRecentDir(la.cwd);
        const agentId = await spawnAgent({ cwd: la.cwd, name: la.name, model: la.model });
        for (const tab of la.tabs) {
          for (const pane of tab.panes) {
            if (pane.type === 'claude') continue; // primary Claude tab already created
            openPaneIn(
              agentId,
              pane.type as PaneType,
              pane.title,
              pane.url,
              pane.cwd ?? la.cwd,
              pane.pluginId,
            );
          }
        }
      }
    },
    [spawnAgent, openPaneIn, recordRecentDir],
  );

  const openAnalytics = useCallback(() => {
    setShowCommandPalette(false);
    // Analytics lives in the catalog plugin now (djtouchette.analytics); the
    // legacy 'analytics' pane type renders an install pointer for the
    // not-installed case, so this action always lands somewhere useful.
    const plug = pluginPanesRef.current.find((p) => p.pluginId === 'djtouchette.analytics');
    let tabId: string;
    if (plug) {
      // Bake in the static per-plugin bus token: this is a global-scope pane
      // (no cwd), so PluginPane won't mint an ephemeral one — without the token
      // the webview connects to /bus unauthenticated and shows "Bus disconnected".
      const params = new URLSearchParams();
      if (plug.busToken) params.set('busToken', plug.busToken);
      const sep = plug.url.includes('?') ? '&' : '?';
      const url = params.toString() ? `${plug.url}${sep}${params.toString()}` : plug.url;
      tabId = openPaneIn(GLOBAL_WORKSPACE_ID, 'plugin', 'Analytics', url, undefined, plug.pluginId);
    } else {
      tabId = openPaneIn(GLOBAL_WORKSPACE_ID, 'analytics', 'Analytics');
    }
    requestAnimationFrame(() => scrollToTab(tabId));
  }, [openPaneIn, scrollToTab]);

  /** Open the Agent Monitor pane in the global workspace. */
  const openAgentsPane = useCallback(() => {
    setShowCommandPalette(false);
    const tabId = openPaneIn(GLOBAL_WORKSPACE_ID, 'agents', 'Agent Monitor');
    requestAnimationFrame(() => scrollToTab(tabId));
  }, [openPaneIn, scrollToTab]);

  /** Open an Inspector pane for the currently-piloted agent (command-palette
   *  entry). The pane binds to that agent's session and live-updates; needs a
   *  running session to inspect. */
  const openInspectorForActive = useCallback(() => {
    setShowCommandPalette(false);
    const target = activeAgentRef.current;
    if (!target || target.global || !target.sessionId) return;
    const tabId = openInspector({ sessionId: target.sessionId, agentName: target.name });
    if (tabId) requestAnimationFrame(() => scrollToTab(tabId));
  }, [openInspector, scrollToTab]);

  /** Open a Context pane for the currently-piloted agent (command-palette
   *  entry). Itemizes what occupies that session's context window. */
  const openContextForActive = useCallback(() => {
    setShowCommandPalette(false);
    const target = activeAgentRef.current;
    if (!target || target.global || !target.sessionId) return;
    const tabId = openContext({ sessionId: target.sessionId, agentName: target.name });
    if (tabId) requestAnimationFrame(() => scrollToTab(tabId));
  }, [openContext, scrollToTab]);

  /** Open the Ask pane in the global Overview workspace (command-palette entry
   *  "Ask the fleet"). Reuses an existing Ask tab rather than opening a duplicate. */
  const openAskPane = useCallback(() => {
    setShowCommandPalette(false);
    const tabId = openPaneIn(GLOBAL_WORKSPACE_ID, 'ask', 'Ask');
    requestAnimationFrame(() => scrollToTab(tabId));
  }, [openPaneIn, scrollToTab]);

  /** Jump to a specific agent by id — passed down to the Ask pane. */
  const handleJumpToAgent = useCallback(
    (agentId: string) => {
      handleSelectAgent(agentId);
    },
    [handleSelectAgent],
  );

  const goToAgent = useCallback(
    (delta: number) => {
      if (agents.length === 0) return;
      const idx = agents.findIndex((a) => a.id === activeAgentId);
      const base = idx < 0 ? 0 : idx;
      const next = (base + delta + agents.length) % agents.length;
      handleSelectAgent(agents[next].id);
    },
    [agents, activeAgentId, handleSelectAgent],
  );

  const handlePrevAgent = useCallback(() => goToAgent(-1), [goToAgent]);
  const handleNextAgent = useCallback(() => goToAgent(1), [goToAgent]);
  const handleSpawnAgentShortcut = useCallback(() => setShowSpawnDialog(true), []);

  // The single cross-agent attention feed — lifted here so the SAME instance
  // (its dismiss/snooze state included) drives goToNextAttention below and the
  // SideBar / Inbox / Fleet via AttentionProvider. This is the spine.
  const attention = useAttentionFeed(snapshotBySession, agents);
  // Expose the live feed to handleSelectAgent (declared above) so selecting an
  // agent clears its notifications. Assigning a ref during render is safe here.
  attentionRef.current = attention;

  // Brief "all clear" pulse on the sidebar header when goToNextAttention finds
  // nothing — the only feedback we have without a toast system.
  const [noAttentionFlash, setNoAttentionFlash] = useState(false);
  const noAttentionTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Jump to the next item in the SORTED attention feed (priority order), wrapping
  // around starting just after the active agent so all kinds — question / stuck /
  // error / done as well as approvals — are reachable. Flashes if nothing needs you.
  const goToNextAttention = useCallback(() => {
    const feed = attention.items;
    if (feed.length === 0) {
      setNoAttentionFlash(true);
      if (noAttentionTimer.current) clearTimeout(noAttentionTimer.current);
      noAttentionTimer.current = setTimeout(() => setNoAttentionFlash(false), 1100);
      return;
    }
    // Walk the feed in priority order, but rotate so we start AFTER the active
    // agent's items — pressing the key repeatedly cycles through everything.
    const firstForActive = feed.findIndex((it) => it.agentId === activeAgentId);
    const rotateBy = firstForActive < 0 ? 0 : firstForActive + 1;
    const ordered = [...feed.slice(rotateBy), ...feed.slice(0, rotateBy)];
    const next = ordered.find((it) => it.agentId !== activeAgentId) ?? feed[0];
    handleSelectAgent(next.agentId);
  }, [attention.items, activeAgentId, handleSelectAgent]);

  // Tell main which agent session is on screen so notifications can skip the
  // one you're watching.
  useEffect(() => {
    window.electronAPI.setActiveSession(activeAgent?.sessionId ?? null);
  }, [activeAgent?.sessionId]);

  // Clicking an OS notification focuses the window (main) and asks us to jump
  // to the agent that fired it.
  useEffect(() => {
    const unsub = window.electronAPI.onFocusAgent((sessionId) => {
      const agent = agents.find((a) => a.sessionId === sessionId);
      if (agent) handleSelectAgent(agent.id);
    });
    return unsub;
  }, [agents, handleSelectAgent]);

  // When the active agent changes, pull keyboard focus into its active pane.
  // Switching agents (sidebar click or keyboard shortcut) leaves DOM focus on
  // whatever was focused before — the sidebar button, a dialog, etc. — so the
  // new agent's pane is *shown* but keystrokes still go to the old element.
  // The per-pane `isActive` focus effects are best-effort and lose the race
  // against that external focus, so nudge focus into the active pane here.
  const prevFocusedAgentRef = useRef(activeAgentId);
  useEffect(() => {
    if (activeAgentId === prevFocusedAgentRef.current) return;
    prevFocusedAgentRef.current = activeAgentId;
    if (!activeAgentId) return;

    const activeTab = tabs.find((t) => t.id === activeTabId);
    const paneId = activeTab?.activePaneId;
    if (!paneId) return;

    // The just-shown container may have a stale scroll position (it was hidden
    // while other agents were active), so re-center its active tab.
    requestAnimationFrame(() => scrollContainerRef.current?.scrollToTab(activeTabId));

    // The active pane (and its lazily-loaded content) may not be mounted on the
    // first frame after the switch, so retry across a few frames.
    let attempts = 0;
    let raf = 0;
    const focusActivePane = () => {
      const wrapper = document.querySelector(`[data-pane-id="${paneId}"]`);
      if (wrapper) {
        // Terminal view exposes xterm's hidden textarea; GUI view exposes the
        // message input. Only one is visible at a time, so focus the first
        // visible focusable element (skips the hidden terminal textarea while
        // in GUI view).
        const candidates = wrapper.querySelectorAll<HTMLElement>('textarea, input');
        const target = Array.from(candidates).find((el) => el.offsetParent !== null);
        if (target) {
          target.focus();
          return;
        }
      }
      if (attempts++ < 15) raf = requestAnimationFrame(focusActivePane);
    };
    raf = requestAnimationFrame(focusActivePane);
    return () => cancelAnimationFrame(raf);
  }, [activeAgentId, tabs, activeTabId]);

  // ── App-wide text scale ──
  // Applied as the document root font-size: every text size in the app is
  // rem-based, so scaling the root scales all text (chrome + chat alike;
  // the chat's own guiFontScale still multiplies on top). Clamped to sane
  // bounds; mod+= / mod+- nudge by one step, mod+0 resets.
  const uiFontScale = config.ui.uiFontScale ?? 1.0;
  useEffect(() => {
    document.documentElement.style.fontSize =
      uiFontScale === 1 ? '' : `${(uiFontScale * 100).toFixed(1)}%`;
  }, [uiFontScale]);
  const setTextScale = useCallback(
    (value: number) => {
      const cur = config.ui.uiFontScale ?? 1.0;
      const next = Math.round(Math.min(1.5, Math.max(0.8, value)) * 100) / 100;
      if (next !== cur) void saveConfig({ ui: { ...config.ui, uiFontScale: next } });
    },
    [config.ui, saveConfig],
  );

  useKeyboardNav({
    tabs,
    activeTabId,
    activeTab,
    setActiveTabId,
    scrollToTab,
    addTab: addTabWithConfig,
    splitTab,
    removeTab,
    removePane,
    renameTab,
    moveTab,
    setActivePane,
    onToggleHelp: toggleHelp,
    onRenameTab: useCallback(() => setRenameSignal((s) => s + 1), []),
    prefix: kbPrefix,
    onChordPathChange: setChordPath,
    onOpenSettings: openSettings,
    onSaveSession: saveCurrentSession,
    onOpenCommandPalette: useCallback(() => {
      setPaletteRestrict(undefined);
      setPaletteMode('tab');
      setShowCommandPalette(true);
    }, []),
    onOpenSplitPalette: useCallback(() => {
      setPaletteRestrict(undefined);
      setPaletteMode('split');
      setShowCommandPalette(true);
    }, []),
    onOpenFile: openFileInEditor,
    onPrevAgent: handlePrevAgent,
    onNextAgent: handleNextAgent,
    onNextAttention: goToNextAttention,
    onSpawnAgent: handleSpawnAgentShortcut,
    onToggleTerminal: useCallback(() => setShowBottomTerminal((v) => !v), []),
    onToggleSidebar: toggleSidebar,
    onToggleInbox: toggleInbox,
    onToggleFleet: toggleFleet,
    onToggleUiMode: toggleUiMode,
    onTextSizeUp: useCallback(() => setTextScale(uiFontScale + 0.05), [setTextScale, uiFontScale]),
    onTextSizeDown: useCallback(
      () => setTextScale(uiFontScale - 0.05),
      [setTextScale, uiFontScale],
    ),
    onTextSizeReset: useCallback(() => setTextScale(1.0), [setTextScale]),
    onOpenReview: openReview,
    shortcuts: resolvedShortcuts,
  });

  // Escape exits the Fleet Deck back to piloting (when the inbox isn't
  // capturing). Only while the deck can actually be on screen — in focus mode
  // it never mounts, so Escape must not be swallowed there.
  useEffect(() => {
    if (!uiManifest.fleetDeck || viewLevel !== 'fleet') return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && !inboxOpen) {
        e.preventDefault();
        setViewLevel('piloting');
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [uiManifest.fleetDeck, viewLevel, inboxOpen, setViewLevel]);

  const handleTabClick = useCallback(
    (id: string) => {
      setActiveTabId(id);
      scrollToTab(id);
    },
    [setActiveTabId, scrollToTab],
  );

  const handleTabFocus = useCallback(
    (id: string) => {
      setActiveTabId(id);
    },
    [setActiveTabId],
  );

  const handlePaneClose = useCallback(
    (tabId: string, paneId: string) => {
      removePane(tabId, paneId);
    },
    [removePane],
  );

  const handlePaneFocus = useCallback(
    (tabId: string, paneId: string) => {
      setActiveTabId(tabId);
      setActivePane(tabId, paneId);
    },
    [setActiveTabId, setActivePane],
  );

  const handleAddTab = useCallback(
    (
      type: PaneType,
      shell?: string,
      label?: string,
      cwd?: string,
      profileId?: string,
      resumeSessionId?: string,
      attachSessionId?: string,
    ) => {
      // The editor is opened through openFileInEditor (→ the editor plugin, or the
      // terminal engine), so "New → Editor" / command-palette routes there too.
      if (type === 'editor') {
        void openFileInEditor();
        return;
      }
      // If opening a Claude session that already has a tab, navigate to it.
      const sessionId = resumeSessionId || attachSessionId;
      if (type === 'claude' && sessionId) {
        for (const tab of tabs) {
          const match = tab.panes.find(
            (p) =>
              p.resumeSessionId === sessionId ||
              p.attachSessionId === sessionId ||
              ptyMapping[p.id] === sessionId,
          );
          if (match) {
            setActiveTabId(tab.id);
            setActivePane(tab.id, match.id);
            scrollToTab(tab.id);
            return;
          }
        }
      }
      // New panes inherit the active agent's working directory. Read it from the
      // ref so a stale caller closure (e.g. the command palette) still resolves the
      // currently-selected agent's cwd. With no agent scope (the Overview
      // workspace), fall back to the neutral ~/.workspacer home rather than the
      // app's own launch directory — an Overview terminal pane shouldn't
      // land inside whatever repo happened to launch workspacer.
      const resolvedCwd =
        cwd ||
        activeAgentRef.current?.cwd ||
        scratchHomeRef.current ||
        appCwdRef.current ||
        undefined;
      const newId = addTabWithConfig(
        type,
        label,
        shell,
        undefined,
        undefined,
        resolvedCwd,
        profileId,
        resumeSessionId,
        attachSessionId,
      );
      requestAnimationFrame(() => scrollToTab(newId));
    },
    [
      tabs,
      ptyMapping,
      addTabWithConfig,
      setActiveTabId,
      setActivePane,
      scrollToTab,
      openFileInEditor,
    ],
  );

  const handleSplitPane = useCallback(
    (type: PaneType, shell?: string, label?: string, cwd?: string) => {
      if (!activeTabId) return;
      const resolvedCwd = cwd || activeAgentRef.current?.cwd;
      splitTab(activeTabId, type, label, shell, undefined, undefined, resolvedCwd);
    },
    [activeTabId, splitTab],
  );

  // Open a changed file in the Review pane (from the Claude pane's file list).
  // Focus an existing Review pane in the active agent if there is one, else
  // open a new one; then hand the target file to the (now-mounted) pane. The
  // double rAF lets a freshly-created pane mount + attach its listener first.
  useEffect(() => {
    const handler = (e: Event) => {
      const target = (e as CustomEvent).detail as ReviewFileTarget | undefined;
      if (!target?.path) return;
      const cwd = target.cwd || activeAgent?.cwd;
      const targetAgent = target.agentId ? agents.find((a) => a.id === target.agentId) : null;

      if (targetAgent && !targetAgent.global) {
        const tabId = openPaneIn(targetAgent.id, 'review', 'Review', undefined, cwd);
        requestAnimationFrame(() => scrollToTab(tabId));
        requestAnimationFrame(() =>
          requestAnimationFrame(() =>
            openReviewFile({ path: target.path, cwd, agentId: target.agentId }),
          ),
        );
        return;
      }

      // Only reuse a Review pane diffing the SAME tree. A worktree request
      // must not land on the home repo's pane — ReviewPane ignores open-file
      // events whose cwd differs from its own, so the click would do nothing.
      const norm = (p?: string) => (p ?? '').replace(/\\/g, '/').replace(/\/+$/, '').toLowerCase();
      let existing: { tabId: string; paneId: string } | null = null;
      for (const tab of tabs) {
        const pane = tab.panes.find(
          (p) => p.type === 'review' && (!cwd || !p.cwd || norm(p.cwd) === norm(cwd)),
        );
        if (pane) {
          existing = { tabId: tab.id, paneId: pane.id };
          break;
        }
      }
      if (existing) {
        setActiveTabId(existing.tabId);
        setActivePane(existing.tabId, existing.paneId);
        scrollToTab(existing.tabId);
      } else {
        handleAddTab('review', undefined, 'Review', cwd);
      }
      requestAnimationFrame(() =>
        requestAnimationFrame(() => openReviewFile({ path: target.path, cwd })),
      );
    };
    window.addEventListener(REVIEW_REQUEST_FILE_EVENT, handler);
    return () => window.removeEventListener(REVIEW_REQUEST_FILE_EVENT, handler);
  }, [
    tabs,
    activeAgent,
    agents,
    openPaneIn,
    setActiveTabId,
    setActivePane,
    scrollToTab,
    handleAddTab,
  ]);

  // Watch one subagent / workflow run in a dedicated pane (inspector rail
  // click-through). openAgentWatch dedupes by target, so a repeat click
  // focuses the existing watch tab.
  useEffect(() => {
    const handler = (e: Event) => {
      const t = (e as CustomEvent).detail as AgentWatchTarget | undefined;
      if (!t?.sessionId || !t.id || !t.kind) return;
      const tabId = openAgentWatch(t);
      // The pane lives under the Fleet Deck overlay — drop to piloting so the
      // click visibly lands on the opened transcript instead of nothing.
      if (viewLevel === 'fleet') setViewLevel('piloting');
      if (tabId) requestAnimationFrame(() => scrollToTab(tabId));
    };
    window.addEventListener(AGENT_WATCH_EVENT, handler);
    return () => window.removeEventListener(AGENT_WATCH_EVENT, handler);
  }, [openAgentWatch, scrollToTab, viewLevel, setViewLevel]);

  // Open a standalone Inspector pane for a session (command palette / Fleet Deck
  // "Open as pane"). openInspector dedupes by session, so a repeat request
  // focuses the existing pane.
  useEffect(() => {
    const handler = (e: Event) => {
      const t = (e as CustomEvent).detail as InspectorTarget | undefined;
      if (!t?.sessionId) return;
      const tabId = openInspector({ sessionId: t.sessionId, agentName: t.agentName });
      // Same as agent-watch: surface the pane from under the Fleet Deck.
      if (viewLevel === 'fleet') setViewLevel('piloting');
      if (tabId) requestAnimationFrame(() => scrollToTab(tabId));
    };
    window.addEventListener(INSPECTOR_OPEN_EVENT, handler);
    return () => window.removeEventListener(INSPECTOR_OPEN_EVENT, handler);
  }, [openInspector, scrollToTab, viewLevel, setViewLevel]);

  // Open a Context pane for a session (inspector Usage chips / command
  // palette). openContext dedupes by session, so a repeat request focuses the
  // existing pane, re-aimed at the requested section.
  useEffect(() => {
    const handler = (e: Event) => {
      const t = (e as CustomEvent).detail as ContextTarget | undefined;
      if (!t?.sessionId) return;
      const tabId = openContext({
        sessionId: t.sessionId,
        agentName: t.agentName,
        focus: t.focus,
      });
      if (viewLevel === 'fleet') setViewLevel('piloting');
      if (tabId) requestAnimationFrame(() => scrollToTab(tabId));
    };
    window.addEventListener(CONTEXT_OPEN_EVENT, handler);
    return () => window.removeEventListener(CONTEXT_OPEN_EVENT, handler);
  }, [openContext, scrollToTab, viewLevel, setViewLevel]);

  // Watch a whole session in a GUI viewer pane (Agents pane click-through).
  // Focus an existing viewer for that session in the current workspace, else
  // attach a new one — the pane never owns the session's lifetime.
  useEffect(() => {
    const handler = (e: Event) => {
      const t = (e as CustomEvent).detail as SessionWatchTarget | undefined;
      if (!t?.sessionId) return;
      for (const tab of tabs) {
        const match = tab.panes.find(
          (p) =>
            p.type === 'claude' &&
            (p.attachSessionId === t.sessionId || ptyMapping[p.id] === t.sessionId),
        );
        if (match) {
          setActiveTabId(tab.id);
          setActivePane(tab.id, match.id);
          scrollToTab(tab.id);
          return;
        }
      }
      const newId = addTabWithConfig(
        'claude',
        t.title,
        undefined,
        undefined,
        undefined,
        t.cwd,
        undefined,
        undefined,
        t.sessionId,
        undefined,
        undefined,
        t.provider,
      );
      requestAnimationFrame(() => scrollToTab(newId));
    };
    window.addEventListener(SESSION_WATCH_EVENT, handler);
    return () => window.removeEventListener(SESSION_WATCH_EVENT, handler);
  }, [tabs, ptyMapping, setActiveTabId, setActivePane, scrollToTab, addTabWithConfig]);

  // Restart an agent-managed session with new launch settings (composer pills
  // in an attached ClaudePane dispatch this — same CustomEvent pattern as
  // library:insert, since the pane doesn't own the agent record).
  useEffect(() => {
    const handler = (e: Event) => {
      const d = (e as CustomEvent).detail as
        | {
            sessionId?: string;
            overrides?: { model?: string; effort?: string; permissionMode?: string };
          }
        | undefined;
      if (!d?.sessionId) return;
      void respawnAgentWithSettings(d.sessionId, d.overrides ?? {});
    };
    window.addEventListener('agent:respawn', handler);
    return () => window.removeEventListener('agent:respawn', handler);
  }, [respawnAgentWithSettings]);

  // Cross-provider handoff: spawn the successor agent in the same cwd with its
  // composer pre-filled to read the brief (written by claudemon under
  // ~/.workspacer/handoffs/). The user reviews the takeover message — and can
  // append their next ask — before sending.
  useEffect(() => {
    const handler = (e: Event) => {
      const t = (e as CustomEvent).detail as HandoffTarget | undefined;
      if (!t?.targetProvider || !t.briefPath || !t.cwd) return;
      void spawnAgent({
        cwd: t.cwd,
        provider: t.targetProvider,
        name: `handoff → ${t.targetProvider}`,
        initialPrompt:
          `You are taking over an in-progress session from another AI coding agent. ` +
          `First read the handoff brief at ${t.briefPath}, then continue the work from where it left off — ` +
          `don't start over or redo completed steps. Reply with a one-paragraph summary of the state and your next step.`,
      });
    };
    window.addEventListener(AGENT_HANDOFF_EVENT, handler);
    return () => window.removeEventListener(AGENT_HANDOFF_EVENT, handler);
  }, [spawnAgent]);

  const handleLaunchApp = useCallback(
    (app: { name: string; url: string }) => {
      const newId = addTab('browser', app.name, insertPosition, undefined, app.url, true);
      requestAnimationFrame(() => scrollToTab(newId));
    },
    [addTab, insertPosition, scrollToTab],
  );

  // Publish every UI action (pane open/close, focus changes) onto the hub bus
  // so plugins/MCP can react to what's happening in the app.
  useUiEventBus(agents, activeAgentId);

  // --- Plugins (contributed panes + hotkeys from the hub) ---
  const { panes: pluginPanes, hotkeys: pluginHotkeys } = usePlugins();
  pluginPanesRef.current = pluginPanes; // let openFileInEditor resolve the editor plugin
  const [showInstallPlugin, setShowInstallPlugin] = useState(false);

  // Webview URL for a plugin pane: the plugin's static bus token baked in (so
  // its page can connect to the hub bus scoped to its capabilities), plus the
  // target agent's session/cwd for scoped panes.
  const buildPluginPaneUrl = useCallback((pane: PluginPane, target?: AgentWorkspace) => {
    const params = new URLSearchParams();
    if (pane.busToken) params.set('busToken', pane.busToken);
    if (target?.sessionId) params.set('sessionId', target.sessionId);
    if (target?.cwd) params.set('cwd', target.cwd);
    const sep = pane.url.includes('?') ? '&' : '?';
    return params.toString() ? `${pane.url}${sep}${params.toString()}` : pane.url;
  }, []);

  const handleOpenPlugin = useCallback(
    (pane: PluginPane) => {
      // Place the pane by its declared scope:
      //  - global → the Overview workspace
      //  - agent  → the active agent (else the first real agent), with that
      //             agent's session/cwd handed to the webview via query params
      //  - both   → wherever the user currently is
      const activeIsAgent = !!activeAgent && !activeAgent.global;
      let target: AgentWorkspace | undefined;
      if (pane.scope === 'global') {
        target = undefined; // global
      } else if (pane.scope === 'agent') {
        target = activeIsAgent ? activeAgent : agents.find((a) => !a.global);
      } else {
        target = activeIsAgent ? activeAgent : undefined; // 'both'
      }

      const url = buildPluginPaneUrl(pane, target);
      // Pass the plugin id + the agent's cwd so an agent-scoped pane can mint an
      // ephemeral token confined to that cwd on mount (see PluginPane). The static
      // busToken stays baked into the URL as the fallback when minting is
      // unavailable (e.g. the web build, or the hub momentarily unreachable).
      const tabId = openPaneIn(
        target ? target.id : GLOBAL_WORKSPACE_ID,
        'plugin',
        pane.title,
        url,
        target?.cwd,
        pane.pluginId,
      );
      requestAnimationFrame(() => scrollToTab(tabId));
    },
    [openPaneIn, activeAgent, agents, scrollToTab],
  );

  // Resolved pane-creation menu (built-ins + plugins, per ui.paneMenu), shared
  // with the "Split into…" button and the "+" new-tab dropdown via context.
  const paneMenuValue = useMemo<PaneMenuContextValue>(
    () => ({
      entries: buildPaneMenu(config.ui.paneMenu, pluginPanes),
      onOpenPlugin: handleOpenPlugin,
    }),
    [config.ui.paneMenu, pluginPanes, handleOpenPlugin],
  );

  // Bind plugin-contributed hotkeys + library-picker shortcut.
  usePluginHotkeys({
    pluginHotkeys,
    pluginPanes,
    handleOpenPlugin,
    libraryPickerCombo: config.keybindings?.shortcuts?.['library-picker'],
    openLibraryPicker: toggleLibraryPanel,
  });

  // Listen for bus commands (from plugins / MCP) and drive the UI. The ui.*
  // event each action emits doubles as the confirmation back on the bus.
  useUiCommands({
    focusAgent: (idOrSession) => {
      const a = agents.find((x) => x.id === idOrSession || x.sessionId === idOrSession);
      if (a) handleSelectAgent(a.id);
    },
    spawnAgent: (opts) => {
      const cwd = opts.cwd || activeAgent?.cwd || appCwdRef.current;
      if (cwd) {
        recordRecentDir(cwd);
        void spawnAgent({ cwd, name: opts.name, model: opts.model });
      }
    },
    openSpawnDialog: (opts) => {
      setSpawnDialogCwd(opts.cwd?.trim() || null);
      setShowSpawnDialog(true);
    },
    openPane: (paneType, opts) =>
      handleAddTab(paneType as PaneType, undefined, undefined, opts?.cwd),
    openPlugin: (type) => {
      const pane = pluginPanes.find((p) => p.type === type);
      if (pane) handleOpenPlugin(pane);
    },
    closePane: (paneId) => {
      for (const a of agents) {
        for (const t of a.tabs) {
          if (t.panes.some((p) => p.id === paneId)) {
            removePane(t.id, paneId);
            return;
          }
        }
      }
    },
    openAskPane,
  });

  // --- Per-directory script buttons ---
  const agentCwd = activeAgent?.cwd ?? '';
  const dirScripts = agentCwd ? (config.scripts?.[scriptKey(agentCwd)] ?? []) : [];

  // Run a script in a fresh terminal tab rooted at the agent's workspace.
  const handleRunScript = useCallback(
    (name: string, command: string) => {
      if (!agentCwd) return;
      const newId = addTabWithConfig(
        'terminal',
        name,
        undefined,
        undefined,
        undefined,
        agentCwd,
        undefined,
        undefined,
        undefined,
        command,
      );
      requestAnimationFrame(() => scrollToTab(newId));
    },
    [agentCwd, addTabWithConfig, scrollToTab],
  );

  // Persist this directory's script list to config.
  const handleSaveScripts = useCallback(
    (entries: { name: string; command: string }[]) => {
      if (!agentCwd) return;
      saveConfig({ scripts: { ...(config.scripts ?? {}), [scriptKey(agentCwd)]: entries } });
    },
    [agentCwd, config.scripts, saveConfig],
  );

  // --- Render ---
  // Phones get a taller bar so the (fattened) touch targets fit; this height
  // also drives the content top-offset below, so the two stay in sync.
  const navHeight = resolveNavHeight(config.ui.navBarHeight, isSmallScreen);

  const handleNavBarRename = useCallback(
    (tabId: string) => {
      setActiveTabId(tabId);
      setRenameSignal((s) => s + 1);
    },
    [setActiveTabId],
  );
  const handleNavBarSplit = useCallback(
    // New split panes inherit the active agent's working directory.
    (tabId: string, type: PaneType) => {
      splitTab(tabId, type, undefined, undefined, undefined, undefined, activeAgent?.cwd);
    },
    [splitTab, activeAgent],
  );
  // In-pane split (the pane-header split button). Ref-based so it stays stable
  // for the memoized agent-view handler bundle; inherits the agent's cwd (or
  // the app cwd) just like the navbar split.
  const handlePaneSplit = useCallback(
    (tabId: string, type: PaneType) => {
      splitTab(
        tabId,
        type,
        undefined,
        undefined,
        undefined,
        undefined,
        activeAgentRef.current?.cwd || appCwdRef.current || undefined,
      );
    },
    [splitTab],
  );
  // Plugin entry in the same split menu: land the pane as a SPLIT in place —
  // "Shiplight beside my agent" — scoped to the active agent (its session/cwd
  // ride the webview URL and PaneConfig, so scope-aware plugins show just that
  // project). handleOpenPlugin's tab-opening path stays for palette/menu opens,
  // where a global-scope pane should still route to the Overview workspace.
  const handlePaneSplitPlugin = useCallback(
    (tabId: string, pane: PluginPane) => {
      const active = activeAgentRef.current;
      const target = active && !active.global ? active : undefined;
      const url = buildPluginPaneUrl(pane, target);
      splitTab(tabId, 'plugin', pane.title, undefined, url, true, target?.cwd, pane.pluginId);
    },
    [splitTab, buildPluginPaneUrl],
  );

  // Stable inputs for the per-agent workspace views. `workspaceAgents` was being
  // rebuilt inline in every render of every agent's ScrollContainer, giving each
  // a fresh-identity array prop; lifting it here (recomputed only when the agent
  // session set changes) is what lets the AgentWorkspaceView memo actually hold,
  // so a snapshot for agent X no longer reconciles every other agent's subtree.
  const workspaceAgents = useMemo(
    () => agents.filter((a) => !a.global).map((a) => ({ sessionId: a.sessionId })),
    // Only the (ordered) session ids matter to consumers; depending on `agents`
    // directly would defeat the memo since that ref changes on any tab edit.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [
      agents
        .filter((a) => !a.global)
        .map((a) => a.sessionId ?? '')
        .join(','),
    ],
  );
  const hasAgentMonitorActivity = useMemo(
    () =>
      Object.values(snapshotBySession).some(
        (snapshot) =>
          (snapshot.subagents?.length ?? 0) > 0 || (snapshot.workflows?.length ?? 0) > 0,
      ),
    [snapshotBySession],
  );

  // Bundle the stable per-agent callbacks/props once so the memoized wrapper
  // sees a single stable object instead of ~14 individually-threaded props.
  const agentViewHandlers = useMemo(
    () => ({
      onTabFocus: handleTabFocus,
      onPaneClose: handlePaneClose,
      onPaneFocus: handlePaneFocus,
      onTabRename: renameTab,
      onTabMove: moveTab,
      onPtyReady: handlePtyReady,
      onUrlChange: handleUrlChange,
      onNavigateToTab: handleTabClick,
      onAddTab: handleAddTab,
      onSplit: handlePaneSplit,
      onSplitPlugin: handlePaneSplitPlugin,
      spawnSupervisor,
      onJumpToAgent: handleJumpToAgent,
    }),
    [
      handleTabFocus,
      handlePaneClose,
      handlePaneFocus,
      renameTab,
      moveTab,
      handlePtyReady,
      handleUrlChange,
      handleTabClick,
      handleAddTab,
      handlePaneSplit,
      handlePaneSplitPlugin,
      spawnSupervisor,
      handleJumpToAgent,
    ],
  );

  return (
    <NotificationsProvider
      onFocusSession={(sessionId) => {
        const agent = agents.find((a) => a.sessionId === sessionId);
        if (agent) handleSelectAgent(agent.id);
      }}
      onOpenPane={(paneType) => {
        const pane = pluginPanes.find((p) => p.type === paneType);
        if (pane) handleOpenPlugin(pane);
        else handleAddTab(paneType as PaneType);
      }}
    >
      <PaneMenuProvider value={paneMenuValue}>
        <AttentionProvider
          agents={agents}
          activeAgentId={activeAgentId}
          snapshotBySession={snapshotBySession}
          inboxOpen={inboxOpen}
          openInbox={openInbox}
          closeInbox={closeInbox}
          viewLevel={effectiveViewLevel}
          setViewLevel={setViewLevel}
          onOpenAgent={handleSelectAgent}
          onRespawnAgent={respawnAgent}
          onSpawnAgent={() => setShowSpawnDialog(true)}
          attention={attention}
        >
          <div className="app-root">
            {sidebarOverlay && !sidebarCollapsed && (
              <div
                onClick={() => setSidebarCollapsed(true)}
                style={{
                  position: 'fixed',
                  inset: 0,
                  zIndex: 90,
                  background: 'rgba(0,0,0,0.45)',
                  // @ts-ignore — stay clickable over the draggable navbar region
                  WebkitAppRegion: 'no-drag',
                }}
              />
            )}
            {/* Desktop always shows the sidebar (a rail when collapsed); mobile shows
          the full panel as an overlay only while expanded. */}
            {(!sidebarOverlay || !sidebarCollapsed) && (
              <ErrorBoundary label="Sidebar" variant="region">
                <SideBar
                  agents={agents}
                  activeAgentId={activeAgentId}
                  statusBySession={statusBySession}
                  snapshotBySession={snapshotBySession}
                  onSelectAgent={(id) => {
                    handleSelectAgent(id);
                    if (sidebarOverlay) setSidebarCollapsed(true);
                  }}
                  onSpawnAgent={() => setShowSpawnDialog(true)}
                  onTerminateAgent={handleTerminateAgent}
                  onRenameAgent={renameAgent}
                  onJumpToAttention={goToNextAttention}
                  onOpenInbox={openInbox}
                  onToggleFleet={toggleFleet}
                  viewLevel={effectiveViewLevel}
                  onOpenRemote={() => setShowRemote(true)}
                  onToggleCollapse={toggleSidebar}
                  onToggleHelp={toggleHelp}
                  noAttentionFlash={noAttentionFlash}
                  collapsed={!sidebarOverlay && railShown}
                  recentSessions={recentSessions}
                  onResumeSession={handleResumeRecentSession}
                />
              </ErrorBoundary>
            )}
            {/* Focus mode: the rail's expand affordance shows the FULL sidebar as a
          temporary overlay (scrim + floating panel) — the content column stays
          at rail width. Dismissed by scrim click or agent selection. */}
            {sidebarRailForced && focusSidebarOverlay && (
              <>
                <div
                  onClick={() => setFocusSidebarOverlay(false)}
                  style={{
                    position: 'fixed',
                    inset: 0,
                    zIndex: 90,
                    background: 'rgba(0,0,0,0.45)',
                    // @ts-ignore — stay clickable over the draggable navbar region
                    WebkitAppRegion: 'no-drag',
                  }}
                />
                <ErrorBoundary label="Sidebar" variant="region">
                  <SideBar
                    agents={agents}
                    activeAgentId={activeAgentId}
                    statusBySession={statusBySession}
                    snapshotBySession={snapshotBySession}
                    onSelectAgent={(id) => {
                      handleSelectAgent(id);
                      setFocusSidebarOverlay(false);
                    }}
                    onSpawnAgent={() => setShowSpawnDialog(true)}
                    onTerminateAgent={handleTerminateAgent}
                    onRenameAgent={renameAgent}
                    onJumpToAttention={goToNextAttention}
                    onOpenInbox={openInbox}
                    onToggleFleet={toggleFleet}
                    viewLevel={effectiveViewLevel}
                    onOpenRemote={() => setShowRemote(true)}
                    onToggleCollapse={() => setFocusSidebarOverlay(false)}
                    onToggleHelp={toggleHelp}
                    noAttentionFlash={noAttentionFlash}
                    collapsed={false}
                    recentSessions={recentSessions}
                    onResumeSession={handleResumeRecentSession}
                  />
                </ErrorBoundary>
              </>
            )}
            {sidebarOverlay && sidebarCollapsed && (
              <button
                onClick={() => setSidebarCollapsed(false)}
                title="Show sidebar (Ctrl+B)"
                style={{
                  position: 'fixed',
                  zIndex: 200,
                  // Clear the notch/status bar on phones; keep it tight on desktop.
                  top: isSmallScreen ? 'calc(env(safe-area-inset-top) + 6px)' : 6,
                  left: isSmallScreen ? 'calc(env(safe-area-inset-left) + 6px)' : 6,
                  // Larger fingertip target on phones (Apple HIG floor is ~44px).
                  width: isSmallScreen ? 38 : 26,
                  height: isSmallScreen ? 38 : 26,
                  display: 'flex',
                  alignItems: 'center',
                  justifyContent: 'center',
                  border: '1px solid var(--wks-glass-border)',
                  borderRadius: 'var(--wks-radius-md)',
                  background: 'var(--wks-bg-surface)',
                  color: 'var(--wks-text-secondary)',
                  cursor: 'pointer',
                  fontSize: isSmallScreen ? '1.1rem' : '0.95rem',
                  lineHeight: 1,
                  // @ts-ignore — keep it clickable over the draggable navbar region
                  WebkitAppRegion: 'no-drag',
                }}
              >
                <ChevronRight size={16} strokeWidth={2} />
              </button>
            )}

            <ErrorBoundary label="Tab bar" variant="region">
              <NavBar
                tabs={tabs}
                activeTabId={activeTabId}
                onTabClick={handleTabClick}
                onAddTab={handleAddTab}
                onCloseTab={removeTab}
                onRenameTab={handleNavBarRename}
                onSplitTab={handleNavBarSplit}
                onMoveTab={moveTab}
                leftOffset={navLeft}
                cwd={agentCwd || undefined}
                scripts={dirScripts}
                onRunScript={handleRunScript}
                onSaveScripts={handleSaveScripts}
              />
            </ErrorBoundary>

            <div
              className="app-content"
              style={{
                // Panes sit flush under the tab bar's divider (mockup layout).
                marginTop: `${navHeight}px`,
                marginLeft: `${contentLeft}px`,
              }}
            >
              {agents.length > 0 ? (
                // Keep every agent's workspace mounted and just toggle visibility, so
                // switching agents never unmounts a Claude pane (which would detach
                // its viewer and clear the terminal). Only the active agent's
                // container is shown and wired to the scroll ref.
                agents.map((agent) => (
                  <AgentWorkspaceView
                    key={agent.id}
                    agent={agent}
                    isActiveAgent={agent.id === activeAgentId}
                    liveCwd={
                      agent.sessionId ? snapshotBySession[agent.sessionId]?.liveCwd : undefined
                    }
                    scrollContainerRef={scrollContainerRef}
                    ptyMapping={ptyMapping}
                    renameSignal={renameSignal}
                    workspaceAgents={workspaceAgents}
                    appCwd={appCwd}
                    allAgents={agents}
                    handlers={agentViewHandlers}
                  />
                ))
              ) : (
                <HomeSpace
                  onSpawn={() => setShowSpawnDialog(true)}
                  spawnShortcut={config.keybindings?.shortcuts?.['spawn-agent'] ?? 'ctrl+shift+n'}
                />
              )}
            </div>

            <ShortcutOverlay
              visible={showHelp}
              onClose={closeHelp}
              prefix={kbPrefix}
              shortcuts={resolvedShortcuts}
            />

            {(firstRunWelcome || showWelcome) && (
              <Onboarding
                overlay
                firstRun={firstRunWelcome}
                onSpawn={() => {
                  dismissWelcome();
                  setShowSpawnDialog(true);
                }}
                onDismiss={dismissWelcome}
                onOpenKeybindings={() => {
                  dismissWelcome();
                  openSettings();
                  requestSettingsSection('keybindings');
                }}
                shortcuts={config.keybindings?.shortcuts ?? {}}
                prefix={kbPrefix}
                presetId={config.keybindings?.presetId}
                onChoosePreset={(id) => saveConfig(presetConfigPatch(id, config))}
              />
            )}

            <CommandPalette
              visible={showCommandPalette}
              apps={config.apps ?? []}
              agentCwd={agentCwd || undefined}
              mode={paletteMode}
              restrictTo={paletteRestrict}
              libraryItems={libraryItems}
              onClose={useCallback(() => {
                setShowCommandPalette(false);
                setPaletteRestrict(undefined);
              }, [])}
              onLaunchApp={handleLaunchApp}
              onAddTab={handleAddTab}
              onSplitPane={handleSplitPane}
              pluginPanes={pluginPanes}
              onOpenPlugin={handleOpenPlugin}
              onInstallPlugin={() => {
                setShowCommandPalette(false);
                setShowInstallPlugin(true);
              }}
              onManagePlugins={() => {
                setShowCommandPalette(false);
                const tabId = openPaneIn(GLOBAL_WORKSPACE_ID, 'plugins', 'Plugins');
                requestAnimationFrame(() => scrollToTab(tabId));
              }}
              onOpenLibrary={() => {
                setShowCommandPalette(false);
                // Open in the active agent's workspace (with its project cwd) so the
                // pane shows that project's library + .claude skills; fall back to
                // the global Overview when no agent is focused.
                const tabId =
                  activeAgent && !activeAgent.global
                    ? openPaneIn(activeAgent.id, 'library', 'Library', undefined, activeAgent.cwd)
                    : openPaneIn(GLOBAL_WORKSPACE_ID, 'library', 'Library');
                requestAnimationFrame(() => scrollToTab(tabId));
              }}
              onOpenAnalytics={openAnalytics}
              onOpenAgents={hasAgentMonitorActivity ? openAgentsPane : undefined}
              onOpenInspector={openInspectorForActive}
              onOpenContext={openContextForActive}
              onOpenLayouts={() => {
                setShowCommandPalette(false);
                setShowLayouts(true);
              }}
              onOpenRemote={() => {
                setShowCommandPalette(false);
                setShowRemote(true);
              }}
              onOpenAskPane={openAskPane}
              onOpenFile={() => {
                setShowCommandPalette(false);
                openFileInEditor();
              }}
              shortcuts={resolvedShortcuts}
              prefix={kbPrefix}
              onSpawnAgent={() => {
                setShowCommandPalette(false);
                setShowSpawnDialog(true);
              }}
              onShowWelcome={() => {
                setShowCommandPalette(false);
                setShowWelcome(true);
              }}
              onInstallCli={() => {
                setShowCommandPalette(false);
                // Outcome (installed / PATH instructions / failure) arrives as a
                // system-notice banner pushed by main — no local result UI needed.
                window.electronAPI.installCli?.().catch(() => {});
              }}
              updateStatus={updateStatus ?? undefined}
              onCheckUpdates={() => {
                setShowCommandPalette(false);
                window.electronAPI.updatesCheck?.().catch(() => {});
              }}
              onInstallUpdate={() => {
                setShowCommandPalette(false);
                window.electronAPI.updatesInstall?.().catch(() => {});
              }}
              onToggleSidebar={() => {
                setShowCommandPalette(false);
                toggleSidebar();
              }}
              onToggleInbox={() => {
                setShowCommandPalette(false);
                setInboxOpen((v) => !v);
              }}
              onToggleFleet={() => {
                setShowCommandPalette(false);
                toggleFleet();
              }}
              onToggleUiMode={() => {
                setShowCommandPalette(false);
                toggleUiMode();
              }}
              onSaveSession={() => {
                setShowCommandPalette(false);
                saveCurrentSession();
              }}
              onOpenSettings={() => {
                setShowCommandPalette(false);
                openSettings();
              }}
              onToggleHelp={() => {
                setShowCommandPalette(false);
                toggleHelp();
              }}
            />

            <LibraryHost
              activeAgent={activeAgent}
              appCwd={appCwd}
              spawnAgent={(opts) => {
                void spawnAgent(opts);
              }}
              recordRecentDir={recordRecentDir}
            />

            {showInstallPlugin && (
              <PluginInstallDialog onClose={() => setShowInstallPlugin(false)} />
            )}

            {showRemote && (
              <Suspense fallback={null}>
                <RemoteShareDialog onClose={() => setShowRemote(false)} />
              </Suspense>
            )}

            {/* Host filesystem browser for the web build's pickFolder (inert on desktop). */}
            <WebFolderPicker />

            {/* Main-process system notices (daemon/startup failures) as in-app banners. */}
            <SystemNotices />

            {/* Notification-center transient toasts (bottom-right). */}
            <NotificationToasts />

            <LibrarySidePanel
              visible={showLibraryPanel}
              onClose={() => setShowLibraryPanel(false)}
              cwd={libraryCwd}
            />

            <BottomTerminalPanel
              visible={showBottomTerminal}
              onClose={() => setShowBottomTerminal(false)}
              cwd={agentCwd || appCwd || undefined}
              left={contentLeft}
            />

            {showSpawnDialog && (
              <SpawnAgentDialog
                defaultCwd={
                  spawnDialogCwd || config.agents?.defaultCwd?.trim() || appCwdRef.current
                }
                defaultProvider={config.agents?.defaultProvider}
                defaultTransport={config.claude?.transport}
                defaultWorktree={config.agents?.spawnInWorktree ?? false}
                onSpawn={handleSpawnAgent}
                onCancel={() => {
                  setShowSpawnDialog(false);
                  setSpawnDialogCwd(null);
                }}
              />
            )}

            {showLayouts && (
              <LayoutsDialog
                agentCount={agents.filter((a) => !a.global).length}
                onSaveCurrent={handleSaveLayout}
                onRestore={handleRestoreLayout}
                onClose={() => setShowLayouts(false)}
              />
            )}

            <ChordHint
              path={chordPath}
              prefix={kbPrefix}
              shortcuts={resolvedShortcuts}
              showOptions={kbChordHints}
            />

            {/* Fleet Deck — cross-agent radar overlay. Sits OVER the still-mounted
          per-agent workspaces, so entering/leaving never remounts a pane.
          Never mounts in focus mode (manifest.fleetDeck). */}
            {uiManifest.fleetDeck && viewLevel === 'fleet' && agents.some((a) => !a.global) && (
              <FleetDeck top={navHeight} left={contentLeft} />
            )}

            {/* Triage Inbox — top-level drawer, reachable from any agent. */}
            <InboxDrawer />

            {/* Full-height workflow timeline, opened from a WorkflowRunCard. */}
            <WorkflowOverlay />
          </div>
        </AttentionProvider>
      </PaneMenuProvider>
    </NotificationsProvider>
  );
}

export default App;
