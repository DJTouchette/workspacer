import { useEffect, useCallback, useRef } from 'react';
import { PaneType, TabConfig } from '../types/pane';
import { tilingColumns } from '../lib/layoutUtils';
import { setLayerArmed } from '../lib/layerArmed';
import {
  buildChordTree,
  chordNodeAt,
  parseDigitRangeCombo,
  DigitRangeCombo,
  comboHasModifiers,
  comboMatcher,
  DIGIT_RANGE_TOKEN,
  isEditableTarget,
  leaderPassthroughBytes,
  REPEAT_ACTIONS,
  SCOPED_ACTIONS,
} from '../lib/shortcuts';
import type { CommandLayerConfig } from './useConfig';

// The canonical matcher moved to lib/shortcuts.ts (one implementation for the
// window dispatcher, pane listeners, and plugin hotkeys alike); re-exported so
// existing imports keep working.
export { comboMatcher } from '../lib/shortcuts';

const CHORD_TIMEOUT = 1500;

const MODIFIER_KEY_NAMES = new Set(['Control', 'Alt', 'Shift', 'Meta']);

/** A lone-modifier leader (e.g. Linux's single Alt tap, substituted for the
 *  IME-grabbed ctrl+space — see resolveLeader) → the KeyboardEvent.key it fires
 *  on. A lone modifier can't be matched on key-down (that would fire on the Alt
 *  of every Alt+Tab); it arms on key-UP after a clean press/release with no other
 *  key in between, leaving Alt+<key> combos untouched. */
const LONE_MODIFIER_LEADERS: Record<string, string> = {
  alt: 'Alt',
  ctrl: 'Control',
  shift: 'Shift',
  meta: 'Meta',
};

/** True when a modifier other than `selfKey` is held — used to reject Ctrl+Alt
 *  etc. as a lone-Alt tap. */
function otherModifierHeld(e: KeyboardEvent, selfKey: string): boolean {
  return (
    (selfKey !== 'Control' && e.ctrlKey) ||
    (selfKey !== 'Alt' && e.altKey) ||
    (selfKey !== 'Shift' && e.shiftKey) ||
    (selfKey !== 'Meta' && e.metaKey)
  );
}

/** Matchers for direct (non-prefix) bindings only; prefix chords are handled by
 *  the chord tree. */
function buildDirectMatchers(
  shortcuts: Record<string, string>,
): Record<string, (e: KeyboardEvent) => boolean> {
  const out: Record<string, (e: KeyboardEvent) => boolean> = {};
  for (const [action, combo] of Object.entries(shortcuts)) {
    const trimmed = (combo ?? '').trim();
    // Prefix chords go through the chord tree; digit-range bindings (ctrl+1-9)
    // are matched separately since the trailing "1-9" isn't a single key.
    // Surface-scoped actions (fleet-*/inbox-*) are matched by their own
    // overlay's listener — binding bare keys like "j" globally would eat them.
    if (
      !trimmed ||
      /^prefix\s/i.test(trimmed) ||
      parseDigitRangeCombo(trimmed) ||
      SCOPED_ACTIONS.has(action)
    )
      continue;
    out[action] = comboMatcher(trimmed);
  }
  return out;
}

/** Match a digit-range spec against a keydown; returns the pressed digit (1–9)
 *  or null. Uses e.code so Shift-modified digits ("!") still resolve to 1. */
function matchDigitRange(spec: DigitRangeCombo | null, e: KeyboardEvent): number | null {
  if (!spec) return null;
  if (
    e.ctrlKey !== spec.ctrl ||
    e.altKey !== spec.alt ||
    e.shiftKey !== spec.shift ||
    e.metaKey !== spec.meta
  )
    return null;
  const m = e.code?.match(/^Digit([1-9])$/);
  return m ? parseInt(m[1], 10) : null;
}

