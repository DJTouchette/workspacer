import { useEffect, useCallback, useRef } from 'react';
import { PaneType, TabConfig } from '../types/pane';

const CHORD_TIMEOUT = 500;

type ChordState = 'idle' | 'waiting';

const MODIFIER_NAMES = new Set(['ctrl', 'alt', 'shift', 'meta']);
const KEY_TO_CODE: Record<string, string> = { space: 'Space' };

function parseKeyCombo(combo: string): { match: (e: KeyboardEvent) => boolean; isModifierOnly: boolean } {
  const parts = combo.toLowerCase().split('+');
  const lastPart = parts[parts.length - 1];
  const isModifierOnly = parts.length === 1 && MODIFIER_NAMES.has(lastPart);

  if (isModifierOnly) {
    const modKeyMap: Record<string, string> = { ctrl: 'Control', alt: 'Alt', shift: 'Shift', meta: 'Meta' };
    const expectedKey = modKeyMap[lastPart];
    return { isModifierOnly: true, match: (e) => e.key === expectedKey };
  }

  const key = lastPart;
  const needsCtrl = parts.includes('ctrl');
  const needsAlt = parts.includes('alt');
  const needsShift = parts.includes('shift');
  const needsMeta = parts.includes('meta');
  const expectedCode = KEY_TO_CODE[key];

  return {
    isModifierOnly: false,
    match: (e) => {
      const keyMatch = expectedCode ? e.code === expectedCode : (e.key === ' ' ? 'space' : e.key.toLowerCase()) === key;
      return keyMatch && e.ctrlKey === needsCtrl && e.altKey === needsAlt && e.shiftKey === needsShift && e.metaKey === needsMeta;
    },
  };
}

interface UseKeyboardNavOptions {
  tabs: TabConfig[];
  activeTabId: string;
  activeTab?: TabConfig;
  setActiveTabId: (id: string) => void;
  scrollToTab: (id: string) => void;
  addTab: (type: PaneType, title?: string, shell?: string, url?: string, appMode?: boolean) => string;
  splitTab: (tabId: string, type: PaneType, title?: string, shell?: string, url?: string, appMode?: boolean, cwd?: string) => string;
  removeTab: (tabId: string) => void;
  removePane: (tabId: string, paneId: string) => void;
  renameTab: (tabId: string, title: string) => void;
  moveTab: (tabId: string, toIndex: number) => void;
  setActivePane: (tabId: string, paneId: string) => void;
  onToggleHelp: () => void;
  onRenameTab?: () => void;
  keybindingsMode?: 'default' | 'vim';
  leaderKey?: string;
  onChordStateChange?: (state: ChordState) => void;
  onOpenSettings?: () => void;
  onSaveSession?: () => void;
  onOpenCommandPalette?: () => void;
}

