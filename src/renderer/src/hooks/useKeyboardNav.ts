import { useEffect, useCallback, useRef } from 'react';
import { PaneConfig, PaneType } from '../types/pane';

const RESIZE_STEP = 80;
const CHORD_TIMEOUT = 500;

type ChordState = 'idle' | 'waiting';

interface UseKeyboardNavOptions {
  panes: PaneConfig[];
  activePaneId: string;
  setActivePaneId: (id: string) => void;
  scrollToPane: (id: string) => void;
  addPane: (type: PaneType, title?: string, width?: number, shell?: string) => string;
  removePane: (id: string) => void;
  resizePane: (id: string, width: number) => void;
  resetPaneWidth: (id: string) => void;
  movePane: (id: string, toIndex: number) => void;
  defaultPaneWidth: number;
  onToggleHelp: () => void;
  onRenamePane?: () => void;
  keybindingsMode?: 'default' | 'vim';
  leaderKey?: string;
  onChordStateChange?: (state: ChordState) => void;
  onOpenSettings?: () => void;
}

/**
 * Map from key name to e.code values for keys where e.key is unreliable.
 */
const KEY_TO_CODE: Record<string, string> = {
  space: 'Space',
};

const MODIFIER_NAMES = new Set(['ctrl', 'alt', 'shift', 'meta']);

/**
 * Parse a key combo string into a matcher function.
 * Supports: "ctrl+space", "alt+a", or modifier-only like "ctrl", "alt".
 * A modifier-only leader fires on keydown of that modifier key.
 */