interface UseKeyboardNavOptions {
  tabs: TabConfig[];
  activeTabId: string;
  activeTab?: TabConfig;
  setActiveTabId: (id: string) => void;
  scrollToTab: (id: string) => void;
  addTab: (
    type: PaneType,
    title?: string,
    shell?: string,
    url?: string,
    appMode?: boolean,
  ) => string;
  splitTab: (
    tabId: string,
    type: PaneType,
    title?: string,
    shell?: string,
    url?: string,
    appMode?: boolean,
    cwd?: string,
  ) => string;
  removeTab: (tabId: string) => void;
  removePane: (tabId: string, paneId: string) => void;
  renameTab: (tabId: string, title: string) => void;
  moveTab: (tabId: string, toIndex: number) => void;
  setActivePane: (tabId: string, paneId: string) => void;
  onToggleHelp: () => void;
  onRenameTab?: () => void;
  /** Workspace prefix combo (e.g. 'ctrl+space'); bindings of the form
   *  'prefix <key> [<key>…]' fire as a (possibly nested) chord after it. */
  prefix?: string;
  /** Reports the live chord path: null when idle, [] at the root after the
   *  prefix, ['t'] inside the Tab submenu, etc. Drives the chord hint. */
  onChordPathChange?: (path: string[] | null) => void;
  onOpenSettings?: () => void;
  onSaveSession?: () => void;
  onOpenCommandPalette?: () => void;
  onOpenSplitPalette?: () => void;
  onOpenFile?: () => void;
  onPrevAgent?: () => void;
  onNextAgent?: () => void;
  onNextAttention?: () => void;
  onSpawnAgent?: () => void;
  onToggleTerminal?: () => void;
  onToggleSidebar?: () => void;
  onToggleInbox?: () => void;
  onToggleFleet?: () => void;
  onToggleUiMode?: () => void;
  /** App-wide text scale nudges (mod+= / mod+- / mod+0). */
  onTextSizeUp?: () => void;
  onTextSizeDown?: () => void;
  onTextSizeReset?: () => void;
  onOpenReview?: () => void;
  /** Library quick-picker (default mod+shift+l). Owned here so the chord tree
   *  and direct matcher share one dispatch path — the old separate
   *  bubble-phase listener had its own matcher that never resolved `mod`,
   *  which left the default binding permanently dead. */
  onLibraryPicker?: () => void;
  /** Inspector rail toggle (default mod+shift+e). The rail is per-pane state,
   *  so App routes this to the active ClaudePane over the `inspector:toggle`
   *  CustomEvent — the pane's old focus-scoped listener had the same dead
   *  never-resolves-`mod` matcher. */
  onToggleInspector?: () => void;
  /** The transient command layer (COMMAND_LAYER.md). Absent/disabled = the
   *  legacy chord behavior, byte for byte. */
  commandLayer?: CommandLayerConfig;
  // ── Command-layer verbs (scope:'layer' — fire from the chord tree only) ──
  onZoomPane?: () => void;
  onSwapPaneLeft?: () => void;
  onSwapPaneRight?: () => void;
  onCyclePane?: () => void;
  onFocusComposer?: () => void;
  onChatScroll?: (kind: 'half-up' | 'half-down' | 'top' | 'bottom') => void;
  onAlternateAgent?: () => void;
  onPinAgent?: () => void;
  /** Jump to pinned agent N (the `prefix 1-9` digit chord step). */
  onJumpPinned?: (slot: number) => void;
  onApproveAttention?: () => void;
  onDenyAttention?: () => void;
  /** Show numbered pane badges; the next digit focuses that pane (App owns
   *  the transient hint state and its own digit listener). */
  onPaneHints?: () => void;
  /** Open the command palette in its `:` cmdline variant (ex verbs). */
  onCmdline?: () => void;
  /** Session jumplist over focused agents (prefix ctrl+o / ctrl+i). */
  onJumpBack?: () => void;
  onJumpForward?: () => void;
  /** The CONFIGURED (unresolved) leader — the passthrough writes the byte this
   *  combo would have sent (ctrl+space → NUL), even where the platform
   *  substitutes the armed key (the Linux Alt tap). Defaults to `prefix`. */
  configuredPrefix?: string;
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
  prefix = 'ctrl+space',
  onChordPathChange,
  onOpenSettings,
  onSaveSession,
  onOpenCommandPalette,
  onOpenSplitPalette,
  onOpenFile,
  onPrevAgent,
  onNextAgent,
  onNextAttention,
  onToggleTerminal,
  onToggleSidebar,
  onToggleInbox,
  onToggleFleet,
  onToggleUiMode,
  onTextSizeUp,
  onTextSizeDown,
  onTextSizeReset,
  onOpenReview,
  onSpawnAgent,
  onLibraryPicker,
  onToggleInspector,
  commandLayer,
  configuredPrefix,
  onZoomPane,
  onSwapPaneLeft,
  onSwapPaneRight,
  onCyclePane,
  onFocusComposer,
  onChatScroll,
  onAlternateAgent,
  onPinAgent,
  onJumpPinned,
  onApproveAttention,
  onDenyAttention,
  onPaneHints,
  onCmdline,
  onJumpBack,
  onJumpForward,
  shortcuts = {},
}: UseKeyboardNavOptions) {
  const directRef = useRef(buildDirectMatchers(shortcuts));
  directRef.current = buildDirectMatchers(shortcuts);
  const treeRef = useRef(buildChordTree(shortcuts));
  treeRef.current = buildChordTree(shortcuts);
  // Parsed digit-range bindings (e.g. ctrl+1-9). Reassigned each render like the
  // matchers above so the handler always sees the current config.
  const numberKeysRef = useRef({
    jump: parseDigitRangeCombo(shortcuts['jump-tab']),
    move: parseDigitRangeCombo(shortcuts['move-tab']),
  });
  numberKeysRef.current = {
    jump: parseDigitRangeCombo(shortcuts['jump-tab']),
    move: parseDigitRangeCombo(shortcuts['move-tab']),
  };
  // path === null → idle; [] → prefix armed (root); ['t'] → inside Tab submenu.
  const chordRef = useRef<{
    path: string[] | null;
    timeoutId: ReturnType<typeof setTimeout> | null;
  }>({
    path: null,
    timeoutId: null,
  });
  // For a lone-modifier leader: true once the modifier is pressed alone and still
  // a candidate for a "tap" (no other key has joined it). Fires on its key-up.
  const tapArmedRef = useRef(false);
  // The element focused when the layer ARMED — the terminal a `prefix prefix`
  // passthrough addresses. Deliberately not a focus park: nothing ever
  // focuses it back (doctrine: no focus parking, see COMMAND_LAYER.md).
  const armSourceRef = useRef<Element | null>(null);
  // True while the layer is re-armed by a repeat-group action rather than an
  // explicit leader press. A focus landing in an editable target cancels THIS
  // window (the composer autofocus beats repeatMs, and stealing typed 'hjkl'
  // as navigation is a mode error) but never an explicit arm.
  const repeatWindowRef = useRef(false);
  // Latest cancelChord, so the chord can be reset from outside the big effect
  // below without that effect's cleanup owning the reset. See the teardown
  // comment where it's assigned.
  const cancelChordRef = useRef<() => void>(() => {});

  // Tab navigation
  const goToTab = useCallback(
    (index: number) => {
      if (index >= 0 && index < tabs.length) {
        const tab = tabs[index];
        setActiveTabId(tab.id);
        scrollToTab(tab.id);
      }
    },
    [tabs, setActiveTabId, scrollToTab],
  );

  const goToPrevTab = useCallback(() => {
    const idx = tabs.findIndex((t) => t.id === activeTabId);
    goToTab(idx > 0 ? idx - 1 : tabs.length - 1);
  }, [tabs, activeTabId, goToTab]);

  const goToNextTab = useCallback(() => {
    const idx = tabs.findIndex((t) => t.id === activeTabId);
    goToTab(idx < tabs.length - 1 ? idx + 1 : 0);
  }, [tabs, activeTabId, goToTab]);

  // Sub-pane navigation within current tab
  const navigatePane = useCallback(
    (direction: 'left' | 'right' | 'up' | 'down') => {
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
    },
    [activeTab, setActivePane],
  );

  useEffect(() => {
    const prefixMatch = comboMatcher(prefix);
    // A modifier-less leader (bare "`" or space) must not arm the chord while the
    // user is typing — it would swallow the keystroke. Modifier leaders (Ctrl+Space)
    // stay live everywhere. See isEditableTarget for what counts as "typing".
    const prefixNeedsEditableGuard = !comboHasModifiers(prefix);
    // A lone-modifier leader (Linux's Alt tap) is matched by tap detection on
    // key-up, not by comboMatcher on key-down; null for ordinary combo leaders.
    const loneModLeaderKey = LONE_MODIFIER_LEADERS[prefix.toLowerCase().trim()] ?? null;
    // The command layer's semantics only apply while enabled; disabled keeps
    // the legacy chord behavior byte for byte.
    const layerEnabled = commandLayer?.enabled === true;
    const layerTimeout = layerEnabled ? (commandLayer?.timeoutMs ?? 0) : CHORD_TIMEOUT;
    const layerRepeatMs = commandLayer?.repeatMs ?? 500;
    const layerPassthrough = layerEnabled && (commandLayer?.passthrough ?? true);
    const passthroughBytes = leaderPassthroughBytes(configuredPrefix ?? prefix);

    const cancelChord = () => {
      if (chordRef.current.timeoutId) clearTimeout(chordRef.current.timeoutId);
      chordRef.current = { path: null, timeoutId: null };
      repeatWindowRef.current = false;
      setLayerArmed(false);
      onChordPathChange?.(null);
    };
    cancelChordRef.current = cancelChord;

    // Enter/move to a chord path (root = []), (re)arming the idle timeout so a
    // half-typed chord doesn't linger forever. With the layer enabled the
    // timeout is configurable and 0 means armed-until-resolved (the armed
    // state is always visible chrome, so there is nothing to expire); a
    // repeat re-arm passes its own shorter window.
    const setChordPath = (path: string[], ttlMs: number = layerTimeout) => {
      if (chordRef.current.timeoutId) clearTimeout(chordRef.current.timeoutId);
      if (chordRef.current.path === null) armSourceRef.current = document.activeElement;
      chordRef.current.path = path;
      chordRef.current.timeoutId = ttlMs > 0 ? setTimeout(cancelChord, ttlMs) : null;
      repeatWindowRef.current = false;
      setLayerArmed(true);
      onChordPathChange?.(path);
    };

    // `prefix prefix` — send the literal leader byte to the terminal the layer
    // was armed FROM (nested tmux). Addressed by the remembered element, not
    // "the focused terminal": arming never moves focus, but a webview arm
    // blurs its guest, so the capture-time element is the only honest address.
    const sendPassthrough = () => {
      const target = armSourceRef.current;
      cancelChord();
      if (!passthroughBytes) return;
      window.dispatchEvent(
        new CustomEvent('terminal:write-prefix', {
          detail: { target, bytes: passthroughBytes },
        }),
      );
    };

    /**
     * Run an action. Returns true if this hook owns and handled it. Actions it
     * doesn't own (library-picker, toggle-inspector — handled by their own
     * focus-scoped listeners) return false so the event isn't consumed here.
     */
    const executeAction = (action: string, digit?: number): boolean => {
      switch (action) {
        case 'new-terminal': {
          const id = addTab('terminal');
          requestAnimationFrame(() => scrollToTab(id));
          return true;
        }
        case 'new-browser': {
          const id = addTab('browser');
          requestAnimationFrame(() => scrollToTab(id));
          return true;
        }
        case 'new-claude': {
          const id = addTab('claude');
          requestAnimationFrame(() => scrollToTab(id));
          return true;
        }
        case 'split':
          onOpenSplitPalette?.();
          return true;
        case 'quick-split': {
          if (activeTab) {
            const activePane = activeTab.panes.find((p) => p.id === activeTab.activePaneId);
            const splitType = activePane?.type ?? 'terminal';
            splitTab(
              activeTab.id,
              splitType,
              undefined,
              activePane?.shell,
              undefined,
              undefined,
              activePane?.cwd,
            );
          }
          return true;
        }
        case 'close-pane': {
          if (activeTab) {
            if (activeTab.panes.length <= 1) removeTab(activeTabId);
            else removePane(activeTabId, activeTab.activePaneId);
          }
          return true;
        }
        case 'rename-tab':
          onRenameTab?.();
          return true;
        case 'toggle-help':
          onToggleHelp();
          return true;
        case 'nav-left':
          navigatePane('left');
          return true;
        case 'nav-right':
          navigatePane('right');
          return true;
        case 'nav-up':
          navigatePane('up');
          return true;
        case 'nav-down':
          navigatePane('down');
          return true;
        case 'prev-tab':
          goToPrevTab();
          return true;
        case 'next-tab':
          goToNextTab();
          return true;
        case 'move-tab-left': {
          const idx = tabs.findIndex((t) => t.id === activeTabId);
          if (idx > 0) moveTab(activeTabId, idx - 1);
          return true;
        }
        case 'move-tab-right': {
          const idx = tabs.findIndex((t) => t.id === activeTabId);
          if (idx >= 0 && idx < tabs.length - 1) moveTab(activeTabId, idx + 1);
          return true;
        }
        case 'open-review':
          onOpenReview?.();
          return true;
        case 'settings':
          onOpenSettings?.();
          return true;
        case 'save-session':
          onSaveSession?.();
          return true;
        case 'command-palette':
          onOpenCommandPalette?.();
          return true;
        case 'open-file':
          onOpenFile?.();
          return true;
        case 'prev-agent':
          onPrevAgent?.();
          return true;
        case 'next-agent':
          onNextAgent?.();
          return true;
        case 'next-attention':
          onNextAttention?.();
          return true;
        case 'spawn-agent':
          onSpawnAgent?.();
          return true;
        case 'toggle-terminal':
          onToggleTerminal?.();
          return true;
        case 'toggle-sidebar':
          onToggleSidebar?.();
          return true;
        case 'toggle-inbox':
          onToggleInbox?.();
          return true;
        case 'toggle-fleet':
          onToggleFleet?.();
          return true;
        case 'toggle-ui-mode':
          onToggleUiMode?.();
          return true;
        case 'text-size-up':
          onTextSizeUp?.();
          return true;
        case 'text-size-down':
          onTextSizeDown?.();
          return true;
        case 'text-size-reset':
          onTextSizeReset?.();
          return true;
        case 'library-picker':
          if (!onLibraryPicker) return false;
          onLibraryPicker();
          return true;
        case 'toggle-inspector':
          if (!onToggleInspector) return false;
          onToggleInspector();
          return true;
        // ── Command-layer verbs ──
        case 'zoom-pane':
          onZoomPane?.();
          return true;
        case 'swap-pane-left':
          onSwapPaneLeft?.();
          return true;
        case 'swap-pane-right':
          onSwapPaneRight?.();
          return true;
        case 'cycle-pane':
          onCyclePane?.();
          return true;
        case 'focus-composer':
          onFocusComposer?.();
          return true;
        case 'chat-scroll-up':
          onChatScroll?.('half-up');
          return true;
        case 'chat-scroll-down':
          onChatScroll?.('half-down');
          return true;
        case 'chat-scroll-top':
          onChatScroll?.('top');
          return true;
        case 'chat-scroll-bottom':
          onChatScroll?.('bottom');
          return true;
        case 'alternate-agent':
          onAlternateAgent?.();
          return true;
        case 'pin-agent':
          onPinAgent?.();
          return true;
        case 'jump-pinned':
          if (digit !== undefined) onJumpPinned?.(digit);
          return true;
        case 'approve-attention':
          onApproveAttention?.();
          return true;
        case 'deny-attention':
          onDenyAttention?.();
          return true;
        case 'pane-hints':
          onPaneHints?.();
          return true;
        case 'cmdline':
          onCmdline?.();
          return true;
        case 'jump-back':
          onJumpBack?.();
          return true;
        case 'jump-forward':
          onJumpForward?.();
          return true;
        default:
          return false; // not owned here
      }
    };

    const handler = (e: KeyboardEvent) => {
      // Don't hijack keys while the settings rebind input is capturing.
      const isCapture =
        e.target instanceof HTMLElement && e.target.dataset.leaderCapture === 'true';
      if (isCapture) return;

      const path = chordRef.current.path;

      // 1. Chord in progress: walk the tree. Groups descend a level; leaves fire.
      if (path !== null) {
        // Leader again at the root → literal passthrough (nested tmux),
        // checked before the modifier wait so Ctrl+Space's held Ctrl doesn't
        // eat it. Only at the root: deeper in a submenu the leader is just an
        // unknown key.
        if (layerPassthrough && path.length === 0 && !loneModLeaderKey && prefixMatch(e)) {
          e.preventDefault();
          e.stopPropagation();
          sendPassthrough();
          return;
        }
        if (MODIFIER_KEY_NAMES.has(e.key)) return; // wait for the real key
        e.preventDefault();
        e.stopPropagation();

        if (e.key === 'Escape') {
          cancelChord();
          return;
        }
        if (e.key === 'Backspace') {
          if (path.length === 0) cancelChord();
          else setChordPath(path.slice(0, -1)); // pop up one submenu
          return;
        }

        const node = chordNodeAt(treeRef.current, path);
        let child = node?.children.find((c) => comboMatcher(c.step)(e));
        // A '1-9' chord step (jump-pinned): any unmodified digit resolves it,
        // carrying the digit to the action. e.code so Shift-symbols still map;
        // numpad digits excluded by the Digit code (documented limitation).
        let stepDigit: number | undefined;
        if (!child && !e.ctrlKey && !e.altKey && !e.metaKey) {
          const m = e.code?.match(/^Digit([1-9])$/);
          if (m) {
            const rangeChild = node?.children.find((c) => c.step === DIGIT_RANGE_TOKEN);
            if (rangeChild) {
              child = rangeChild;
              stepDigit = parseInt(m[1], 10);
            }
          }
        }
        if (!child) {
          cancelChord();
          return;
        } // unknown key cancels (which-key style)

        if (child.node.children.length > 0) {
          setChordPath([...path, child.step]); // descend into submenu
        } else if (child.node.action) {
          const action = child.node.action;
          if (layerEnabled && REPEAT_ACTIONS.has(action)) {
            // Repeat group: fire, then re-arm at the PARENT path for the
            // repeat window so `prefix h h l` walks panes. The strip stays
            // visible for the whole window (path non-null), and a focus
            // landing in an editable target cancels it (focusin listener).
            executeAction(action, stepDigit);
            setChordPath(path, layerRepeatMs);
            repeatWindowRef.current = true;
          } else {
            cancelChord();
            executeAction(action, stepDigit);
          }
        } else {
          cancelChord();
        }
        return;
      }

      // 2. Lone-modifier leader (Alt tap): the modifier alone arms a pending tap;
      //    any OTHER key means it's a combo (Alt+Tab), so cancel and fall through.
      //    The tap itself fires on key-up (see the keyup handler below).
      if (loneModLeaderKey) {
        if (e.key === loneModLeaderKey) {
          tapArmedRef.current = !otherModifierHeld(e, loneModLeaderKey);
          return;
        }
        tapArmedRef.current = false;
      }

      // 3. Combo leader pressed → arm the chord at the root. A bare (modifier-less)
      //    leader is suppressed inside editable contexts so it doesn't eat typing,
      //    and any leader is suppressed inside surfaces that opt out wholesale
      //    (modal dialogs mark their root data-leader-suppress — arming while
      //    typing a spawn prompt would steal the following keystrokes).
      if (!loneModLeaderKey && prefixMatch(e)) {
        if (prefixNeedsEditableGuard && isEditableTarget(e.target)) return;
        if (leaderSuppressed(e.target)) return;
        e.preventDefault();
        e.stopPropagation();
        setChordPath([]);
        return;
      }

      // 4. Direct bindings. Only consume the event for actions we own; let the
      //    rest (e.g. toggle-inspector, library-picker) reach their listeners.
      for (const [action, matcher] of Object.entries(directRef.current)) {
        if (matcher(e)) {
          if (executeAction(action)) {
            e.preventDefault();
            e.stopPropagation();
          }
          return;
        }
      }

      // 5. Digit-range bindings (jump to tab / move tab to slot). Config-driven
      //    via shortcuts['jump-tab'] / ['move-tab'] (defaults Ctrl+1-9 /
      //    Ctrl+Shift+1-9). Exact-modifier match, so the two never collide.
      const { jump, move } = numberKeysRef.current;
      const jumpN = matchDigitRange(jump, e);
      if (jumpN !== null) {
        e.preventDefault();
        e.stopPropagation();
        goToTab(jumpN - 1);
        return;
      }
      const moveN = matchDigitRange(move, e);
      if (moveN !== null) {
        e.preventDefault();
        e.stopPropagation();
        moveTab(activeTabId, moveN - 1);
        return;
      }
    };

    // Lone-modifier leader fires on key-up: a clean tap (armed, nothing else
    // pressed, no chord already open, no other modifier still held) arms the
    // chord root. Inert for ordinary combo leaders (loneModLeaderKey === null).
    // A second clean tap while armed at the root is the passthrough gesture
    // (the Linux twin of Ctrl+Space Ctrl+Space).
    const upHandler = (e: KeyboardEvent) => {
      if (!loneModLeaderKey || e.key !== loneModLeaderKey) return;
      const armed = tapArmedRef.current;
      tapArmedRef.current = false;
      if (!armed || otherModifierHeld(e, loneModLeaderKey)) return;
      if (chordRef.current.path !== null) {
        if (layerPassthrough && chordRef.current.path.length === 0) {
          e.preventDefault();
          e.stopPropagation();
          sendPassthrough();
        }
        return;
      }
      if (leaderSuppressed(e.target)) return;
      e.preventDefault();
      e.stopPropagation();
      setChordPath([]);
    };

    // Surfaces that opt out of leader arming wholesale (modal dialogs).
    const leaderSuppressed = (target: EventTarget | null): boolean =>
      target instanceof HTMLElement && !!target.closest('[data-leader-suppress]');

    // The mouse never fights the layer: any click disarms. Window blur too —
    // an armed layer must not survive an Alt-Tab away and eat the first
    // keystroke on return.
    const disarmOnMouse = () => {
      if (chordRef.current.path !== null) cancelChord();
    };
    const disarmOnBlur = () => {
      tapArmedRef.current = false;
      if (chordRef.current.path !== null) cancelChord();
    };
    // A repeat window ends the moment focus lands somewhere the user types —
    // the composer autofocus (a ~15-frame retry) beats repeatMs, and stealing
    // a typed 'hjkl' as pane navigation is exactly the mode-error class the
    // layer exists to avoid. Explicit arms are untouched: the user pressed
    // the leader on purpose, wherever focus sits.
    const cancelRepeatOnEditableFocus = (e: FocusEvent) => {
      if (!repeatWindowRef.current) return;
      if (isEditableTarget(e.target)) cancelChord();
    };

    // The `command:action` door: the `:` cmdline (and any future bus parity
    // hook) executes actions through the SAME switch the chords use, so the
    // two surfaces cannot disagree about what a verb does.
    const onCommandAction = (e: Event) => {
      const action = (e as CustomEvent).detail?.action;
      if (typeof action === 'string') executeAction(action);
    };

    window.addEventListener('keydown', handler, true);
    window.addEventListener('keyup', upHandler, true);
    window.addEventListener('command:action', onCommandAction);
    window.addEventListener('mousedown', disarmOnMouse, true);
    window.addEventListener('blur', disarmOnBlur);
    window.addEventListener('focusin', cancelRepeatOnEditableFocus);
    return () => {
      window.removeEventListener('keydown', handler, true);
      window.removeEventListener('keyup', upHandler, true);
      window.removeEventListener('command:action', onCommandAction);
      window.removeEventListener('mousedown', disarmOnMouse, true);
      window.removeEventListener('blur', disarmOnBlur);
      window.removeEventListener('focusin', cancelRepeatOnEditableFocus);
      // Deliberately NOT cancelChord() — see the effects below. This effect
      // re-runs whenever any of its ~30 callback deps changes identity, which
      // in practice is constantly; cancelling here disarmed a chord the instant
      // anything else in the app updated.
    };
  }, [
    goToTab,
    goToPrevTab,
    goToNextTab,
    navigatePane,
    addTab,
    splitTab,
    removeTab,
    removePane,
    moveTab,
    tabs,
    activeTabId,
    activeTab,
    scrollToTab,
    onToggleHelp,
    onRenameTab,
    prefix,
    onChordPathChange,
    onOpenSettings,
    onSaveSession,
    onOpenCommandPalette,
    onOpenSplitPalette,
    onOpenFile,
    onPrevAgent,
    onNextAgent,
    onNextAttention,
    onSpawnAgent,
    onToggleTerminal,
    onToggleSidebar,
    onToggleInbox,
    onToggleFleet,
    onToggleUiMode,
    onTextSizeUp,
    onTextSizeDown,
    onTextSizeReset,
    onOpenReview,
    onLibraryPicker,
    onToggleInspector,
    commandLayer,
    configuredPrefix,
    onZoomPane,
    onSwapPaneLeft,
    onSwapPaneRight,
    onCyclePane,
    onFocusComposer,
    onChatScroll,
    onAlternateAgent,
    onPinAgent,
    onJumpPinned,
    onApproveAttention,
    onDenyAttention,
    onPaneHints,
    onCmdline,
    onJumpBack,
    onJumpForward,
  ]);

  // A half-typed chord survives the key-handler effect re-subscribing, and is
  // only reset for the two reasons a user would expect: the hook going away,
  // and the leader itself being rebound. Anything else — a session snapshot
  // landing, a tab list changing — has nothing to do with what the user is
  // mid-way through typing.
  //
  // Safe because the state involved outlives the effect: chordRef is a ref, and
  // onChordPathChange is a setState, so the pending CHORD_TIMEOUT still
  // resolves against live state after a re-subscribe.
  useEffect(() => () => cancelChordRef.current(), []);
  useEffect(() => {
    cancelChordRef.current();
  }, [prefix]);
}
