import { useRef, useCallback, useState, useEffect } from 'react';
import './App.css';
import NavBar from './components/NavBar';
import SideBar, { SIDEBAR_WIDTH } from './components/SideBar';
import PluginInstallDialog from './components/PluginInstallDialog';
import { usePlugins } from './hooks/usePlugins';
import { useUiEventBus } from './hooks/useUiEventBus';
import { useUiCommands } from './hooks/useUiCommands';
import type { PluginPane } from './types/plugin';
import SpawnAgentDialog from './components/SpawnAgentDialog';
import RemoteShareDialog from './components/RemoteShareDialog';
import ScrollContainer, { ScrollContainerRef } from './components/ScrollContainer';
import ShortcutOverlay from './components/ShortcutOverlay';
import SessionPicker from './components/SessionPicker';
import CommandPalette from './components/CommandPalette';
import LayoutsDialog from './components/LayoutsDialog';
import LibraryHost from './components/LibraryHost';
import type { Layout, LayoutAgent } from './types/layout';
import { useLibrary } from './hooks/useLibrary';
import { useAgentManager, GLOBAL_WORKSPACE_ID } from './hooks/useAgentManager';
import type { PaneType, AgentWorkspace, ViewMode } from './types/pane';
import type { SessionAmbientState, SessionUsage } from './types/claudeSession';
import { useKeyboardNav } from './hooks/useKeyboardNav';
import { useConfig } from './hooks/useConfig';
import { useTheme } from './hooks/useTheme';

/** Normalize a workspace dir into a stable config key (slashes + no trailing /). */
function scriptKey(cwd: string): string {
  return cwd.replace(/\\/g, '/').replace(/\/+$/, '');
}