function parseKeyCombo(combo: string): { match: (e: KeyboardEvent) => boolean; isModifierOnly: boolean } {
  const parts = combo.toLowerCase().split('+');
  const lastPart = parts[parts.length - 1];
  const isModifierOnly = parts.length === 1 && MODIFIER_NAMES.has(lastPart);

  if (isModifierOnly) {
    // Map modifier name to e.key value
    const modKeyMap: Record<string, string> = { ctrl: 'Control', alt: 'Alt', shift: 'Shift', meta: 'Meta' };
    const expectedKey = modKeyMap[lastPart];
    return {
      isModifierOnly: true,
      match: (e: KeyboardEvent) => e.key === expectedKey,
    };
  }

  // Regular combo: modifier(s) + key
  const key = lastPart;
  const needsCtrl = parts.includes('ctrl');
  const needsAlt = parts.includes('alt');
  const needsShift = parts.includes('shift');
  const needsMeta = parts.includes('meta');
  const expectedCode = KEY_TO_CODE[key];

  return {
    isModifierOnly: false,
    match: (e: KeyboardEvent) => {
      let keyMatch: boolean;
      if (expectedCode) {
        keyMatch = e.code === expectedCode;
      } else {
        const eventKey = e.key === ' ' ? 'space' : e.key.toLowerCase();
        keyMatch = eventKey === key;
      }
      return (
        keyMatch &&
        e.ctrlKey === needsCtrl &&
        e.altKey === needsAlt &&
        e.shiftKey === needsShift &&
        e.metaKey === needsMeta
      );
    },
  };
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
  onRenamePane,
  keybindingsMode = 'default',
  leaderKey = 'ctrl',
  onChordStateChange,
  onOpenSettings,
}: UseKeyboardNavOptions) {
  const chordRef = useRef<{ state: ChordState; timeoutId: ReturnType<typeof setTimeout> | null }>({
    state: 'idle',
    timeoutId: null,
  });

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

  // For modifier-only leaders: track whether the modifier was "tapped" (pressed+released without another key)
  const modTapRef = useRef<{ pressed: boolean; usedInCombo: boolean }>({ pressed: false, usedInCombo: false });

  useEffect(() => {
    const leaderConfig = parseKeyCombo(leaderKey);

    const cancelChord = () => {
      if (chordRef.current.timeoutId) {
        clearTimeout(chordRef.current.timeoutId);
        chordRef.current.timeoutId = null;
      }
      chordRef.current.state = 'idle';
      onChordStateChange?.('idle');
    };

    const startChord = () => {
      chordRef.current.state = 'waiting';
      onChordStateChange?.('waiting');
      chordRef.current.timeoutId = setTimeout(cancelChord, CHORD_TIMEOUT);
    };

    const executeChordAction = (key: string) => {
      const num = parseInt(key, 10);

      if (num >= 1 && num <= 9) {
        goToIndex(num - 1);
      } else if (key === 'h') {
        goToPrev();
      } else if (key === 'l') {
        goToNext();
      } else if (key === 'H') {
        const idx = panes.findIndex((p) => p.id === activePaneId);
        if (idx > 0) movePane(activePaneId, idx - 1);
      } else if (key === 'L') {
        const idx = panes.findIndex((p) => p.id === activePaneId);
        if (idx < panes.length - 1) movePane(activePaneId, idx + 1);
      } else if (key === 'n') {
        const newId = addPane('terminal');
        requestAnimationFrame(() => scrollToPane(newId));
      } else if (key === 'b') {
        const newId = addPane('browser');
        requestAnimationFrame(() => scrollToPane(newId));
      } else if (key === 'q') {
        removePane(activePaneId);
      } else if (key === 'r') {
        onRenamePane?.();
      } else if (key === '+' || key === '>') {
        const pane = panes.find((p) => p.id === activePaneId);
        if (pane) {
          const current = pane.widthOverride ?? defaultPaneWidth;
          resizePane(activePaneId, current + RESIZE_STEP);
        }
      } else if (key === '-' || key === '<') {
        const pane = panes.find((p) => p.id === activePaneId);
        if (pane) {
          const current = pane.widthOverride ?? defaultPaneWidth;
          resizePane(activePaneId, Math.max(300, current - RESIZE_STEP));
        }
      } else if (key === '=') {
        resetPaneWidth(activePaneId);
      } else if (key === '?') {
        onToggleHelp();
      }
    };

    // For modifier-only leaders: detect "tap" via keyup
    const handleKeyUp = (e: KeyboardEvent) => {
      if (keybindingsMode !== 'vim' || !leaderConfig.isModifierOnly) return;

      if (leaderConfig.match(e) && modTapRef.current.pressed && !modTapRef.current.usedInCombo) {
        // Modifier was pressed and released without any other key — it's a tap
        modTapRef.current.pressed = false;
        if (chordRef.current.state === 'idle') {
          startChord();
        }
      }
      modTapRef.current.pressed = false;
    };

    const handler = (e: KeyboardEvent) => {
      // Ctrl+, : open settings (both modes)
      if (e.ctrlKey && !e.shiftKey && !e.altKey && e.key === ',') {
        e.preventDefault();
        e.stopPropagation();
        onOpenSettings?.();
        return;
      }

      // --- Vim chord handling ---
      if (keybindingsMode === 'vim') {
        const isLeaderCapture = e.target instanceof HTMLElement && e.target.dataset.leaderCapture === 'true';

        // Track modifier-only leader: on keydown of the modifier, start tracking
        if (leaderConfig.isModifierOnly && !isLeaderCapture) {
          if (leaderConfig.match(e)) {
            // Modifier key pressed — start tracking for a tap
            modTapRef.current.pressed = true;
            modTapRef.current.usedInCombo = false;
            return; // Let the event pass through (it's just Ctrl down)
          } else if (modTapRef.current.pressed) {
            // Another key pressed while modifier held — this is a combo, not a tap
            modTapRef.current.usedInCombo = true;
          }
        }

        if (chordRef.current.state === 'waiting') {
          // Ignore modifier-only keypresses while waiting for action
          if (['Control', 'Shift', 'Alt', 'Meta'].includes(e.key)) {
            return;
          }

          // We're waiting for an action key
          e.preventDefault();
          e.stopPropagation();
          cancelChord();
          executeChordAction(e.key);
          return;
        }

        // Non-modifier leader (e.g. ctrl+space): detect on keydown
        if (!leaderConfig.isModifierOnly && !isLeaderCapture && leaderConfig.match(e)) {
          e.preventDefault();
          e.stopPropagation();
          startChord();
          return;
        }
      }

      // --- Direct shortcuts (both modes) ---

      // Ctrl+T: new terminal pane
      if (e.ctrlKey && !e.shiftKey && !e.altKey && e.key === 't') {
        e.preventDefault();
        e.stopPropagation();
        const newId = addPane('terminal');
        requestAnimationFrame(() => scrollToPane(newId));
        return;
      }

      // Ctrl+B: new browser pane
      if (e.ctrlKey && !e.shiftKey && !e.altKey && e.key === 'b') {
        e.preventDefault();
        e.stopPropagation();
        const newId = addPane('browser');
        requestAnimationFrame(() => scrollToPane(newId));
        return;
      }

      // Ctrl+W: close active pane
      if (e.ctrlKey && !e.shiftKey && !e.altKey && e.key === 'w') {
        e.preventDefault();
        e.stopPropagation();
        removePane(activePaneId);
        return;
      }

      // F2: rename active pane
      if (e.key === 'F2' && !e.ctrlKey && !e.altKey && !e.shiftKey) {
        e.preventDefault();
        e.stopPropagation();
        if (onRenamePane) onRenamePane();
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
    window.addEventListener('keyup', handleKeyUp, true);
    return () => {
      window.removeEventListener('keydown', handler, true);
      window.removeEventListener('keyup', handleKeyUp, true);
      cancelChord();
    };
  }, [goToIndex, goToPrev, goToNext, addPane, removePane, resizePane, resetPaneWidth, movePane, defaultPaneWidth, panes, activePaneId, scrollToPane, onToggleHelp, onRenamePane, keybindingsMode, leaderKey, onChordStateChange, onOpenSettings]);

  return {
    activePaneId,
    goToIndex,
    goToPrev,
    goToNext,
  };
}
