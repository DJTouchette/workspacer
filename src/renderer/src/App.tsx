import { useRef, useCallback, useState, useEffect } from 'react';
import './App.css';
import NavBar from './components/NavBar';
import ScrollContainer, { ScrollContainerRef } from './components/ScrollContainer';
import ScrollIndicator from './components/ScrollIndicator';
import ShortcutOverlay from './components/ShortcutOverlay';
import SessionPicker from './components/SessionPicker';
import CommandPalette from './components/CommandPalette';
import { usePaneManager, defaultPanes } from './hooks/usePaneManager';
import type { PaneType } from './types/pane';
import { useKeyboardNav } from './hooks/useKeyboardNav';
import { useConfig } from './hooks/useConfig';

function App() {
  const { config } = useConfig();
  const {
    panes,
    addPane,
    removePane,
    renamePane,
    resizePane,
    resetPaneWidth,
    movePane,
    updatePaneUrl,
    loadFromSession,
    activePaneId,
    setActivePaneId,
  } = usePaneManager();

  const scrollContainerRef = useRef<ScrollContainerRef>(null);
  const [showHelp, setShowHelp] = useState(false);
  const [renameSignal, setRenameSignal] = useState(0);
  const [chordState, setChordState] = useState<'idle' | 'waiting'>('idle');
  const [showCommandPalette, setShowCommandPalette] = useState(false);

  // Session state
  const [sessionPhase, setSessionPhase] = useState<'loading' | 'picker' | 'active'>('loading');
  const [sessionList, setSessionList] = useState<any[]>([]);
  const [sessionName, setSessionName] = useState('Default');

  // PTY mapping: paneId -> ptySessionId (for CWD lookup on save)
  const ptyMappingRef = useRef<Record<string, string>>({});

  const handlePtyReady = useCallback((paneId: string, ptySessionId: string) => {
    ptyMappingRef.current[paneId] = ptySessionId;
  }, []);

  const handleUrlChange = useCallback((paneId: string, url: string) => {
    updatePaneUrl(paneId, url);
  }, [updatePaneUrl]);

  // --- Session lifecycle ---

  // On startup: check for saved sessions
  useEffect(() => {
    // Load defaults immediately, then check for sessions
    loadFromSession(defaultPanes, defaultPanes[0].id);

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
    loadFromSession(defaultPanes, defaultPanes[0].id);
    setSessionName('Default');
    ptyMappingRef.current = {};
    setSessionPhase('active');
  }, [loadFromSession]);

  const handleResumeSession = useCallback((filename: string) => {
    window.electronAPI.loadSession(filename).then((data: any) => {
      if (data && data.panes?.length > 0) {
        loadFromSession(data.panes, data.activePaneId);
        setSessionName(data.name || 'Default');
      } else {
        loadFromSession(defaultPanes, defaultPanes[0].id);
      }
      ptyMappingRef.current = {};
      setSessionPhase('active');
    }).catch(() => {
      loadFromSession(defaultPanes, defaultPanes[0].id);
      setSessionPhase('active');
    });
  }, [loadFromSession]);

  const handleDeleteSession = useCallback((filename: string) => {
    window.electronAPI.deleteSession(filename).then(() => {
      setSessionList((prev) => prev.filter((s) => s.filename !== filename));
    });
  }, []);

  // Save current session
  const saveCurrentSession = useCallback(() => {
    if (sessionPhase !== 'active' || panes.length === 0) return;

    const paneData = panes.map((p) => ({
      id: p.id,
      type: p.type,
      title: p.title,
      width: p.width,
      widthOverride: p.widthOverride,
      shell: p.shell,
      url: p.url,
    }));

    window.electronAPI.saveSession({
      name: sessionName,
      activePaneId,
      panes: paneData,
      ptyMapping: { ...ptyMappingRef.current },
    }).catch((err: any) => {
      console.error('[Session] save failed:', err);
    });
  }, [panes, activePaneId, sessionName, sessionPhase]);

  // Auto-save every 30s
  useEffect(() => {
    if (sessionPhase !== 'active') return;
    const interval = setInterval(saveCurrentSession, 30000);
    return () => clearInterval(interval);
  }, [sessionPhase, saveCurrentSession]);

  // Save on window close
  useEffect(() => {
    const handler = () => saveCurrentSession();
    window.addEventListener('beforeunload', handler);
    return () => window.removeEventListener('beforeunload', handler);
  }, [saveCurrentSession]);

  // Save on app quit signal from main process
  useEffect(() => {
    const unsub = window.electronAPI.onBeforeQuit(() => {
      saveCurrentSession();
    });
    return unsub;
  }, [saveCurrentSession]);

  // --- Normal app logic ---

  const scrollToPane = useCallback((id: string) => {
    scrollContainerRef.current?.scrollToPane(id);
  }, []);

  const toggleHelp = useCallback(() => {
    setShowHelp((prev) => !prev);
  }, []);

  const closeHelp = useCallback(() => {
    setShowHelp(false);
  }, []);

  const insertPosition = config.panes.insertPosition || 'after';

  const addPaneWithConfig = useCallback((type: PaneType, title?: string, width?: number, shell?: string) => {
    return addPane(type, title, width, insertPosition, shell);
  }, [addPane, insertPosition]);

  const openSettings = useCallback(() => {
    const existing = panes.find((p) => p.type === 'settings');
    if (existing) {
      setActivePaneId(existing.id);
      scrollToPane(existing.id);
    } else {
      const newId = addPaneWithConfig('settings', 'Settings');
      requestAnimationFrame(() => scrollToPane(newId));
    }
  }, [panes, addPaneWithConfig, setActivePaneId, scrollToPane]);

  const kbMode = config.keybindings?.mode ?? 'default';
  const kbLeader = config.keybindings?.leader ?? 'ctrl';

  useKeyboardNav({
    panes,
    activePaneId,
    setActivePaneId,
    scrollToPane,
    addPane: addPaneWithConfig,
    removePane,
    resizePane,
    resetPaneWidth,
    movePane,
    defaultPaneWidth: config.panes.defaultWidth || 800,
    onToggleHelp: toggleHelp,
    onRenamePane: useCallback(() => setRenameSignal((s) => s + 1), []),
    keybindingsMode: kbMode,
    leaderKey: kbLeader,
    onChordStateChange: setChordState,
    onOpenSettings: openSettings,
    onSaveSession: saveCurrentSession,
    onOpenCommandPalette: useCallback(() => setShowCommandPalette(true), []),
  });

  const handlePaneClick = useCallback((id: string) => {
    setActivePaneId(id);
    scrollToPane(id);
  }, [setActivePaneId, scrollToPane]);

  const handlePaneClose = useCallback((id: string) => {
    removePane(id);
  }, [removePane]);

  const handlePaneFocus = useCallback((id: string) => {
    setActivePaneId(id);
  }, [setActivePaneId]);

  const handleAddPane = useCallback((type: PaneType, shell?: string, label?: string) => {
    const title = label ?? undefined;
    const newId = addPaneWithConfig(type, title, undefined, shell);
    requestAnimationFrame(() => {
      scrollToPane(newId);
    });
  }, [addPaneWithConfig, scrollToPane]);

  const handleLaunchApp = useCallback((app: { name: string; url: string }) => {
    const newId = addPane('browser', app.name, undefined, insertPosition, undefined, app.url);
    requestAnimationFrame(() => scrollToPane(newId));
  }, [addPane, insertPosition, scrollToPane]);

  // --- Render ---

  // --- Render ---
  return (
    <div className="app-root">
      <NavBar
        panes={panes}
        activePaneId={activePaneId}
        onPaneClick={handlePaneClick}
        onAddPane={handleAddPane}
      />

      <div className="app-content" style={{ marginTop: `${config.ui.navBarHeight || 28}px` }}>
        <ScrollContainer
          ref={scrollContainerRef}
          panes={panes}
          activePaneId={activePaneId}
          onPaneFocus={handlePaneFocus}
          onPaneClose={handlePaneClose}
          onPaneResize={resizePane}
          onPaneResetWidth={resetPaneWidth}
          onPaneMove={movePane}
          onPaneRename={renamePane}
          onPtyReady={handlePtyReady}
          onUrlChange={handleUrlChange}
          renameSignal={renameSignal}
        />
      </div>

      <ScrollIndicator
        panes={panes}
        activePaneId={activePaneId}
        onDotClick={handlePaneClick}
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
          backgroundColor: 'rgb(80, 120, 200)',
          color: '#fff',
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
