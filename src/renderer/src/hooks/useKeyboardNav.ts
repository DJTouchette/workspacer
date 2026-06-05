import { useEffect, useCallback, useRef } from 'react';
import { PaneType, TabConfig } from '../types/pane';
import { tilingColumns } from '../lib/layoutUtils';

const CHORD_TIMEOUT = 500;

type ChordState = 'idle' | 'waiting';

const MODIFIER_NAMES = new Set(['ctrl', 'alt', 'shift', 'meta']);
const KEY_TO_CODE: Record<string, string> = { space: 'Space', '`': 'Backquote' };

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

/** Pre-parsed shortcut matchers, keyed by action name. */
function buildShortcutMatchers(shortcuts: Record<string, string>): Record<string, (e: KeyboardEvent) => boolean> {
  const matchers: Record<string, (e: KeyboardEvent) => boolean> = {};
  for (const [action, combo] of Object.entries(shortcuts)) {
    matchers[action] = parseKeyCombo(combo).match;
  }
  return matchers;
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
  onOpenSplitPalette?: () => void;
  onPrevAgent?: () => void;
  onNextAgent?: () => void;
  onNextAttention?: () => void;
  onSpawnAgent?: () => void;
  onToggleTerminal?: () => void;
  onToggleSidebar?: () => void;
  shortcuts?: Record<string, string>;
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
  onOpenSplitPalette,
  onPrevAgent,
  onNextAgent,
  onNextAttention,
  onToggleTerminal,
  onToggleSidebar,
  onSpawnAgent,
  shortcuts = {},
}: UseKeyboardNavOptions) {
  const matchersRef = useRef(buildShortcutMatchers(shortcuts));
  // Update matchers when shortcuts change
  matchersRef.current = buildShortcutMatchers(shortcuts);
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
    const cols = tilingColumns(count);
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
      // Agents (vertical sidebar): k/j move up/down, a spawns, m → next that
      // needs me (approval / input).
      else if (key === 'k') onPrevAgent?.();
      else if (key === 'j') onNextAgent?.();
      else if (key === 'm') onNextAttention?.();
      else if (key === 'a') onSpawnAgent?.();
      else if (key === 'r') onRenameTab?.();
      else if (key === '?') onToggleHelp();
      else if (key === 's') onSaveSession?.();
      else if (key === 'd') {
        onOpenSplitPalette?.();
      }
      else if (key === 'D') {
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
      const m = matchersRef.current;

      // --- Global shortcuts (both modes) ---
      if (m['settings']?.(e)) { e.preventDefault(); e.stopPropagation(); onOpenSettings?.(); return; }
      if (m['save-session']?.(e)) { e.preventDefault(); e.stopPropagation(); onSaveSession?.(); return; }
      if (m['command-palette']?.(e)) { e.preventDefault(); e.stopPropagation(); onOpenCommandPalette?.(); return; }
      if (m['prev-agent']?.(e)) { e.preventDefault(); e.stopPropagation(); onPrevAgent?.(); return; }
      if (m['next-agent']?.(e)) { e.preventDefault(); e.stopPropagation(); onNextAgent?.(); return; }
      if (m['next-attention']?.(e)) { e.preventDefault(); e.stopPropagation(); onNextAttention?.(); return; }
      if (m['spawn-agent']?.(e)) { e.preventDefault(); e.stopPropagation(); onSpawnAgent?.(); return; }
      if (m['toggle-terminal']?.(e)) { e.preventDefault(); e.stopPropagation(); onToggleTerminal?.(); return; }
      if (m['toggle-sidebar']?.(e)) { e.preventDefault(); e.stopPropagation(); onToggleSidebar?.(); return; }

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

      // --- Config-driven shortcuts ---
      if (m['new-terminal']?.(e)) {
        e.preventDefault(); e.stopPropagation();
        const newId = addTab('terminal');
        requestAnimationFrame(() => scrollToTab(newId));
        return;
      }
      if (m['new-browser']?.(e)) {
        e.preventDefault(); e.stopPropagation();
        const newId = addTab('browser');
        requestAnimationFrame(() => scrollToTab(newId));
        return;
      }
      if (m['new-claude']?.(e)) {
        e.preventDefault(); e.stopPropagation();
        const newId = addTab('claude');
        requestAnimationFrame(() => scrollToTab(newId));
        return;
      }
      if (m['split']?.(e)) {
        e.preventDefault(); e.stopPropagation();
        onOpenSplitPalette?.();
        return;
      }
      if (m['quick-split']?.(e)) {
        e.preventDefault(); e.stopPropagation();
        if (activeTab) {
          const activePane = activeTab.panes.find(p => p.id === activeTab.activePaneId);
          const splitType = activePane?.type ?? 'terminal';
          splitTab(activeTab.id, splitType, undefined, activePane?.shell, undefined, undefined, activePane?.cwd);
        }
        return;
      }
      if (m['close-pane']?.(e)) {
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
      if (m['rename-tab']?.(e)) {
        e.preventDefault(); e.stopPropagation();
        if (onRenameTab) onRenameTab();
        return;
      }
      if (m['toggle-help']?.(e)) {
        e.preventDefault(); e.stopPropagation(); onToggleHelp(); return;
      }
      if (m['nav-left']?.(e)) { e.preventDefault(); e.stopPropagation(); navigatePane('left'); return; }
      if (m['nav-right']?.(e)) { e.preventDefault(); e.stopPropagation(); navigatePane('right'); return; }
      if (m['nav-up']?.(e)) { e.preventDefault(); e.stopPropagation(); navigatePane('up'); return; }
      if (m['nav-down']?.(e)) { e.preventDefault(); e.stopPropagation(); navigatePane('down'); return; }
      if (m['prev-tab']?.(e)) { e.preventDefault(); e.stopPropagation(); goToPrevTab(); return; }
      if (m['next-tab']?.(e)) { e.preventDefault(); e.stopPropagation(); goToNextTab(); return; }

      // Ctrl+1-9 : jump to tab (always hardcoded — too dynamic to configure)
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
    };

    window.addEventListener('keydown', handler, true);
    window.addEventListener('keyup', handleKeyUp, true);
    return () => {
      window.removeEventListener('keydown', handler, true);
      window.removeEventListener('keyup', handleKeyUp, true);
      cancelChord();
    };
  }, [goToTab, goToPrevTab, goToNextTab, navigatePane, addTab, splitTab, removeTab, removePane, moveTab, tabs, activeTabId, activeTab, scrollToTab, onToggleHelp, onRenameTab, keybindingsMode, leaderKey, onChordStateChange, onOpenSettings, onSaveSession, onOpenCommandPalette, onOpenSplitPalette, onPrevAgent, onNextAgent, onNextAttention, onSpawnAgent]);
}
