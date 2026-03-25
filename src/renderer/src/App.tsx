import { useRef, useCallback, useState } from 'react';
import './App.css';
import NavBar from './components/NavBar';
import ScrollContainer, { ScrollContainerRef } from './components/ScrollContainer';
import ScrollIndicator from './components/ScrollIndicator';
import ShortcutOverlay from './components/ShortcutOverlay';
import { usePaneManager } from './hooks/usePaneManager';
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
    activePaneId,
    setActivePaneId,
  } = usePaneManager();

  const scrollContainerRef = useRef<ScrollContainerRef>(null);
  const [showHelp, setShowHelp] = useState(false);
  const [renameSignal, setRenameSignal] = useState(0);
  const [chordState, setChordState] = useState<'idle' | 'waiting'>('idle');

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

  // Open settings — singleton: navigate to existing or create new
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
  const kbLeader = config.keybindings?.leader ?? 'ctrl+space';

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

      {/* Chord mode indicator */}
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
