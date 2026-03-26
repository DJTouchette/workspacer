import { useRef, useCallback, useState, useEffect } from 'react';
import './App.css';
import NavBar from './components/NavBar';
import ScrollContainer, { ScrollContainerRef } from './components/ScrollContainer';
import ScrollIndicator from './components/ScrollIndicator';
import ShortcutOverlay from './components/ShortcutOverlay';
import SessionPicker from './components/SessionPicker';
import CommandPalette from './components/CommandPalette';
import { useTabManager, defaultTabs } from './hooks/useTabManager';
import type { PaneType } from './types/pane';
import { useKeyboardNav } from './hooks/useKeyboardNav';
import { useConfig } from './hooks/useConfig';
import { useTheme } from './hooks/useTheme';

function App() {
  const { config } = useConfig();
  useTheme();
  const {
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
    loadFromSession,
    getActiveTab,
  } = useTabManager();

  const scrollContainerRef = useRef<ScrollContainerRef>(null);
  const [showHelp, setShowHelp] = useState(false);
  const [renameSignal, setRenameSignal] = useState(0);
  const [chordState, setChordState] = useState<'idle' | 'waiting'>('idle');
  const [showCommandPalette, setShowCommandPalette] = useState(false);

  // App working directory (used as default cwd for Claude panes)
  const appCwdRef = useRef<string>('');
  useEffect(() => {
    window.electronAPI.getCwd().then((cwd) => { appCwdRef.current = cwd; }).catch(() => {});
  }, []);

  // Font loading happens in main.tsx at module level (before React mounts)
  // Terminal panes await window.__fontsReady before calling term.open()

  // Session state
  const [sessionPhase, setSessionPhase] = useState<'loading' | 'picker' | 'active'>('loading');
  const [sessionList, setSessionList] = useState<any[]>([]);
  const [sessionName, setSessionName] = useState('Default');

  // PTY mapping: paneId -> ptySessionId
  const ptyMappingRef = useRef<Record<string, string>>({});

  const handlePtyReady = useCallback((paneId: string, ptySessionId: string) => {
    ptyMappingRef.current[paneId] = ptySessionId;
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

  // Hibernate timer
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

  // --- Session lifecycle ---
  useEffect(() => {
    loadFromSession(defaultTabs, defaultTabs[0].id);
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
  }, [loadFromSession]);

  const handleNewSession = useCallback(() => {
    loadFromSession(defaultTabs, defaultTabs[0].id);
    setSessionName('Default');
    ptyMappingRef.current = {};
    setSessionPhase('active');
  }, [loadFromSession]);

  const handleResumeSession = useCallback((filename: string) => {
    window.electronAPI.loadSession(filename).then((data: any) => {
      if (data && data.tabs?.length > 0) {
        loadFromSession(data.tabs, data.activeTabId);
        setSessionName(data.name || 'Default');
      } else if (data && data.panes?.length > 0) {
        // Backward compat: old flat pane format → wrap each in a tab
        const migrated = data.panes.map((p: any) => ({
          id: `tab-${p.id}`,
          title: p.title,
          panes: [p],
          activePaneId: p.id,
        }));
        loadFromSession(migrated, `tab-${data.activePaneId}`);
        setSessionName(data.name || 'Default');
      } else {
        loadFromSession(defaultTabs, defaultTabs[0].id);
      }
      ptyMappingRef.current = {};
      setSessionPhase('active');
    }).catch(() => {
      loadFromSession(defaultTabs, defaultTabs[0].id);
      setSessionPhase('active');
    });
  }, [loadFromSession]);

  const handleDeleteSession = useCallback((filename: string) => {
    window.electronAPI.deleteSession(filename).then(() => {
      setSessionList((prev) => prev.filter((s) => s.filename !== filename));
    });
  }, []);

  const saveCurrentSession = useCallback(() => {
    if (sessionPhase !== 'active' || tabs.length === 0) return;
    window.electronAPI.saveSession({
      name: sessionName,
      activeTabId,
      tabs: tabs.map((t) => ({
        ...t,
        panes: t.panes.map((p) => ({ ...p })),
      })),
      ptyMapping: { ...ptyMappingRef.current },
    }).catch((err: any) => {
      console.error('[Session] save failed:', err);
    });
  }, [tabs, activeTabId, sessionName, sessionPhase]);

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
    const unsub = window.electronAPI.onBeforeQuit(() => saveCurrentSession());
    return unsub;
  }, [saveCurrentSession]);

  // --- Normal app logic ---

  const scrollToTab = useCallback((id: string) => {
    scrollContainerRef.current?.scrollToTab(id);
  }, []);

  const toggleHelp = useCallback(() => setShowHelp((prev) => !prev), []);
  const closeHelp = useCallback(() => setShowHelp(false), []);

  const insertPosition = config.panes.insertPosition || 'after';

  const addTabWithConfig = useCallback((type: PaneType, title?: string, shell?: string, url?: string, appMode?: boolean, cwd?: string) => {
    return addTab(type, title, insertPosition, shell, url, appMode, cwd);
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

  // Get current tab for keyboard nav context
  const activeTab = getActiveTab();

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
    onOpenCommandPalette: useCallback(() => setShowCommandPalette(true), []),
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

  const handleAddTab = useCallback((type: PaneType, shell?: string, label?: string, cwd?: string) => {
    // Default Claude panes to the app's working directory
    const resolvedCwd = cwd || (type === 'claude' ? appCwdRef.current : undefined);
    const newId = addTabWithConfig(type, label, shell, undefined, undefined, resolvedCwd);
    requestAnimationFrame(() => scrollToTab(newId));
  }, [addTabWithConfig, scrollToTab]);

  const handleLaunchApp = useCallback((app: { name: string; url: string }) => {
    const newId = addTab('browser', app.name, insertPosition, undefined, app.url, true);
    requestAnimationFrame(() => scrollToTab(newId));
  }, [addTab, insertPosition, scrollToTab]);

  // --- Render ---
  return (
    <div className="app-root">
      <NavBar
        tabs={tabs}
        activeTabId={activeTabId}
        onTabClick={handleTabClick}
        onAddTab={handleAddTab}
      />

      <div className="app-content" style={{ marginTop: `${config.ui.navBarHeight || 28}px` }}>
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
          renameSignal={renameSignal}
        />
      </div>

      <ScrollIndicator
        tabs={tabs}
        activeTabId={activeTabId}
        onDotClick={handleTabClick}
      />

      <ShortcutOverlay
        visible={showHelp}
        onClose={closeHelp}
        mode={kbMode}
        leader={kbLeader}
      />

      <CommandPalette
        visible={showCommandPalette}
        apps={config.apps ?? []}
        onClose={useCallback(() => setShowCommandPalette(false), [])}
        onLaunchApp={handleLaunchApp}
      />

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
