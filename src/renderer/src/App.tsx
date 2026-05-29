import { useRef, useCallback, useState, useEffect } from 'react';
import './App.css';
import NavBar from './components/NavBar';
import SideBar, { SIDEBAR_WIDTH } from './components/SideBar';
import SpawnAgentDialog from './components/SpawnAgentDialog';
import ScrollContainer, { ScrollContainerRef } from './components/ScrollContainer';
import ScrollIndicator from './components/ScrollIndicator';
import ShortcutOverlay from './components/ShortcutOverlay';
import SessionPicker from './components/SessionPicker';
import CommandPalette from './components/CommandPalette';
import { useAgentManager } from './hooks/useAgentManager';
import type { PaneType, AgentWorkspace } from './types/pane';
import type { SessionAmbientState } from './types/claudeSession';
import { useKeyboardNav } from './hooks/useKeyboardNav';
import { useConfig } from './hooks/useConfig';
import { useTheme } from './hooks/useTheme';

function App() {
  const { config } = useConfig();
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
  const [chordState, setChordState] = useState<'idle' | 'waiting'>('idle');
  const [showCommandPalette, setShowCommandPalette] = useState(false);
  const [paletteMode, setPaletteMode] = useState<'tab' | 'split'>('tab');
  const [showSpawnDialog, setShowSpawnDialog] = useState(false);

  // App working directory (used as the default cwd for the spawn dialog)
  const appCwdRef = useRef<string>('');
  useEffect(() => {
    window.electronAPI.getCwd().then((cwd) => { appCwdRef.current = cwd; }).catch(() => {});
  }, []);

  // Live agent status: sessionId -> ambient state, sourced from claudemon.
  const [statusBySession, setStatusBySession] = useState<Record<string, SessionAmbientState>>({});
  useEffect(() => {
    let cancelled = false;
    window.electronAPI.getAllClaudeSessions().then((sessions: any[]) => {
      if (cancelled) return;
      const map: Record<string, SessionAmbientState> = {};
      for (const s of sessions) map[s.sessionId] = s.ambientState;
      setStatusBySession(map);
    }).catch(() => {});
    const unsub = window.electronAPI.onClaudeSessionUpdate((sessionId: string, snapshot: any) => {
      setStatusBySession((prev) => ({ ...prev, [sessionId]: snapshot.ambientState }));
    });
    return () => { cancelled = true; unsub(); };
  }, []);

  // Session state
  const [sessionPhase, setSessionPhase] = useState<'loading' | 'picker' | 'active'>('loading');
  const [sessionList, setSessionList] = useState<any[]>([]);
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
  useEffect(() => {
    window.electronAPI.listSessions().then((sessions) => {
      if (sessions.length > 0) {
        setSessionList(sessions);
        setSessionPhase('picker');
      } else {
        setSessionPhase('active');
      }
    }).catch(() => {
      setSessionPhase('active');
    });
  }, []);

  const handleNewSession = useCallback(() => {
    loadAgentsFromSession([], '');
    setSessionName('Default');
    setPtyMapping({});
    setSessionPhase('active');
  }, [loadAgentsFromSession]);

  const handleResumeSession = useCallback((filename: string) => {
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

  // --- Normal app logic ---

  const scrollToTab = useCallback((id: string) => {
    scrollContainerRef.current?.scrollToTab(id);
  }, []);

  const toggleHelp = useCallback(() => setShowHelp((prev) => !prev), []);
  const closeHelp = useCallback(() => setShowHelp(false), []);

  const insertPosition = config.panes.insertPosition || 'after';

  const addTabWithConfig = useCallback((type: PaneType, title?: string, shell?: string, url?: string, appMode?: boolean, cwd?: string, profileId?: string, resumeSessionId?: string, attachSessionId?: string) => {
    return addTab(type, title, insertPosition, shell, url, appMode, cwd, profileId, resumeSessionId, attachSessionId);
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

  const handleSpawnAgent = useCallback((opts: { cwd: string; name?: string; profileId?: string }) => {
    setShowSpawnDialog(false);
    void spawnAgent(opts);
  }, [spawnAgent]);

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
    onOpenCommandPalette: useCallback(() => { setPaletteMode('tab'); setShowCommandPalette(true); }, []),
    onOpenSplitPalette: useCallback(() => { setPaletteMode('split'); setShowCommandPalette(true); }, []),
    onPrevAgent: handlePrevAgent,
    onNextAgent: handleNextAgent,
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

  // --- Render ---
  const navHeight = Math.max(config.ui.navBarHeight || 34, 32);

  const handleNavBarRename = useCallback(
    (tabId: string) => { setActiveTabId(tabId); setRenameSignal((s) => s + 1); },
    [setActiveTabId],
  );
  const handleNavBarSplit = useCallback(
    (tabId: string, type: PaneType) => { splitTab(tabId, type); },
    [splitTab],
  );

  return (
    <div className="app-root">
      <SideBar
        agents={agents}
        activeAgentId={activeAgentId}
        statusBySession={statusBySession}
        onSelectAgent={handleSelectAgent}
        onSpawnAgent={() => setShowSpawnDialog(true)}
        onTerminateAgent={terminateAgent}
        onRenameAgent={renameAgent}
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
        leftOffset={SIDEBAR_WIDTH}
      />

      <div className="app-content" style={{
        marginTop: `${navHeight}px`,
        marginLeft: `${SIDEBAR_WIDTH}px`,
      }}>
        {activeAgent ? (
          <ScrollContainer
            ref={scrollContainerRef}
            tabs={tabs}
            activeTabId={activeTabId}
            onTabFocus={handleTabFocus}
            onPaneClose={handlePaneClose}
            onPaneFocus={handlePaneFocus}
            onTabRename={renameTab}
            onTabMove={moveTab}
            onPtyReady={handlePtyReady}
            onUrlChange={handleUrlChange}
            onNavigateToTab={handleTabClick}
            onAddTab={handleAddTab}
            ptyMapping={ptyMapping}
            renameSignal={renameSignal}
          />
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

      {activeAgent && (
        <ScrollIndicator
          tabs={tabs}
          activeTabId={activeTabId}
          onDotClick={handleTabClick}
        />
      )}

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
        onClose={useCallback(() => setShowCommandPalette(false), [])}
        onLaunchApp={handleLaunchApp}
        onAddTab={handleAddTab}
        onSplitPane={handleSplitPane}
      />

      {showSpawnDialog && (
        <SpawnAgentDialog
          defaultCwd={appCwdRef.current}
          onSpawn={handleSpawnAgent}
          onCancel={() => setShowSpawnDialog(false)}
        />
      )}

      {sessionPhase === 'picker' && (
        <SessionPicker
          sessions={sessionList}
          onNewSession={handleNewSession}
          onResumeSession={handleResumeSession}
          onDeleteSession={handleDeleteSession}
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