export function useKeyboardNav({
  tabs,
  activeTabId,
  activeTab,
  setActiveTabId,
  scrollToTab,
  addTab,
  splitTab,
  removeTab,
  removePane,
  renameTab,
  moveTab,
  setActivePane,
  onToggleHelp,
  onRenameTab,
  keybindingsMode = 'default',
  leaderKey = 'ctrl',
  onChordStateChange,
  onOpenSettings,
  onSaveSession,
  onOpenCommandPalette,
}: UseKeyboardNavOptions) {
  const chordRef = useRef<{ state: ChordState; timeoutId: ReturnType<typeof setTimeout> | null }>({
    state: 'idle', timeoutId: null,
  });
  const modTapRef = useRef<{ pressed: boolean; usedInCombo: boolean }>({ pressed: false, usedInCombo: false });

  // Tab navigation
  const goToTab = useCallback((index: number) => {
    if (index >= 0 && index < tabs.length) {
      const tab = tabs[index];
      setActiveTabId(tab.id);
      scrollToTab(tab.id);
    }
  }, [tabs, setActiveTabId, scrollToTab]);

  const goToPrevTab = useCallback(() => {
    const idx = tabs.findIndex((t) => t.id === activeTabId);
    goToTab(idx > 0 ? idx - 1 : tabs.length - 1);
  }, [tabs, activeTabId, goToTab]);

  const goToNextTab = useCallback(() => {
    const idx = tabs.findIndex((t) => t.id === activeTabId);
    goToTab(idx < tabs.length - 1 ? idx + 1 : 0);
  }, [tabs, activeTabId, goToTab]);

  // Sub-pane navigation within current tab
  const navigatePane = useCallback((direction: 'left' | 'right' | 'up' | 'down') => {
    if (!activeTab || activeTab.panes.length <= 1) return;
    const panes = activeTab.panes;
    const currentIdx = panes.findIndex((p) => p.id === activeTab.activePaneId);
    if (currentIdx < 0) return;

    const count = panes.length;
    const cols = count <= 1 ? 1 : count <= 2 ? 2 : count <= 4 ? 2 : count <= 6 ? 3 : Math.ceil(Math.sqrt(count));
    let targetIdx = currentIdx;

    if (direction === 'left') targetIdx = currentIdx - 1;
    else if (direction === 'right') targetIdx = currentIdx + 1;
    else if (direction === 'up') targetIdx = currentIdx - cols;
    else if (direction === 'down') targetIdx = currentIdx + cols;

    if (targetIdx >= 0 && targetIdx < count) {
      setActivePane(activeTab.id, panes[targetIdx].id);
    }
  }, [activeTab, setActivePane]);

  useEffect(() => {
    const leaderConfig = parseKeyCombo(leaderKey);

    const cancelChord = () => {
      if (chordRef.current.timeoutId) clearTimeout(chordRef.current.timeoutId);
      chordRef.current = { state: 'idle', timeoutId: null };
      onChordStateChange?.('idle');
    };

    const startChord = () => {
      chordRef.current.state = 'waiting';
      onChordStateChange?.('waiting');
      chordRef.current.timeoutId = setTimeout(cancelChord, CHORD_TIMEOUT);
    };

    const executeChordAction = (key: string) => {
      const num = parseInt(key, 10);
      if (num >= 1 && num <= 9) { goToTab(num - 1); }
      else if (key === 'h') goToPrevTab();
      else if (key === 'l') goToNextTab();
      else if (key === 'H') {
        const idx = tabs.findIndex((t) => t.id === activeTabId);
        if (idx > 0) moveTab(activeTabId, idx - 1);
      }
      else if (key === 'L') {
        const idx = tabs.findIndex((t) => t.id === activeTabId);
        if (idx < tabs.length - 1) moveTab(activeTabId, idx + 1);
      }
      else if (key === 'n') {
        const newId = addTab('terminal');
        requestAnimationFrame(() => scrollToTab(newId));
      }
      else if (key === 'b') {
        const newId = addTab('browser');
        requestAnimationFrame(() => scrollToTab(newId));
      }
      else if (key === 'q') removeTab(activeTabId);
      else if (key === 'r') onRenameTab?.();
      else if (key === '?') onToggleHelp();
      else if (key === 's') onSaveSession?.();
      else if (key === 'd') {
        if (activeTab) {
          const activePane = activeTab.panes.find(p => p.id === activeTab.activePaneId);
          const splitType = activePane?.type ?? 'terminal';
          splitTab(activeTab.id, splitType, undefined, activePane?.shell, undefined, undefined, activePane?.cwd);
        }
      }
    };

    const handleKeyUp = (e: KeyboardEvent) => {
      if (keybindingsMode !== 'vim' || !leaderConfig.isModifierOnly) return;
      if (leaderConfig.match(e) && modTapRef.current.pressed && !modTapRef.current.usedInCombo) {
        modTapRef.current.pressed = false;
        if (chordRef.current.state === 'idle') startChord();
      }
      modTapRef.current.pressed = false;
    };

    const handler = (e: KeyboardEvent) => {
      // --- Global shortcuts (both modes) ---

      // Ctrl+, : settings
      if (e.ctrlKey && !e.shiftKey && !e.altKey && e.key === ',') {
        e.preventDefault(); e.stopPropagation(); onOpenSettings?.(); return;
      }
      // Ctrl+S : save session
      if (e.ctrlKey && !e.shiftKey && !e.altKey && e.key === 's') {
        e.preventDefault(); e.stopPropagation(); onSaveSession?.(); return;
      }
      // Ctrl+K : command palette
      if (e.ctrlKey && !e.shiftKey && !e.altKey && e.key === 'k') {
        e.preventDefault(); e.stopPropagation(); onOpenCommandPalette?.(); return;
      }

      // --- Vim chord handling ---
      if (keybindingsMode === 'vim') {
        const isLeaderCapture = e.target instanceof HTMLElement && e.target.dataset.leaderCapture === 'true';

        if (leaderConfig.isModifierOnly && !isLeaderCapture) {
          if (leaderConfig.match(e)) {
            modTapRef.current = { pressed: true, usedInCombo: false };
            return;
          } else if (modTapRef.current.pressed) {
            modTapRef.current.usedInCombo = true;
          }
        }

        if (chordRef.current.state === 'waiting') {
          if (['Control', 'Shift', 'Alt', 'Meta'].includes(e.key)) return;
          e.preventDefault(); e.stopPropagation();
          cancelChord();
          executeChordAction(e.key);
          return;
        }

        if (!leaderConfig.isModifierOnly && !isLeaderCapture && leaderConfig.match(e)) {
          e.preventDefault(); e.stopPropagation(); startChord(); return;
        }
      }

      // --- Direct shortcuts ---

      // Ctrl+T : new terminal tab
      if (e.ctrlKey && !e.shiftKey && !e.altKey && e.key === 't') {
        e.preventDefault(); e.stopPropagation();
        const newId = addTab('terminal');
        requestAnimationFrame(() => scrollToTab(newId));
        return;
      }
      // Ctrl+B : new browser tab
      if (e.ctrlKey && !e.shiftKey && !e.altKey && e.key === 'b') {
        e.preventDefault(); e.stopPropagation();
        const newId = addTab('browser');
        requestAnimationFrame(() => scrollToTab(newId));
        return;
      }
      // Ctrl+D : split current tab (matches active pane type)
      if (e.ctrlKey && !e.shiftKey && !e.altKey && e.key === 'd') {
        e.preventDefault(); e.stopPropagation();
        if (activeTab) {
          const activePane = activeTab.panes.find(p => p.id === activeTab.activePaneId);
          const splitType = activePane?.type ?? 'terminal';
          splitTab(activeTab.id, splitType, undefined, activePane?.shell, undefined, undefined, activePane?.cwd);
        }
        return;
      }
      // Ctrl+W : close active pane (or tab if single pane)
      if (e.ctrlKey && !e.shiftKey && !e.altKey && e.key === 'w') {
        e.preventDefault(); e.stopPropagation();
        if (activeTab) {
          if (activeTab.panes.length <= 1) {
            removeTab(activeTabId);
          } else {
            removePane(activeTabId, activeTab.activePaneId);
          }
        }
        return;
      }
      // F2 : rename active tab
      if (e.key === 'F2' && !e.ctrlKey && !e.altKey && !e.shiftKey) {
        e.preventDefault(); e.stopPropagation();
        if (onRenameTab) onRenameTab();
        return;
      }
      // Ctrl+/ : help
      if (e.ctrlKey && (e.key === '/' || e.key === '?')) {
        e.preventDefault(); e.stopPropagation(); onToggleHelp(); return;
      }
      // Ctrl+1-9 : jump to tab
      if (e.ctrlKey && !e.shiftKey && !e.altKey) {
        const num = parseInt(e.key, 10);
        if (num >= 1 && num <= 9) {
          e.preventDefault(); e.stopPropagation(); goToTab(num - 1); return;
        }
      }
      // Ctrl+Shift+1-9 : move tab to position
      if (e.ctrlKey && e.shiftKey && !e.altKey) {
        const digitMatch = e.code?.match(/^Digit(\d)$/);
        if (digitMatch) {
          const num = parseInt(digitMatch[1], 10);
          if (num >= 1 && num <= 9) {
            e.preventDefault(); e.stopPropagation();
            moveTab(activeTabId, num - 1);
            return;
          }
        }
      }

      // Alt+Arrow : navigate sub-panes within tab
      if (e.altKey && !e.ctrlKey && !e.shiftKey) {
        if (e.key === 'ArrowLeft') { e.preventDefault(); e.stopPropagation(); navigatePane('left'); return; }
        if (e.key === 'ArrowRight') { e.preventDefault(); e.stopPropagation(); navigatePane('right'); return; }
        if (e.key === 'ArrowUp') { e.preventDefault(); e.stopPropagation(); navigatePane('up'); return; }
        if (e.key === 'ArrowDown') { e.preventDefault(); e.stopPropagation(); navigatePane('down'); return; }
      }

      // Ctrl+Alt+Left/Right : navigate between tabs
      if (e.ctrlKey && e.altKey && !e.shiftKey) {
        if (e.key === 'ArrowLeft') { e.preventDefault(); e.stopPropagation(); goToPrevTab(); return; }
        if (e.key === 'ArrowRight') { e.preventDefault(); e.stopPropagation(); goToNextTab(); return; }
      }
    };

    window.addEventListener('keydown', handler, true);
    window.addEventListener('keyup', handleKeyUp, true);
    return () => {
      window.removeEventListener('keydown', handler, true);
      window.removeEventListener('keyup', handleKeyUp, true);
      cancelChord();
    };
  }, [goToTab, goToPrevTab, goToNextTab, navigatePane, addTab, splitTab, removeTab, removePane, moveTab, tabs, activeTabId, activeTab, scrollToTab, onToggleHelp, onRenameTab, keybindingsMode, leaderKey, onChordStateChange, onOpenSettings, onSaveSession, onOpenCommandPalette]);
}