function App() {
  const { config, loaded: configLoaded, save: saveConfig } = useConfig();
  useTheme();
  const {
    agents,
    activeAgentId,
    activeAgent,
    spawnAgent,
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

  // App working directory (used as the default cwd for the spawn dialog + the
  // Library's fallback project root).
  const appCwdRef = useRef<string>('');
  const [appCwd, setAppCwd] = useState('');
  useEffect(() => {
    window.electronAPI.getCwd().then((cwd) => { appCwdRef.current = cwd; setAppCwd(cwd); }).catch(() => {});
  }, []);

  // Library (reusable prompts + skills): global + the active project's items.
  const libraryCwd = activeAgent?.cwd || appCwd || undefined;
  const { items: libraryItems } = useLibrary(libraryCwd);
  const openLibraryPicker = useCallback(() => { setPaletteRestrict('library'); setPaletteMode('tab'); setShowCommandPalette(true); }, []);

  // Live agent status: sessionId -> ambient state, sourced from claudemon.
  const [statusBySession, setStatusBySession] = useState<Record<string, SessionAmbientState>>({});
  const [usageBySession, setUsageBySession] = useState<Record<string, SessionUsage>>({});
  useEffect(() => {
    let cancelled = false;
    window.electronAPI.getAllClaudeSessions().then((sessions: any[]) => {
      if (cancelled) return;
      const map: Record<string, SessionAmbientState> = {};
      const usage: Record<string, SessionUsage> = {};
      for (const s of sessions) {
        map[s.sessionId] = s.ambientState;
        if (s.usage) usage[s.sessionId] = s.usage;
      }
      setStatusBySession(map);
      setUsageBySession(usage);
    }).catch(() => {});
    const unsub = window.electronAPI.onClaudeSessionUpdate((sessionId: string, snapshot: any) => {
      setStatusBySession((prev) => ({ ...prev, [sessionId]: snapshot.ambientState }));
      if (snapshot.usage) {
        setUsageBySession((prev) => ({ ...prev, [sessionId]: snapshot.usage }));
      }
    });
    return () => { cancelled = true; unsub(); };
  }, []);

  // Session state
  const [sessionPhase, setSessionPhase] = useState<'loading' | 'picker' | 'active'>('loading');
  const [sessionList, setSessionList] = useState<any[]>([]);
  // True only when the picker is reopened mid-session (so it can be dismissed).
  const [pickerCancellable, setPickerCancellable] = useState(false);
  const [sessionName, setSessionName] = useState('Default');

  // PTY mapping: paneId -> ptySessionId. For Claude panes, ptySessionId is the
  // Claude session id; used to resolve "which pane shows this session".
  const [ptyMapping, setPtyMapping] = useState<Record<string, string>>({});
  const lastSaveHashRef = useRef<string>('');

  const handlePtyReady = useCallback((paneId: string, ptySessionId: string) => {
    setPtyMapping((prev) => (prev[paneId] === ptySessionId ? prev : { ...prev, [paneId]: ptySessionId }));
  }, []);

  const handleUrlChange = useCallback((tabId: string, paneId: string, url: string) => {
    updatePaneUrl(tabId, paneId, url);
  }, [updatePaneUrl]);

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

  // Reconcile saved agents against the daemon's live sessions — mark any whose
  // session no longer exists as stopped (so the sidebar offers a respawn).
  const reconcileWithDaemon = useCallback(() => {
    window.electronAPI.getAllClaudeSessions().then((sessions: any[]) => {
      reconcileAgents(new Set(sessions.map((s) => s.sessionId)));
    }).catch(() => {});
  }, [reconcileAgents]);

  // --- Session lifecycle ---
  const handleNewSession = useCallback(() => {
    loadAgentsFromSession([], '');
    setSessionName('Default');
    setPtyMapping({});
    setPickerCancellable(false);
    setSessionPhase('active');
  }, [loadAgentsFromSession]);

  const handleResumeSession = useCallback((filename: string) => {
    setPickerCancellable(false);
    window.electronAPI.loadSession(filename).then((data: any) => {
      if (data && Array.isArray(data.agents)) {
        loadAgentsFromSession(data.agents, data.activeAgentId);
        setSessionName(data.name || 'Default');
      } else if (data && (data.tabs?.length > 0 || data.panes?.length > 0)) {
        // Backward compat: old flat workspace → wrap its tabs into one agent.
        const oldTabs = data.tabs?.length > 0
          ? data.tabs
          : data.panes.map((p: any) => ({ id: `tab-${p.id}`, title: p.title, panes: [p], activePaneId: p.id }));
        const migrated: AgentWorkspace = {
          id: `agent-migrated-${Date.now()}`,
          name: data.name || 'Imported',
          cwd: appCwdRef.current,
          tabs: oldTabs,
          activeTabId: data.activeTabId || oldTabs[0]?.id || '',
        };
        loadAgentsFromSession([migrated], migrated.id);
        setSessionName(data.name || 'Default');
      } else {
        loadAgentsFromSession([], '');
      }
      setPtyMapping({});
      setSessionPhase('active');
      reconcileWithDaemon();
    }).catch(() => {
      loadAgentsFromSession([], '');
      setSessionPhase('active');
    });
  }, [loadAgentsFromSession, reconcileWithDaemon]);

  const handleDeleteSession = useCallback((filename: string) => {
    window.electronAPI.deleteSession(filename).then(() => {
      setSessionList((prev) => prev.filter((s) => s.filename !== filename));
    });
  }, []);

  const saveCurrentSession = useCallback((force?: boolean) => {
    if (sessionPhase !== 'active') return;
    const payload = {
      name: sessionName,
      activeAgentId,
      agents: agents.map((a) => ({
        ...a,
        tabs: a.tabs.map((t) => ({ ...t, panes: t.panes.map((p) => ({ ...p })) })),
      })),
      ptyMapping: { ...ptyMapping },
    };
    const hash = JSON.stringify({
      n: payload.name,
      a: payload.activeAgentId,
      g: payload.agents.map((ag) => ag.id + ag.name + (ag.sessionId || '') + ag.activeTabId
        + ag.tabs.map((t) => t.id + t.title + t.panes.map((p) => p.id + p.type + (p.url || '')).join()).join()),
    });
    if (!force && hash === lastSaveHashRef.current) return;
    lastSaveHashRef.current = hash;
    window.electronAPI.saveSession(payload).catch((err: any) => {
      console.error('[Session] save failed:', err);
    });
  }, [agents, activeAgentId, sessionName, sessionPhase, ptyMapping]);

  useEffect(() => {
    if (sessionPhase !== 'active') return;
    const interval = setInterval(saveCurrentSession, 30000);
    return () => clearInterval(interval);
  }, [sessionPhase, saveCurrentSession]);

  useEffect(() => {
    const handler = () => saveCurrentSession();
    window.addEventListener('beforeunload', handler);
    return () => window.removeEventListener('beforeunload', handler);
  }, [saveCurrentSession]);

  useEffect(() => {
    const unsub = window.electronAPI.onBeforeQuit(() => saveCurrentSession(true));
    return unsub;
  }, [saveCurrentSession]);

  // Decide what to show on launch once config is loaded (so a user's saved
  // autoResume preference is respected, not the in-memory default). With
  // autoResume on we restore the most recent session straight away; otherwise
  // we fall back to the picker. Runs exactly once.
  const startupDoneRef = useRef(false);
  useEffect(() => {
    if (!configLoaded || startupDoneRef.current) return;
    startupDoneRef.current = true;
    const autoResume = config.session?.autoResume ?? true;
    window.electronAPI.listSessions().then((sessions) => {
      if (sessions.length === 0) {
        setSessionPhase('active');
        return;
      }
      setSessionList(sessions);
      if (autoResume) {
        handleResumeSession(sessions[0].filename); // most recent (list is sorted desc)
      } else {
        setSessionPhase('picker');
      }
    }).catch(() => setSessionPhase('active'));
  }, [configLoaded, config.session?.autoResume, handleResumeSession]);

  // Re-open the picker mid-session (Command palette → "Switch session"). Saves
  // the current layout first so nothing is lost when switching, and marks the
  // picker dismissable so Escape/Cancel returns to the running app.
  const switchSession = useCallback(() => {
    saveCurrentSession(true);
    setPickerCancellable(true);
    window.electronAPI.listSessions()
      .then((sessions) => { setSessionList(sessions); setSessionPhase('picker'); })
      .catch(() => setSessionPhase('picker'));
  }, [saveCurrentSession]);


  // --- Normal app logic ---

  const scrollToTab = useCallback((id: string) => {
    scrollContainerRef.current?.scrollToTab(id);
  }, []);

  const toggleHelp = useCallback(() => setShowHelp((prev) => !prev), []);
  const closeHelp = useCallback(() => setShowHelp(false), []);

  const insertPosition = config.panes.insertPosition || 'after';

  const addTabWithConfig = useCallback((type: PaneType, title?: string, shell?: string, url?: string, appMode?: boolean, cwd?: string, profileId?: string, resumeSessionId?: string, attachSessionId?: string, initialCommand?: string) => {
    return addTab(type, title, insertPosition, shell, url, appMode, cwd, profileId, resumeSessionId, attachSessionId, initialCommand);
  }, [addTab, insertPosition]);

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

  const handleSpawnAgent = useCallback((opts: { cwd: string; name?: string; profileId?: string; model?: string; skipPermissions?: boolean }) => {
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
    onPrevAgent: handlePrevAgent,
    onNextAgent: handleNextAgent,
    onNextAttention: goToNextAttention,
    onSpawnAgent: handleSpawnAgentShortcut,
    shortcuts: config.keybindings?.shortcuts ?? {},
  });

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

  // Bind plugin-contributed hotkeys: open-pane:<type> or emit:<eventType>.
  useEffect(() => {
    if (pluginHotkeys.length === 0) return;
    const matches = (combo: string, e: KeyboardEvent): boolean => {
      const parts = combo.toLowerCase().split('+');
      const key = parts[parts.length - 1];
      return e.ctrlKey === parts.includes('ctrl')
        && e.shiftKey === parts.includes('shift')
        && e.altKey === parts.includes('alt')
        && e.metaKey === parts.includes('meta')
        && e.key.toLowerCase() === key;
    };
    const handler = (e: KeyboardEvent) => {
      for (const h of pluginHotkeys) {
        if (!matches(h.combo, e)) continue;
        e.preventDefault();
        if (h.command.startsWith('open-pane:')) {
          const type = h.command.slice('open-pane:'.length);
          const pane = pluginPanes.find((p) => p.type === type);
          if (pane) handleOpenPlugin(pane);
        } else if (h.command.startsWith('emit:')) {
          window.electronAPI.hubPublish?.({ type: h.command.slice('emit:'.length), data: {} });
        }
        return;
      }
    };
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, [pluginHotkeys, pluginPanes, handleOpenPlugin]);

  // Library quick-picker hotkey (default ctrl+shift+l): opens the palette
  // restricted to prompts & skills.
  useEffect(() => {
    const combo = config.keybindings?.shortcuts?.['library-picker'];
    if (!combo) return;
    const parts = combo.toLowerCase().split('+');
    const key = parts[parts.length - 1];
    const handler = (e: KeyboardEvent) => {
      if (e.ctrlKey === parts.includes('ctrl') && e.shiftKey === parts.includes('shift')
        && e.altKey === parts.includes('alt') && e.metaKey === parts.includes('meta')
        && e.key.toLowerCase() === key) {
        e.preventDefault();
        openLibraryPicker();
      }
    };
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, [config.keybindings?.shortcuts, openLibraryPicker]);

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
  const navHeight = Math.max(config.ui.navBarHeight || 34, 32);

  const viewMode: ViewMode =
    config.panes?.viewMode === 'spatial' ? 'spatial'
    : config.panes?.viewMode === 'timeline' ? 'timeline'
    : 'tabs';
  const toggleViewMode = useCallback(() => {
    // Cycle: tabs → spatial → timeline → tabs
    const order: ViewMode[] = ['tabs', 'spatial', 'timeline'];
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
    <div className="app-root">
      <SideBar
        agents={agents}
        activeAgentId={activeAgentId}
        statusBySession={statusBySession}
        usageBySession={usageBySession}
        onSelectAgent={handleSelectAgent}
        onSpawnAgent={() => setShowSpawnDialog(true)}
        onTerminateAgent={terminateAgent}
        onRenameAgent={renameAgent}
        onJumpToAttention={goToNextAttention}
        onOpenRemote={() => setShowRemote(true)}
      />

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
        leftOffset={SIDEBAR_WIDTH}
        cwd={agentCwd || undefined}
        scripts={dirScripts}
        onRunScript={handleRunScript}
        onSaveScripts={handleSaveScripts}
      />

      <div className="app-content" style={{
        // Small gap between the tab bar and the top of the panes.
        marginTop: `${navHeight + 8}px`,
        marginLeft: `${SIDEBAR_WIDTH}px`,
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
                  onNavigateToTab={handleTabClick}
                  onAddTab={handleAddTab}
                  ptyMapping={ptyMapping}
                  renameSignal={renameSignal}
                  workspaceAgents={agents.filter((a) => !a.global).map((a) => ({ sessionId: a.sessionId }))}
                  appCwd={appCwd}
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
    </div>
  );
}

export default App;
