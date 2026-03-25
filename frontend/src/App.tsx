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
    resizePane,
    resetPaneWidth,
    movePane,
    activePaneId,
    setActivePaneId,
  } = usePaneManager();

  const scrollContainerRef = useRef<ScrollContainerRef>(null);
  const [scrollFraction, setScrollFraction] = useState(0);
  const [showHelp, setShowHelp] = useState(false);

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

  const handleScrollChange = useCallback(
    (scrollLeft: number, scrollWidth: number, clientWidth: number) => {
      const maxScroll = scrollWidth - clientWidth;
      setScrollFraction(maxScroll > 0 ? scrollLeft / maxScroll : 0);
    },
    []
  );

  const handleAddPane = useCallback((shell?: string, label?: string) => {
    const title = label ?? undefined;
    const newId = addPaneWithConfig('terminal', title, undefined, shell);
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
          onScrollChange={handleScrollChange}
        />
      </div>

      <ScrollIndicator
        panes={panes}
        activePaneId={activePaneId}
        scrollFraction={scrollFraction}
        onDotClick={handlePaneClick}
      />

      <ShortcutOverlay visible={showHelp} onClose={closeHelp} />
    </div>
  );
}

export default App;
