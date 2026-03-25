import { useEffect, useCallback } from 'react';
import { PaneConfig, PaneType } from '../types/pane';

const RESIZE_STEP = 80;

interface UseKeyboardNavOptions {
  panes: PaneConfig[];
  activePaneId: string;
  setActivePaneId: (id: string) => void;
  scrollToPane: (id: string) => void;
  addPane: (type: PaneType, title?: string, width?: number) => string;
  removePane: (id: string) => void;
  resizePane: (id: string, width: number) => void;
  resetPaneWidth: (id: string) => void;
  movePane: (id: string, toIndex: number) => void;
  defaultPaneWidth: number;
  onToggleHelp: () => void;
}

export function useKeyboardNav({
  panes,
  activePaneId,
  setActivePaneId,
  scrollToPane,
  addPane,
  removePane,
  resizePane,
  resetPaneWidth,
  movePane,
  defaultPaneWidth,
  onToggleHelp,
}: UseKeyboardNavOptions) {
  const goToIndex = useCallback(
    (index: number) => {
      if (index >= 0 && index < panes.length) {
        const pane = panes[index];
        setActivePaneId(pane.id);
        scrollToPane(pane.id);
      }
    },
    [panes, setActivePaneId, scrollToPane]
  );

  const goToPrev = useCallback(() => {
    const currentIdx = panes.findIndex((p) => p.id === activePaneId);
    if (currentIdx > 0) {
      goToIndex(currentIdx - 1);
    }
  }, [panes, activePaneId, goToIndex]);

  const goToNext = useCallback(() => {
    const currentIdx = panes.findIndex((p) => p.id === activePaneId);
    if (currentIdx < panes.length - 1) {
      goToIndex(currentIdx + 1);
    }
  }, [panes, activePaneId, goToIndex]);

  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      // Ctrl+T: new terminal pane
      if (e.ctrlKey && !e.shiftKey && !e.altKey && e.key === 't') {
        e.preventDefault();
        e.stopPropagation();
        const newId = addPane('terminal');
        // Scroll to the new pane after it renders
        requestAnimationFrame(() => {
          scrollToPane(newId);
        });
        return;
      }

      // Ctrl+W: close active pane
      if (e.ctrlKey && !e.shiftKey && !e.altKey && e.key === 'w') {
        e.preventDefault();
        e.stopPropagation();
        removePane(activePaneId);
        return;
      }

      // Ctrl+/ or Ctrl+?: toggle help overlay
      if (e.ctrlKey && (e.key === '/' || e.key === '?')) {
        e.preventDefault();
        e.stopPropagation();
        onToggleHelp();
        return;
      }

      // Ctrl+1 through Ctrl+9: jump to pane by index
      if (e.ctrlKey && !e.shiftKey && !e.altKey) {
        const num = parseInt(e.key, 10);
        if (num >= 1 && num <= 9) {
          e.preventDefault();
          e.stopPropagation();
          goToIndex(num - 1);
          return;
        }
      }

      // Ctrl+Shift+1-9: move active pane to position
      if (e.ctrlKey && e.shiftKey && !e.altKey) {
        // Check for digits — e.key gives the shifted char (!, @, #...) so use e.code
        const digitMatch = e.code?.match(/^Digit(\d)$/);
        if (digitMatch) {
          const num = parseInt(digitMatch[1], 10);
          if (num >= 1 && num <= 9) {
            e.preventDefault();
            e.stopPropagation();
            movePane(activePaneId, num - 1);
            return;
          }
          if (num === 0) {
            // Ctrl+Shift+0: reset active pane width
            e.preventDefault();
            e.stopPropagation();
            resetPaneWidth(activePaneId);
            return;
          }
        }

        if (e.key === 'ArrowLeft') {
          e.preventDefault();
          e.stopPropagation();
          const pane = panes.find((p) => p.id === activePaneId);
          if (pane) {
            const current = pane.widthOverride ?? defaultPaneWidth;
            resizePane(activePaneId, Math.max(300, current - RESIZE_STEP));
          }
          return;
        }
        if (e.key === 'ArrowRight') {
          e.preventDefault();
          e.stopPropagation();
          const pane = panes.find((p) => p.id === activePaneId);
          if (pane) {
            const current = pane.widthOverride ?? defaultPaneWidth;
            resizePane(activePaneId, current + RESIZE_STEP);
          }
          return;
        }
      }

      // Alt+Left / Alt+Right: prev/next pane
      if (e.altKey && !e.ctrlKey && !e.shiftKey) {
        if (e.key === 'ArrowLeft') {
          e.preventDefault();
          e.stopPropagation();
          goToPrev();
          return;
        }
        if (e.key === 'ArrowRight') {
          e.preventDefault();
          e.stopPropagation();
          goToNext();
          return;
        }
      }
    };

    // Use capture phase so we intercept before xterm.js handles the event
    window.addEventListener('keydown', handler, true);
    return () => window.removeEventListener('keydown', handler, true);
  }, [goToIndex, goToPrev, goToNext, addPane, removePane, resizePane, resetPaneWidth, movePane, defaultPaneWidth, panes, activePaneId, scrollToPane, onToggleHelp]);

  return {
    activePaneId,
    goToIndex,
    goToPrev,
    goToNext,
  };
}
