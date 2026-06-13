import { useRef, useCallback, useState, useEffect } from 'react';
import './App.css';
import NavBar from './components/NavBar';
import SideBar, { SIDEBAR_WIDTH } from './components/SideBar';
import PluginInstallDialog from './components/PluginInstallDialog';
import { usePlugins } from './hooks/usePlugins';
import { useUiEventBus } from './hooks/useUiEventBus';
import { REVIEW_REQUEST_FILE_EVENT, openReviewFile, type ReviewFileTarget } from './lib/reviewBus';
import { EDITOR_OPEN_FILE_EVENT } from './lib/editorBus';
import { useUiCommands } from './hooks/useUiCommands';
import type { PluginPane } from './types/plugin';
import SpawnAgentDialog from './components/SpawnAgentDialog';
import RemoteShareDialog from './components/RemoteShareDialog';
import WebFolderPicker from './components/WebFolderPicker';
import ScrollContainer, { ScrollContainerRef } from './components/ScrollContainer';
import ShortcutOverlay from './components/ShortcutOverlay';
import SessionPicker from './components/SessionPicker';
import CommandPalette from './components/CommandPalette';
import LayoutsDialog from './components/LayoutsDialog';
import LibraryHost from './components/LibraryHost';
import LibrarySidePanel from './components/LibrarySidePanel';
import BottomTerminalPanel from './components/BottomTerminalPanel';
import InboxDrawer from './components/InboxDrawer';
import FleetDeck from './components/FleetDeck';
import { AttentionProvider } from './contexts/AttentionContext';
import type { Layout, LayoutAgent } from './types/layout';
import { useLibrary } from './hooks/useLibrary';
import { useLayoutSync, type HydrationResult } from './hooks/useLayoutSync';
import { useAgentManager, GLOBAL_WORKSPACE_ID } from './hooks/useAgentManager';
import type { PaneType, AgentWorkspace, ViewMode, ViewLevel } from './types/pane';
import type { SessionAmbientState, SessionUsage, ClaudeSessionSnapshot } from './types/claudeSession';
import { useKeyboardNav } from './hooks/useKeyboardNav';
import { useIsSmallScreen } from './hooks/useMediaQuery';
import { useConfig } from './hooks/useConfig';
import { useTheme } from './hooks/useTheme';
import { useSessionLifecycle } from './hooks/useSessionLifecycle';
import { usePluginHotkeys } from './hooks/usePluginHotkeys';

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
    return { agents: data.agents, activeAgentId: data.activeAgentId || '', name: data.name || 'Default' };
  }
  if (data && (data.tabs?.length > 0 || data.panes?.length > 0)) {
    // Backward compat: old flat workspace → wrap its tabs into one agent.
    const oldTabs = data.tabs?.length > 0
      ? data.tabs
      : data.panes.map((p: any) => ({ id: `tab-${p.id}`, title: p.title, panes: [p], activePaneId: p.id }));
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

function App() {
  const { config, loaded: configLoaded, save: saveConfig } = useConfig();
  useTheme();

  // Shared-layout hydration gate (tmux-style mirror). Until the hub's layout
  // document is read we don't know whether to adopt a shared layout or run the
  // local session picker, so session restore waits on this:
  //   'pending'  — still reading the hub
  //   'adopted'  — the hub had a layout; we mirrored it and skip the picker
  //   'empty'    — no shared layout yet; run normal session restore (which then
  //                seeds the hub via useLayoutSync's push)
  const [hubHydration, setHubHydration] = useState<HydrationResult>('pending');
  const {
    agents,
    activeAgentId,
    activeAgent,
    spawnAgent,
    spawnSupervisor,
    adoptAgent,
    respawnAgent,
    terminateAgent,
    renameAgent,
    reconcileAgents,
    loadAgentsFromSession,
    openPaneIn,
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
    updateTabCanvas,
    setActivePane,
    hibernatePane,
    wakePane,
    updatePaneUrl,
    updatePaneNotes,
    getActiveTab,
  } = useAgentManager();

  const scrollContainerRef = useRef<ScrollContainerRef>(null);
  const [showHelp, setShowHelp] = useState(false);
  const [renameSignal, setRenameSignal] = useState(0);
  const [chordState, setChordState] = useState<'idle' | 'waiting'>('idle');
  const [showCommandPalette, setShowCommandPalette] = useState(false);
  const [paletteMode, setPaletteMode] = useState<'tab' | 'split'>('tab');
  const [paletteRestrict, setPaletteRestrict] = useState<'library' | undefined>(undefined);
  const [showSpawnDialog, setShowSpawnDialog] = useState(false);
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
  // Layout offsets: panes go full-width when the sidebar is collapsed; the navbar
  // keeps a small left inset so the floating "show sidebar" button has room.
  // On small screens the sidebar overlays the content, so we never reserve space.
  const sidebarOverlay = isSmallScreen;
  const contentLeft = sidebarCollapsed || sidebarOverlay ? 0 : SIDEBAR_WIDTH;
  const navLeft = sidebarCollapsed || sidebarOverlay ? 36 : SIDEBAR_WIDTH;

  // App working directory (used as the default cwd for the spawn dialog + the
  // Library's fallback project root).
  const appCwdRef = useRef<string>('');
  const [appCwd, setAppCwd] = useState('');
  useEffect(() => {
    window.electronAPI.getCwd().then((cwd) => { appCwdRef.current = cwd; setAppCwd(cwd); }).catch(() => {});
  }, []);

  // Session lifecycle (load / save / auto-resume / picker).
  const {
    sessionPhase,
    setSessionPhase,
    sessionList,
    pickerCancellable,
    setPickerCancellable,
    sessionName,
    ptyMapping,
    handlePtyReady,
    handleNewSession,
    handleResumeSession,
    handleDeleteSession,
    saveCurrentSession,
    switchSession,
  } = useSessionLifecycle({
    // Hold session restore until the hub layout has been read. If the hub
    // already has a shared layout we adopt it instead (hubHydration === 'adopted'
    // never unblocks startup); only 'empty' falls through to local restore.
    configLoaded: configLoaded && hubHydration === 'empty',
    autoResume: config.session?.autoResume,
    agents,
    activeAgentId,
    loadAgentsFromSession,
    reconcileAgents,
    appCwdRef,
  });

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
    onHydration: setHubHydration,
  });

  // Library (reusable prompts + skills): global + the active project's items.
  const libraryCwd = activeAgent?.cwd || appCwd || undefined;
  const { items: libraryItems } = useLibrary(libraryCwd);
  // Toggle the right-side Library panel (bound to the 'library-picker' shortcut,
  // default Ctrl+L). Replaces the old restricted-command-palette quick-picker.
  const toggleLibraryPanel = useCallback(() => { setShowLibraryPanel((v) => !v); }, []);

  // Live agent status: sessionId -> ambient state, sourced from claudemon.
  // We also promote the FULL snapshot per session into snapshotBySession — the
  // shared substrate the Triage Inbox and Fleet Deck both project from. (App
  // already re-renders on every status update, so storing the snapshot here is
  // no extra render churn; it just stops throwing the rich payload away.)
  const [statusBySession, setStatusBySession] = useState<Record<string, SessionAmbientState>>({});
  const [usageBySession, setUsageBySession] = useState<Record<string, SessionUsage>>({});
  const [snapshotBySession, setSnapshotBySession] = useState<Record<string, ClaudeSessionSnapshot>>({});
  useEffect(() => {
    let cancelled = false;
    window.electronAPI.getAllClaudeSessions().then((sessions: any[]) => {
      if (cancelled) return;
      const map: Record<string, SessionAmbientState> = {};
      const usage: Record<string, SessionUsage> = {};
      const snaps: Record<string, ClaudeSessionSnapshot> = {};
      for (const s of sessions) {
        map[s.sessionId] = s.ambientState;
        if (s.usage) usage[s.sessionId] = s.usage;
        snaps[s.sessionId] = s;
      }
      setStatusBySession(map);
      setUsageBySession(usage);
      setSnapshotBySession(snaps);
    }).catch(() => {});
    const unsub = window.electronAPI.onClaudeSessionUpdate((sessionId: string, snapshot: any) => {
      setStatusBySession((prev) => ({ ...prev, [sessionId]: snapshot.ambientState }));
      if (snapshot.usage) {
        setUsageBySession((prev) => ({ ...prev, [sessionId]: snapshot.usage }));
      }
      setSnapshotBySession((prev) => ({ ...prev, [sessionId]: snapshot }));
    });
    return () => { cancelled = true; unsub(); };
  }, []);

  // Auto-adopt any live daemon session that has no AgentWorkspace yet (e.g. one
  // spawned externally via the MCP facade or by another agent). Gated on the
  // session-restore phase so we don't create duplicates for sessions that are
  // about to be loaded from the saved session file.
  const adoptedRef = useRef<Set<string>>(new Set());
  useEffect(() => {
    // Wait until the initial session-restore has completed (phase leaves 'loading').
    if (sessionPhase === 'loading') return;
    for (const [sessionId, snapshot] of Object.entries(snapshotBySession)) {
      // Skip ended sessions and already-adopted ones.
      if (snapshot.status === 'ended') continue;
      if (adoptedRef.current.has(sessionId)) continue;
      // Skip if some agent already owns this session.
      if (agents.some((a) => a.sessionId === sessionId)) continue;
      // Mark as adopted before calling to avoid redundant calls from re-renders.
      adoptedRef.current.add(sessionId);
      adoptAgent({ sessionId, cwd: snapshot.cwd, name: snapshot.label, parentSessionId: snapshot.parentSessionId });
    }
  }, [snapshotBySession, agents, adoptAgent, sessionPhase]);

  // Mission Control surfaces: the Triage Inbox (a top-level drawer) and the
  // Fleet Deck (a cross-agent radar, a global altitude orthogonal to viewMode).
  const [inboxOpen, setInboxOpen] = useState(false);
  const openInbox = useCallback(() => setInboxOpen(true), []);
  const closeInbox = useCallback(() => setInboxOpen(false), []);
  const toggleInbox = useCallback(() => setInboxOpen((v) => !v), []);

  // Altitude: 'piloting' (inside one agent) vs 'fleet' (the cross-agent deck).
  const viewLevel: ViewLevel = config.panes?.viewLevel === 'fleet' ? 'fleet' : 'piloting';
  const setViewLevel = useCallback((next: ViewLevel) => {
    saveConfig({ panes: { ...config.panes, viewLevel: next } });
  }, [config.panes, saveConfig]);
  const toggleFleet = useCallback(() => {
    setViewLevel(viewLevel === 'fleet' ? 'piloting' : 'fleet');
  }, [viewLevel, setViewLevel]);

  const handleUrlChange = useCallback((tabId: string, paneId: string, url: string) => {
    updatePaneUrl(tabId, paneId, url);
  }, [updatePaneUrl]);

  const handleNotesChange = useCallback((tabId: string, paneId: string, notes: string) => {
    updatePaneNotes(tabId, paneId, notes);
  }, [updatePaneNotes]);

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

  const addTabWithConfig = useCallback((type: PaneType, title?: string, shell?: string, url?: string, appMode?: boolean, cwd?: string, profileId?: string, resumeSessionId?: string, attachSessionId?: string, initialCommand?: string, filePath?: string) => {
    return addTab(type, title, insertPosition, shell, url, appMode, cwd, profileId, resumeSessionId, attachSessionId, initialCommand, filePath);
  }, [addTab, insertPosition]);

  // Open a file in an Editor pane: pick a file (if none given), then open a
  // pane bound to it. The pane's engine (CodeMirror vs terminal) is decided at
  // render time from config.editor.engine.
  const openFileInEditor = useCallback(async (filePath?: string) => {
    let target = filePath;
    if (!target) {
      const picked = await window.electronAPI.pickFiles(activeAgent?.cwd);
      target = picked?.[0];
    }
    if (!target) return;
    const name = target.split(/[\\/]/).pop() || 'Editor';
    const dir = target.replace(/[\\/][^\\/]*$/, '') || activeAgent?.cwd;
    const newId = addTabWithConfig('editor', name, undefined, undefined, undefined, dir, undefined, undefined, undefined, undefined, target);
    requestAnimationFrame(() => scrollToTab(newId));
  }, [activeAgent, addTabWithConfig, scrollToTab]);

  // Open-in-editor requests (e.g. right-click in the Review pane's file tree).
  useEffect(() => {
    const handler = (e: Event) => {
      const target = (e as CustomEvent).detail as { path?: string } | undefined;
      if (target?.path) void openFileInEditor(target.path);
    };
    window.addEventListener(EDITOR_OPEN_FILE_EVENT, handler);
    return () => window.removeEventListener(EDITOR_OPEN_FILE_EVENT, handler);
  }, [openFileInEditor]);

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

  const kbMode = config.keybindings?.mode ?? 'default';
  const kbLeader = config.keybindings?.leader ?? 'ctrl';

  const activeTab = getActiveTab();

  // --- Agent handlers (defined before useKeyboardNav so it can bind them) ---
  const handleSelectAgent = useCallback((id: string) => {
    setActiveAgentId(id);
    const agent = agents.find((a) => a.id === id);
    if (agent && !agent.sessionId) respawnAgent(id);
  }, [agents, setActiveAgentId, respawnAgent]);

  // Record a directory at the front of the Overview's recent list (deduped, capped).
  const recordRecentDir = useCallback((cwd?: string) => {
    if (!cwd) return;
    const cur = config.directories?.recent ?? [];
    if (cur[0] === cwd) return;
    const recent = [cwd, ...cur.filter((d) => d !== cwd)].slice(0, 8);
    saveConfig({ directories: { recent, favourites: config.directories?.favourites ?? [] } });
  }, [config.directories, saveConfig]);

  const handleSpawnAgent = useCallback((opts: { cwd: string; name?: string; profileId?: string; model?: string; skipPermissions?: boolean; resumeSessionId?: string }) => {
    setShowSpawnDialog(false);
    // Remember the picked model + skip-permissions choice so they stick next time.
    window.electronAPI.saveConfig({
      claude: { defaultModel: opts.model ?? '', skipPermissionsDefault: opts.skipPermissions ?? false },
    }).catch(() => {});
    recordRecentDir(opts.cwd);
    void spawnAgent(opts);
  }, [spawnAgent, recordRecentDir]);

  // --- Layout templates ---

  // Snapshot the current (non-global) agents as a reusable layout: directories
  // + their pane arrangement, stripped of live session ids.
  const captureLayout = useCallback((): LayoutAgent[] => {
    return agents.filter((a) => !a.global).map((a) => ({
      name: a.name,
      cwd: a.cwd,
      model: a.model,
      tabs: a.tabs.map((t) => ({
        title: t.title,
        panes: t.panes
          .filter((p) => p.type !== 'settings')
          .map((p) => ({ type: p.type, title: p.title, url: p.url, shell: p.shell, cwd: p.cwd })),
      })),
    }));
  }, [agents]);

  const handleSaveLayout = useCallback((name: string) => {
    window.electronAPI.layoutsSave({ name, agents: captureLayout() }).catch((err: any) => {
      console.error('[Layout] save failed:', err);
    });
  }, [captureLayout]);

  // Restore a layout: spawn a fresh agent per saved directory, then reopen its
  // non-Claude panes (spawnAgent already creates the primary Claude tab).
  const handleRestoreLayout = useCallback(async (layout: Layout) => {
    for (const la of layout.agents) {
      recordRecentDir(la.cwd);
      const agentId = await spawnAgent({ cwd: la.cwd, name: la.name, model: la.model });
      for (const tab of la.tabs) {
        for (const pane of tab.panes) {
          if (pane.type === 'claude') continue; // primary Claude tab already created
          openPaneIn(agentId, pane.type as PaneType, pane.title, pane.url, pane.cwd ?? la.cwd);
        }
      }
    }
  }, [spawnAgent, openPaneIn, recordRecentDir]);

  const openAnalytics = useCallback(() => {
    setShowCommandPalette(false);
    openPaneIn(GLOBAL_WORKSPACE_ID, 'analytics', 'Analytics');
  }, [openPaneIn]);

  /** Open the Ask pane in the global Overview workspace (command-palette entry
   *  "Ask the fleet"). Reuses an existing Ask tab rather than opening a duplicate. */
  const openAskPane = useCallback(() => {
    setShowCommandPalette(false);
    openPaneIn(GLOBAL_WORKSPACE_ID, 'ask', 'Ask');
  }, [openPaneIn]);

  /** Jump to a specific agent by id — passed down to the Ask pane. */
  const handleJumpToAgent = useCallback((agentId: string) => {
    handleSelectAgent(agentId);
  }, [handleSelectAgent]);

  const goToAgent = useCallback((delta: number) => {
    if (agents.length === 0) return;
    const idx = agents.findIndex((a) => a.id === activeAgentId);
    const base = idx < 0 ? 0 : idx;
    const next = (base + delta + agents.length) % agents.length;
    handleSelectAgent(agents[next].id);
  }, [agents, activeAgentId, handleSelectAgent]);

  const handlePrevAgent = useCallback(() => goToAgent(-1), [goToAgent]);
  const handleNextAgent = useCallback(() => goToAgent(1), [goToAgent]);
  const handleSpawnAgentShortcut = useCallback(() => setShowSpawnDialog(true), []);

  // Jump to the next agent that's blocked on you (approval / input), cycling
  // from the current one. No-op if nothing needs you.
  const goToNextAttention = useCallback(() => {
    if (agents.length === 0) return;
    const needsMe = (a: AgentWorkspace) => {
      const s = a.sessionId ? statusBySession[a.sessionId] : undefined;
      return s === 'waiting_approval' || s === 'waiting_input';
    };
    const startIdx = agents.findIndex((a) => a.id === activeAgentId);
    const base = startIdx < 0 ? 0 : startIdx;
    for (let off = 1; off <= agents.length; off++) {
      const cand = agents[(base + off) % agents.length];
      if (needsMe(cand)) { handleSelectAgent(cand.id); return; }
    }
  }, [agents, activeAgentId, statusBySession, handleSelectAgent]);

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
    keybindingsMode: kbMode,
    leaderKey: kbLeader,
    onChordStateChange: setChordState,
    onOpenSettings: openSettings,
    onSaveSession: saveCurrentSession,
    onOpenCommandPalette: useCallback(() => { setPaletteRestrict(undefined); setPaletteMode('tab'); setShowCommandPalette(true); }, []),
    onOpenSplitPalette: useCallback(() => { setPaletteRestrict(undefined); setPaletteMode('split'); setShowCommandPalette(true); }, []),
    onOpenFile: openFileInEditor,
    onPrevAgent: handlePrevAgent,
    onNextAgent: handleNextAgent,
    onNextAttention: goToNextAttention,
    onSpawnAgent: handleSpawnAgentShortcut,
    onToggleTerminal: useCallback(() => setShowBottomTerminal((v) => !v), []),
    onToggleSidebar: useCallback(() => setSidebarCollapsed((v) => !v), []),
    onToggleInbox: toggleInbox,
    onToggleFleet: toggleFleet,
    shortcuts: config.keybindings?.shortcuts ?? {},
  });

  // Escape exits the Fleet Deck back to piloting (when the inbox isn't capturing).
  useEffect(() => {
    if (viewLevel !== 'fleet') return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && !inboxOpen) { e.preventDefault(); setViewLevel('piloting'); }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [viewLevel, inboxOpen, setViewLevel]);

  const handleTabClick = useCallback((id: string) => {
    setActiveTabId(id);
    scrollToTab(id);
  }, [setActiveTabId, scrollToTab]);

  const handleTabFocus = useCallback((id: string) => {
    setActiveTabId(id);
  }, [setActiveTabId]);

  const handlePaneClose = useCallback((tabId: string, paneId: string) => {
    removePane(tabId, paneId);
  }, [removePane]);

  const handlePaneFocus = useCallback((tabId: string, paneId: string) => {
    setActiveTabId(tabId);
    setActivePane(tabId, paneId);
  }, [setActiveTabId, setActivePane]);

  const handleAddTab = useCallback((type: PaneType, shell?: string, label?: string, cwd?: string, profileId?: string, resumeSessionId?: string, attachSessionId?: string) => {
    // If opening a Claude session that already has a tab, navigate to it.
    const sessionId = resumeSessionId || attachSessionId;
    if (type === 'claude' && sessionId) {
      for (const tab of tabs) {
        const match = tab.panes.find((p) =>
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
    // New panes inherit the active agent's working directory.
    const resolvedCwd = cwd || activeAgent?.cwd;
    const newId = addTabWithConfig(type, label, shell, undefined, undefined, resolvedCwd, profileId, resumeSessionId, attachSessionId);
    requestAnimationFrame(() => scrollToTab(newId));
  }, [tabs, ptyMapping, activeAgent, addTabWithConfig, setActiveTabId, setActivePane, scrollToTab]);

  const handleSplitPane = useCallback((type: PaneType, shell?: string, label?: string, cwd?: string) => {
    if (!activeTabId) return;
    const resolvedCwd = cwd || activeAgent?.cwd;
    splitTab(activeTabId, type, label, shell, undefined, undefined, resolvedCwd);
  }, [activeTabId, activeAgent, splitTab]);

  // Open a changed file in the Review pane (from the Claude pane's file list).
  // Focus an existing Review pane in the active agent if there is one, else
  // open a new one; then hand the target file to the (now-mounted) pane. The
  // double rAF lets a freshly-created pane mount + attach its listener first.
  useEffect(() => {
    const handler = (e: Event) => {
      const target = (e as CustomEvent).detail as ReviewFileTarget | undefined;
      if (!target?.path) return;
      const cwd = target.cwd || activeAgent?.cwd;
      let existing: { tabId: string; paneId: string } | null = null;
      for (const tab of tabs) {
        const pane = tab.panes.find((p) => p.type === 'review');
        if (pane) { existing = { tabId: tab.id, paneId: pane.id }; break; }
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
  }, [tabs, activeAgent, setActiveTabId, setActivePane, scrollToTab, handleAddTab]);

  const handleLaunchApp = useCallback((app: { name: string; url: string }) => {
    const newId = addTab('browser', app.name, insertPosition, undefined, app.url, true);
    requestAnimationFrame(() => scrollToTab(newId));
  }, [addTab, insertPosition, scrollToTab]);

  // Publish every UI action (pane open/close, focus changes) onto the hub bus
  // so plugins/MCP can react to what's happening in the app.
  useUiEventBus(agents, activeAgentId);

  // --- Plugins (contributed panes + hotkeys from the hub) ---
  const { panes: pluginPanes, hotkeys: pluginHotkeys } = usePlugins();
  const [showInstallPlugin, setShowInstallPlugin] = useState(false);

  const handleOpenPlugin = useCallback((pane: PluginPane) => {
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

    if (!target) {
      openPaneIn(GLOBAL_WORKSPACE_ID, 'plugin', pane.title, pane.url);
      return;
    }
    // Hand agent context to the plugin's webview.
    const sep = pane.url.includes('?') ? '&' : '?';
    const params = new URLSearchParams();
    if (target.sessionId) params.set('sessionId', target.sessionId);
    if (target.cwd) params.set('cwd', target.cwd);
    const url = params.toString() ? `${pane.url}${sep}${params.toString()}` : pane.url;
    openPaneIn(target.id, 'plugin', pane.title, url);
  }, [openPaneIn, activeAgent, agents]);

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
      if (cwd) { recordRecentDir(cwd); void spawnAgent({ cwd, name: opts.name, model: opts.model }); }
    },
    openPane: (paneType, opts) => handleAddTab(paneType as PaneType, undefined, undefined, opts?.cwd),
    openPlugin: (type) => {
      const pane = pluginPanes.find((p) => p.type === type);
      if (pane) handleOpenPlugin(pane);
    },
    closePane: (paneId) => {
      for (const a of agents) {
        for (const t of a.tabs) {
          if (t.panes.some((p) => p.id === paneId)) { removePane(t.id, paneId); return; }
        }
      }
    },
    openAskPane,
  });

  // --- Per-directory script buttons ---
  const agentCwd = activeAgent?.cwd ?? '';
  const dirScripts = agentCwd ? (config.scripts?.[scriptKey(agentCwd)] ?? []) : [];

  // Run a script in a fresh terminal tab rooted at the agent's workspace.
  const handleRunScript = useCallback((name: string, command: string) => {
    if (!agentCwd) return;
    const newId = addTabWithConfig('terminal', name, undefined, undefined, undefined, agentCwd, undefined, undefined, undefined, command);
    requestAnimationFrame(() => scrollToTab(newId));
  }, [agentCwd, addTabWithConfig, scrollToTab]);

  // Persist this directory's script list to config.
  const handleSaveScripts = useCallback((entries: { name: string; command: string }[]) => {
    if (!agentCwd) return;
    saveConfig({ scripts: { ...(config.scripts ?? {}), [scriptKey(agentCwd)]: entries } });
  }, [agentCwd, config.scripts, saveConfig]);

  // --- Render ---
  // Phones get a taller bar so the (fattened) touch targets fit; this height
  // also drives the content top-offset below, so the two stay in sync.
  const navHeight = Math.max(config.ui.navBarHeight || 34, isSmallScreen ? 44 : 32);

  const rawViewMode = config.panes?.viewMode as string | undefined;
  const viewMode: ViewMode =
    rawViewMode === 'spatial' ? 'spatial'
    // 'timeline' is the old name for 'stacked' — keep reading old configs.
    : (rawViewMode === 'stacked' || rawViewMode === 'timeline') ? 'stacked'
    : 'tabs';
  const toggleViewMode = useCallback(() => {
    // Cycle: tabs → spatial → stacked → tabs
    const order: ViewMode[] = ['tabs', 'spatial', 'stacked'];
    const next = order[(order.indexOf(viewMode) + 1) % order.length];
    saveConfig({ panes: { ...config.panes, viewMode: next } });
  }, [viewMode, config.panes, saveConfig]);

  const handleNavBarRename = useCallback(
    (tabId: string) => { setActiveTabId(tabId); setRenameSignal((s) => s + 1); },
    [setActiveTabId],
  );
  const handleNavBarSplit = useCallback(
    // New split panes inherit the active agent's working directory.
    (tabId: string, type: PaneType) => { splitTab(tabId, type, undefined, undefined, undefined, undefined, activeAgent?.cwd); },
    [splitTab, activeAgent],
  );

  return (
    <AttentionProvider
      agents={agents}
      activeAgentId={activeAgentId}
      snapshotBySession={snapshotBySession}
      inboxOpen={inboxOpen}
      openInbox={openInbox}
      closeInbox={closeInbox}
      viewLevel={viewLevel}
      setViewLevel={setViewLevel}
      onOpenAgent={handleSelectAgent}
    >
    <div className="app-root">
      {!sidebarCollapsed && sidebarOverlay && (
        <div
          onClick={() => setSidebarCollapsed(true)}
          style={{
            position: 'fixed', inset: 0, zIndex: 90,
            background: 'rgba(0,0,0,0.45)',
            // @ts-ignore — stay clickable over the draggable navbar region
            WebkitAppRegion: 'no-drag',
          }}
        />
      )}
      {!sidebarCollapsed && (
        <SideBar
          agents={agents}
          activeAgentId={activeAgentId}
          statusBySession={statusBySession}
          usageBySession={usageBySession}
          onSelectAgent={(id) => { handleSelectAgent(id); if (sidebarOverlay) setSidebarCollapsed(true); }}
          onSpawnAgent={() => setShowSpawnDialog(true)}
          onTerminateAgent={terminateAgent}
          onRenameAgent={renameAgent}
          onJumpToAttention={goToNextAttention}
          onOpenInbox={openInbox}
          onToggleFleet={toggleFleet}
          viewLevel={viewLevel}
          onOpenRemote={() => setShowRemote(true)}
          onToggleCollapse={() => setSidebarCollapsed(true)}
        />
      )}
      {sidebarCollapsed && (
        <button
          onClick={() => setSidebarCollapsed(false)}
          title="Show sidebar (Ctrl+B)"
          style={{
            position: 'fixed', zIndex: 200,
            // Clear the notch/status bar on phones; keep it tight on desktop.
            top: isSmallScreen ? 'calc(env(safe-area-inset-top) + 6px)' : 6,
            left: isSmallScreen ? 'calc(env(safe-area-inset-left) + 6px)' : 6,
            // Larger fingertip target on phones (Apple HIG floor is ~44px).
            width: isSmallScreen ? 38 : 26, height: isSmallScreen ? 38 : 26,
            display: 'flex', alignItems: 'center', justifyContent: 'center',
            border: '1px solid var(--wks-glass-border)', borderRadius: 'var(--wks-radius-md)',
            background: 'var(--wks-bg-surface)', color: 'var(--wks-text-secondary)',
            cursor: 'pointer', fontSize: isSmallScreen ? '1.1rem' : '0.95rem', lineHeight: 1,
            // @ts-ignore — keep it clickable over the draggable navbar region
            WebkitAppRegion: 'no-drag',
          }}
        >»</button>
      )}

      <NavBar
        tabs={tabs}
        activeTabId={activeTabId}
        onTabClick={handleTabClick}
        onAddTab={handleAddTab}
        onCloseTab={removeTab}
        onRenameTab={handleNavBarRename}
        onSplitTab={handleNavBarSplit}
        onMoveTab={moveTab}
        viewMode={viewMode}
        onToggleViewMode={toggleViewMode}
        leftOffset={navLeft}
        cwd={agentCwd || undefined}
        scripts={dirScripts}
        onRunScript={handleRunScript}
        onSaveScripts={handleSaveScripts}
      />

      <div className="app-content" style={{
        // Small gap between the tab bar and the top of the panes.
        marginTop: `${navHeight + 8}px`,
        marginLeft: `${contentLeft}px`,
      }}>
        {agents.length > 0 ? (
          // Keep every agent's workspace mounted and just toggle visibility, so
          // switching agents never unmounts a Claude pane (which would detach
          // its viewer and clear the terminal). Only the active agent's
          // container is shown and wired to the scroll ref.
          agents.map((agent) => {
            const isActiveAgent = agent.id === activeAgentId;
            return (
              <div
                key={agent.id}
                style={{ display: isActiveAgent ? 'block' : 'none', height: '100%' }}
              >
                <ScrollContainer
                  ref={isActiveAgent ? scrollContainerRef : undefined}
                  agentActive={isActiveAgent}
                  tabs={agent.tabs}
                  activeTabId={agent.activeTabId}
                  onTabFocus={handleTabFocus}
                  onPaneClose={handlePaneClose}
                  onPaneFocus={handlePaneFocus}
                  onTabRename={renameTab}
                  onTabMove={moveTab}
                  viewMode={viewMode}
                  onTabCanvasChange={updateTabCanvas}
                  onPtyReady={handlePtyReady}
                  onUrlChange={handleUrlChange}
                  onNotesChange={handleNotesChange}
                  onNavigateToTab={handleTabClick}
                  onAddTab={handleAddTab}
                  ptyMapping={ptyMapping}
                  renameSignal={renameSignal}
                  workspaceAgents={agents.filter((a) => !a.global).map((a) => ({ sessionId: a.sessionId }))}
                  appCwd={appCwd}
                  allAgents={agents}
                  spawnSupervisor={spawnSupervisor}
                  onJumpToAgent={handleJumpToAgent}
                />
              </div>
            );
          })
        ) : (
          <div style={{
            height: '100%', display: 'flex', flexDirection: 'column',
            alignItems: 'center', justifyContent: 'center', gap: 12,
            color: 'var(--wks-text-muted)', textAlign: 'center', padding: 24,
          }}>
            <div style={{ fontSize: '0.95rem', fontWeight: 600, color: 'var(--wks-text-secondary)' }}>
              No agent selected
            </div>
            <div style={{ fontSize: '0.8rem', maxWidth: 360, lineHeight: 1.5 }}>
              Spawn an agent to start a Claude Code session. It stays running until you terminate it,
              and its tabs &amp; panes are remembered.
            </div>
            <button
              onClick={() => setShowSpawnDialog(true)}
              style={{
                marginTop: 4, fontSize: '0.8rem', fontFamily: 'inherit', fontWeight: 600,
                cursor: 'pointer', background: 'var(--wks-accent)', color: 'var(--wks-text-on-accent, #fff)',
                border: 'none', borderRadius: 4, padding: '8px 16px',
              }}
            >
              + Spawn agent
            </button>
          </div>
        )}
      </div>

      <ShortcutOverlay
        visible={showHelp}
        onClose={closeHelp}
        mode={kbMode}
        leader={kbLeader}
        shortcuts={config.keybindings?.shortcuts}
      />

      <CommandPalette
        visible={showCommandPalette}
        apps={config.apps ?? []}
        agentCwd={agentCwd || undefined}
        mode={paletteMode}
        restrictTo={paletteRestrict}
        libraryItems={libraryItems}
        onClose={useCallback(() => { setShowCommandPalette(false); setPaletteRestrict(undefined); }, [])}
        onLaunchApp={handleLaunchApp}
        onAddTab={handleAddTab}
        onSplitPane={handleSplitPane}
        pluginPanes={pluginPanes}
        onOpenPlugin={handleOpenPlugin}
        onInstallPlugin={() => { setShowCommandPalette(false); setShowInstallPlugin(true); }}
        onManagePlugins={() => { setShowCommandPalette(false); openPaneIn(GLOBAL_WORKSPACE_ID, 'plugins', 'Plugins'); }}
        onOpenLibrary={() => {
          setShowCommandPalette(false);
          // Open in the active agent's workspace (with its project cwd) so the
          // pane shows that project's library + .claude skills; fall back to
          // the global Overview when no agent is focused.
          if (activeAgent && !activeAgent.global) {
            openPaneIn(activeAgent.id, 'library', 'Library', undefined, activeAgent.cwd);
          } else {
            openPaneIn(GLOBAL_WORKSPACE_ID, 'library', 'Library');
          }
        }}
        onSwitchSession={() => { setShowCommandPalette(false); switchSession(); }}
        onOpenAnalytics={openAnalytics}
        onOpenLayouts={() => { setShowCommandPalette(false); setShowLayouts(true); }}
        onOpenRemote={() => { setShowCommandPalette(false); setShowRemote(true); }}
        onOpenAskPane={openAskPane}
        onOpenFile={() => { setShowCommandPalette(false); openFileInEditor(); }}
      />

      <LibraryHost
        activeAgent={activeAgent}
        appCwd={appCwd}
        spawnAgent={(opts) => { void spawnAgent(opts); }}
        recordRecentDir={recordRecentDir}
      />

      {showInstallPlugin && (
        <PluginInstallDialog onClose={() => setShowInstallPlugin(false)} />
      )}

      {showRemote && (
        <RemoteShareDialog onClose={() => setShowRemote(false)} />
      )}

      {/* Host filesystem browser for the web build's pickFolder (inert on desktop). */}
      <WebFolderPicker />

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
          defaultCwd={appCwdRef.current}
          onSpawn={handleSpawnAgent}
          onCancel={() => setShowSpawnDialog(false)}
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

      {sessionPhase === 'picker' && (
        <SessionPicker
          sessions={sessionList}
          onNewSession={handleNewSession}
          onResumeSession={handleResumeSession}
          onDeleteSession={handleDeleteSession}
          onCancel={pickerCancellable ? () => { setPickerCancellable(false); setSessionPhase('active'); } : undefined}
        />
      )}

      {chordState === 'waiting' && (
        <div style={{
          position: 'fixed',
          bottom: '16px',
          right: '16px',
          backgroundColor: 'var(--wks-accent)',
          color: 'var(--wks-text-on-accent)',
          fontSize: '0.65rem',
          fontWeight: 700,
          fontFamily: 'monospace',
          padding: '2px 8px',
          borderRadius: '3px',
          zIndex: 200,
        }}>
          -- CMD --
        </div>
      )}

      {/* Fleet Deck — cross-agent radar overlay. Sits OVER the still-mounted
          per-agent workspaces, so entering/leaving never remounts a pane. */}
      {viewLevel === 'fleet' && agents.some((a) => !a.global) && (
        <FleetDeck top={navHeight + 8} left={contentLeft} />
      )}

      {/* Triage Inbox — top-level drawer, reachable from any agent. */}
      <InboxDrawer />
    </div>
    </AttentionProvider>
  );
}

export default App;
