/**
 * Shared keybinding display helpers. Keep the formatting in one place so the
 * help overlay, command palette, and tooltips all render shortcuts the same
 * way (e.g. "ctrl+shift+a" → "Ctrl+Shift+A").
 */

const PART_DISPLAY: Record<string, string> = {
  ctrl: 'Ctrl',
  alt: 'Alt',
  shift: 'Shift',
  meta: 'Cmd',
  '`': '`',
};

/** True on macOS, where the `mod` token means Cmd (⌘) rather than Ctrl. Guards
 *  for non-browser contexts (tests) where `navigator` may be absent. */
export function isMacPlatform(): boolean {
  if (typeof navigator === 'undefined') return false;
  const p = (navigator.platform || navigator.userAgent || '').toUpperCase();
  return p.includes('MAC');
}

/** True on Linux. Matters for the chord leader: the historical `ctrl+space`
 *  default is the fcitx/ibus input-method toggle (their built-in trigger key),
 *  so it's swallowed by the IME before it ever reaches the app. navigator-guarded
 *  for non-browser contexts (tests). */
export function isLinuxPlatform(): boolean {
  if (typeof navigator === 'undefined') return false;
  const p = (navigator.platform || navigator.userAgent || '').toUpperCase();
  return p.includes('LINUX') && !p.includes('MAC');
}

/** Resolve the workspace leader to what actually works on this platform — the
 *  same use-time substitution `resolveMod` does for the `mod` token, so nothing
 *  needs migrating on disk. On Linux, `ctrl+space` can never reach the app
 *  (fcitx/ibus grab it as their input-method toggle), so it's substituted with a
 *  single Alt tap — a lone modifier the chord handler arms on key-up (see
 *  useKeyboardNav). Every other leader, including a user's own rebind, passes
 *  through untouched. Pure: pass `isLinux` explicitly in tests. */
export function resolveLeader(prefix: string, isLinux: boolean = isLinuxPlatform()): string {
  if (isLinux && (prefix ?? '').trim().toLowerCase() === 'ctrl+space') return 'alt';
  return prefix;
}

/** Expand the platform-neutral `mod` token to the concrete primary modifier —
 *  `meta` (Cmd) on macOS, `ctrl` everywhere else — so a preset can ship one
 *  binding that feels native on every OS. Pure: pass `isMac` explicitly in
 *  tests. Combos without `mod` (incl. `prefix …` chords) pass through unchanged. */
export function resolveMod(combo: string, isMac: boolean = isMacPlatform()): string {
  if (!combo.includes('mod')) return combo;
  return combo
    .split('+')
    .map((p) => (p.toLowerCase() === 'mod' ? (isMac ? 'meta' : 'ctrl') : p))
    .join('+');
}

/** True if the combo carries any modifier (ctrl/alt/shift/meta). A modifier-less
 *  combo (e.g. a bare "`" or "space") is only safe as a global binding when it's
 *  guarded against editable contexts — see isEditableTarget. */
export function comboHasModifiers(combo: string): boolean {
  const parts = (combo ?? '').toLowerCase().trim().split('+');
  return parts.some(
    (p) => p === 'ctrl' || p === 'alt' || p === 'shift' || p === 'meta' || p === 'mod',
  );
}

/** True when the event target is a place the user is actively typing: a form
 *  input, textarea, contenteditable, or an xterm terminal pane (xterm focuses a
 *  .xterm-helper-textarea inside its .xterm container). Used to hold back
 *  modifier-less global bindings so a bare leader key doesn't steal keystrokes. */
export function isEditableTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  const tag = target.tagName;
  if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT') return true;
  if (target.isContentEditable) return true;
  if (target.closest('.xterm')) return true;
  return false;
}

/** "ctrl+shift+a" → "Ctrl+Shift+A". */
export function formatCombo(combo: string): string {
  return combo
    .split('+')
    .map((p) =>
      p === 'mod'
        ? isMacPlatform()
          ? 'Cmd'
          : 'Ctrl'
        : (PART_DISPLAY[p] ??
          (p.length === 1 ? p.toUpperCase() : p.charAt(0).toUpperCase() + p.slice(1))),
    )
    .join('+');
}

/**
 * Format a binding for display, expanding prefix chords. A direct combo renders
 * as "Ctrl+Shift+P"; a chord ("prefix n") renders as "<prefix> N" using the
 * configured prefix (e.g. "Ctrl+Space N").
 */
export function formatBinding(combo: string, prefix?: string): string {
  const trimmed = combo.trim();
  const m = /^prefix\s+(.+)$/i.exec(trimmed);
  if (m) {
    const pfx = prefix ? formatCombo(prefix) : 'Prefix';
    // A chord may be a multi-step sequence ("t w") → "Ctrl+Space T W".
    const steps = m[1].trim().split(/\s+/).map(formatCombo).join(' ');
    return `${pfx} ${steps}`;
  }
  return formatCombo(combo);
}

/**
 * Resolve the human-readable shortcut for an action from the (already merged
 * with defaults) shortcuts map. Returns undefined when the action has no
 * binding, so callers can omit the badge entirely.
 */
/** Metadata for one bindable action. The single source of truth: the chord
 *  hint, help overlay, and settings editor all derive their labels, grouping,
 *  and ordering from this — add an action here and it shows up everywhere. */
export interface ActionMeta {
  /** Action id, matching the keys of the shortcuts config map. */
  action: string;
  /** Short, human label. */
  label: string;
  /** Grouping bucket for the help overlay and settings editor. */
  section: string;
  /** Bound to a digit RANGE (1-9) rather than a single key — e.g. Ctrl+1-9.
   *  These live outside the chord tree and direct-matcher map (see below). */
  digitRange?: boolean;
  /** Set when the binding only applies inside one surface (the Fleet Deck or
   *  the Inbox drawer) or one mode (the command layer). Scoped actions are
   *  matched by their own dispatch path — fleet/inbox by the open overlay's
   *  listener, 'layer' by the chord tree only while commandLayer.enabled — so
   *  the global direct matcher skips them, and the PRESET machinery never
   *  persists them (they are constant across presets, like fleet-*). */
  scope?: 'fleet' | 'inbox' | 'layer';
}

/** The canonical action list, in display order, grouped by section. */
export const ACTION_REGISTRY: ActionMeta[] = [
  // Agents
  { action: 'prev-agent', label: 'Previous agent', section: 'Agents' },
  { action: 'next-agent', label: 'Next agent', section: 'Agents' },
  { action: 'next-attention', label: 'Agent waiting on you', section: 'Agents' },
  { action: 'spawn-agent', label: 'Dispatch agent', section: 'Agents' },
  // Navigation
  { action: 'jump-tab', label: 'Jump to tab', section: 'Navigation', digitRange: true },
  { action: 'move-tab', label: 'Move tab to slot', section: 'Navigation', digitRange: true },
  { action: 'prev-tab', label: 'Previous tab', section: 'Navigation' },
  { action: 'next-tab', label: 'Next tab', section: 'Navigation' },
  { action: 'move-tab-left', label: 'Move tab left', section: 'Navigation' },
  { action: 'move-tab-right', label: 'Move tab right', section: 'Navigation' },
  { action: 'nav-left', label: 'Focus pane left', section: 'Navigation' },
  { action: 'nav-right', label: 'Focus pane right', section: 'Navigation' },
  { action: 'nav-up', label: 'Focus pane up', section: 'Navigation' },
  { action: 'nav-down', label: 'Focus pane down', section: 'Navigation' },
  // Tabs & Panes
  { action: 'new-terminal', label: 'New terminal', section: 'Tabs & Panes' },
  { action: 'new-browser', label: 'New browser', section: 'Tabs & Panes' },
  { action: 'new-claude', label: 'New Claude', section: 'Tabs & Panes' },
  { action: 'split', label: 'Split pane', section: 'Tabs & Panes' },
  { action: 'quick-split', label: 'Quick split', section: 'Tabs & Panes' },
  { action: 'close-pane', label: 'Close pane', section: 'Tabs & Panes' },
  { action: 'open-file', label: 'Open file', section: 'Tabs & Panes' },
  { action: 'open-review', label: 'Review changes', section: 'Tabs & Panes' },
  { action: 'rename-tab', label: 'Rename tab', section: 'Tabs & Panes' },
  // Panels & Overlays
  { action: 'toggle-sidebar', label: 'Toggle sidebar', section: 'Panels & Overlays' },
  { action: 'toggle-terminal', label: 'Toggle terminal', section: 'Panels & Overlays' },
  // DEPRECATED surface: the sidebar live-feed cards supersede the Inbox drawer
  // (user decision 2026-08-20, COMMAND_LAYER.md). Binding + drawer stay
  // functional until the cards reach full parity; nothing new references it.
  { action: 'toggle-inbox', label: 'Toggle inbox (deprecated)', section: 'Panels & Overlays' },
  { action: 'toggle-fleet', label: 'Toggle fleet', section: 'Panels & Overlays' },
  { action: 'toggle-ui-mode', label: 'Toggle focus / full mode', section: 'Panels & Overlays' },
  { action: 'toggle-inspector', label: 'Toggle inspector', section: 'Panels & Overlays' },
  { action: 'library-picker', label: 'Library picker', section: 'Panels & Overlays' },
  // Tools
  { action: 'command-palette', label: 'Command palette', section: 'Tools' },
  { action: 'save-session', label: 'Save session', section: 'Tools' },
  { action: 'settings', label: 'Settings', section: 'Tools' },
  { action: 'toggle-help', label: 'Toggle help', section: 'Tools' },
  { action: 'text-size-up', label: 'Text size up', section: 'Tools' },
  { action: 'text-size-down', label: 'Text size down', section: 'Tools' },
  { action: 'text-size-reset', label: 'Reset text size', section: 'Tools' },
  // Fleet (active only while the deck is open). Movement is bound per
  // fleet view — the Cards grid navigates spatially, the List linearly — so
  // each has its own remappable set; actions on the selected agent are shared.
  { action: 'fleet-open', label: 'Open selected agent', section: 'Fleet', scope: 'fleet' },
  { action: 'fleet-approve-yes', label: 'Approve', section: 'Fleet', scope: 'fleet' },
  { action: 'fleet-approve-no', label: 'Deny', section: 'Fleet', scope: 'fleet' },
  {
    action: 'fleet-answer',
    label: 'Answer question (option)',
    section: 'Fleet',
    scope: 'fleet',
    digitRange: true,
  },
  {
    action: 'fleet-cards-left',
    label: 'Select card left',
    section: 'Fleet · Cards view',
    scope: 'fleet',
  },
  {
    action: 'fleet-cards-down',
    label: 'Select card below',
    section: 'Fleet · Cards view',
    scope: 'fleet',
  },
  {
    action: 'fleet-cards-up',
    label: 'Select card above',
    section: 'Fleet · Cards view',
    scope: 'fleet',
  },
  {
    action: 'fleet-cards-right',
    label: 'Select card right',
    section: 'Fleet · Cards view',
    scope: 'fleet',
  },
  {
    action: 'fleet-list-down',
    label: 'Select next row',
    section: 'Fleet · List view',
    scope: 'fleet',
  },
  {
    action: 'fleet-list-up',
    label: 'Select previous row',
    section: 'Fleet · List view',
    scope: 'fleet',
  },
  // Command layer (COMMAND_LAYER.md) — chord-only verbs, live while
  // keybindings.commandLayer.enabled. Constant across presets (scope dodge).
  { action: 'zoom-pane', label: 'Zoom pane', section: 'Command layer', scope: 'layer' },
  { action: 'swap-pane-left', label: 'Swap pane left', section: 'Command layer', scope: 'layer' },
  {
    action: 'swap-pane-right',
    label: 'Swap pane right',
    section: 'Command layer',
    scope: 'layer',
  },
  { action: 'cycle-pane', label: 'Next pane', section: 'Command layer', scope: 'layer' },
  { action: 'focus-composer', label: 'Focus composer', section: 'Command layer', scope: 'layer' },
  {
    action: 'chat-scroll-up',
    label: 'Chat half-page up',
    section: 'Command layer',
    scope: 'layer',
  },
  {
    action: 'chat-scroll-down',
    label: 'Chat half-page down',
    section: 'Command layer',
    scope: 'layer',
  },
  { action: 'chat-scroll-top', label: 'Chat to top', section: 'Command layer', scope: 'layer' },
  {
    action: 'chat-scroll-bottom',
    label: 'Chat to bottom',
    section: 'Command layer',
    scope: 'layer',
  },
  {
    action: 'alternate-agent',
    label: 'Alternate agent',
    section: 'Command layer',
    scope: 'layer',
  },
  { action: 'pin-agent', label: 'Pin / unpin agent', section: 'Command layer', scope: 'layer' },
  {
    action: 'jump-pinned',
    label: 'Jump to pinned agent',
    section: 'Command layer',
    scope: 'layer',
    digitRange: true,
  },
  {
    action: 'pane-hints',
    label: 'Pane hints (jump by number)',
    section: 'Command layer',
    scope: 'layer',
  },
  {
    action: 'go-overview',
    label: 'Go to Overview',
    section: 'Command layer',
    scope: 'layer',
  },
  {
    action: 'cmdline',
    label: 'Command line (ex verbs)',
    section: 'Command layer',
    scope: 'layer',
  },
  {
    action: 'jump-back',
    label: 'Jumplist back (agents)',
    section: 'Command layer',
    scope: 'layer',
  },
  {
    action: 'jump-forward',
    label: 'Jumplist forward (agents)',
    section: 'Command layer',
    scope: 'layer',
  },
  {
    action: 'approve-attention',
    label: 'Approve (active agent)',
    section: 'Command layer',
    scope: 'layer',
  },
  {
    action: 'deny-attention',
    label: 'Deny (active agent)',
    section: 'Command layer',
    scope: 'layer',
  },
  // Inbox (active only while the drawer is open)
  {
    action: 'inbox-move-down',
    label: 'Select next item',
    section: 'Inbox · deprecated',
    scope: 'inbox',
  },
  {
    action: 'inbox-move-up',
    label: 'Select previous item',
    section: 'Inbox · deprecated',
    scope: 'inbox',
  },
  { action: 'inbox-open', label: 'Open agent', section: 'Inbox · deprecated', scope: 'inbox' },
  { action: 'inbox-approve-yes', label: 'Approve', section: 'Inbox · deprecated', scope: 'inbox' },
  { action: 'inbox-approve-no', label: 'Deny', section: 'Inbox · deprecated', scope: 'inbox' },
  {
    action: 'inbox-answer',
    label: 'Answer question (option)',
    section: 'Inbox',
    scope: 'inbox',
    digitRange: true,
  },
  { action: 'inbox-dismiss', label: 'Dismiss item', section: 'Inbox · deprecated', scope: 'inbox' },
  { action: 'inbox-snooze', label: 'Snooze item', section: 'Inbox · deprecated', scope: 'inbox' },
  {
    action: 'inbox-clear-reviewed',
    label: 'Clear all reviewed',
    section: 'Inbox · deprecated',
    scope: 'inbox',
  },
];

/** Action ids that only bind inside their own surface (fleet/inbox) or mode
 *  (layer); the global direct-binding matcher must skip these. */
export const SCOPED_ACTIONS = new Set(ACTION_REGISTRY.filter((a) => a.scope).map((a) => a.action));

/** Command-layer verb ids — their chords merge into the tree ONLY while
 *  keybindings.commandLayer.enabled (App strips them otherwise), so persisted
 *  user/preset chords always win over layer defaults. */
export const LAYER_ACTIONS = new Set(
  ACTION_REGISTRY.filter((a) => a.scope === 'layer').map((a) => a.action),
);

/** action id → label, derived from the registry. */
export const ACTION_LABELS: Record<string, string> = Object.fromEntries(
  ACTION_REGISTRY.map((a) => [a.action, a.label]),
);

/** The registry grouped into sections, preserving registry order. Drives the
 *  help overlay and the settings editor. */
export const ACTION_SECTIONS: { section: string; items: ActionMeta[] }[] = (() => {
  const order: string[] = [];
  const bySection = new Map<string, ActionMeta[]>();
  for (const a of ACTION_REGISTRY) {
    if (!bySection.has(a.section)) {
      bySection.set(a.section, []);
      order.push(a.section);
    }
    bySection.get(a.section)!.push(a);
  }
  return order.map((section) => ({ section, items: bySection.get(section)! }));
})();

/** The token marking a digit-range binding ("ctrl+1-9" → Ctrl plus any of 1–9). */
export const DIGIT_RANGE_TOKEN = '1-9';

/** Action ids bound to a digit range rather than a single key. */
export const DIGIT_RANGE_ACTIONS = new Set(
  ACTION_REGISTRY.filter((a) => a.digitRange).map((a) => a.action),
);

export interface DigitRangeCombo {
  ctrl: boolean;
  alt: boolean;
  shift: boolean;
  meta: boolean;
}

/** Parse "ctrl+shift+1-9" into its modifier flags; null if it isn't a
 *  digit-range combo. The pressed digit (1–9) is supplied at match time. */
export function parseDigitRangeCombo(combo: string | undefined): DigitRangeCombo | null {
  const parts = resolveMod((combo ?? '').toLowerCase().trim()).split('+');
  if (parts[parts.length - 1] !== DIGIT_RANGE_TOKEN) return null;
  return {
    ctrl: parts.includes('ctrl'),
    alt: parts.includes('alt'),
    shift: parts.includes('shift'),
    meta: parts.includes('meta'),
  };
}

/** Labels for chord group nodes, keyed by the space-joined step path (e.g. 't'
 *  → 'Tab'). Falls back to the raw key when a group has no label. */
export const CHORD_GROUP_LABELS: Record<string, string> = {
  n: 'New',
  t: 'Tab',
  p: 'Pane',
  // Groups used by the Vim preset's which-key submenus.
  w: 'Window',
  a: 'Agent',
  // The command layer's chat-motion group (g g / g …).
  g: 'Chat scroll',
};

export interface ChordTreeNode {
  /** Set on leaves: the action fired when this node is reached. */
  action?: string;
  children: { step: string; node: ChordTreeNode }[];
}

/**
 * Build the chord tree from resolved shortcuts. Each `prefix a b c` binding adds
 * a path a→b→c; intermediate nodes are groups (submenus), the final node is a
 * leaf carrying the action. Single-step chords ("prefix n") are just depth-1
 * leaves, so flat and grouped bindings coexist.
 */
export function buildChordTree(shortcuts: Record<string, string>): ChordTreeNode {
  const root: ChordTreeNode = { children: [] };
  for (const [action, combo] of Object.entries(shortcuts)) {
    const m = /^prefix\s+(.+)$/i.exec((combo ?? '').trim());
    if (!m) continue;
    const steps = m[1].trim().split(/\s+/);
    let node = root;
    for (const step of steps) {
      let child = node.children.find((c) => c.step.toLowerCase() === step.toLowerCase());
      if (!child) {
        child = { step, node: { children: [] } };
        node.children.push(child);
      }
      node = child.node;
    }
    node.action = action;
  }
  return root;
}

/** Walk the tree along a path of step strings; null if the path is invalid. */
export function chordNodeAt(root: ChordTreeNode, path: string[]): ChordTreeNode | null {
  let node: ChordTreeNode | undefined = root;
  for (const step of path) {
    node = node!.children.find((c) => c.step.toLowerCase() === step.toLowerCase())?.node;
    if (!node) return null;
  }
  return node ?? null;
}

export interface ChordMenuItem {
  step: string;
  keyLabel: string;
  label: string;
  isGroup: boolean;
}

/** The selectable items at `path`: groups first (with submenu indicator), then
 *  actions, each sorted by key. */
export function chordMenu(
  root: ChordTreeNode,
  path: string[],
  groupLabels: Record<string, string> = CHORD_GROUP_LABELS,
): ChordMenuItem[] {
  const node = chordNodeAt(root, path);
  if (!node) return [];
  return node.children
    .map((c) => {
      const isGroup = c.node.children.length > 0;
      const fullKey = [...path, c.step].join(' ');
      const label = isGroup
        ? (groupLabels[fullKey] ?? groupLabels[c.step] ?? formatCombo(c.step))
        : (ACTION_LABELS[c.node.action ?? ''] ?? c.node.action ?? formatCombo(c.step));
      return { step: c.step, keyLabel: formatCombo(c.step), label, isGroup };
    })
    .sort((a, b) =>
      a.isGroup === b.isGroup
        ? a.keyLabel.localeCompare(b.keyLabel, undefined, { sensitivity: 'base' })
        : a.isGroup
          ? -1
          : 1,
    );
}

/** Human breadcrumb for the current chord path, e.g. ['Tab']. */
export function chordBreadcrumb(
  path: string[],
  groupLabels: Record<string, string> = CHORD_GROUP_LABELS,
): string[] {
  return path.map((step, i) => {
    const fullKey = path.slice(0, i + 1).join(' ');
    return groupLabels[fullKey] ?? groupLabels[step] ?? formatCombo(step);
  });
}

/** Combo keys whose KeyboardEvent.key is ambiguous — matched on e.code instead
 *  (space so Shift+Space still reads as space; backquote for layouts where the
 *  key produces a dead key). */
const KEY_TO_CODE: Record<string, string> = { space: 'Space', '`': 'Backquote' };

/** Build a predicate matching a single keydown against a combo like "ctrl+shift+p".
 *  Resolves the `mod` token (Cmd on macOS / Ctrl elsewhere) FIRST — without this a
 *  stored `mod+shift+n` parses with needsCtrl=false and silently listens for the
 *  bare Shift+N instead of Ctrl+Shift+N (the display layer resolves `mod`, so the
 *  UI would advertise Ctrl+Shift+N while nothing fired). THE canonical matcher:
 *  every inline copy of this logic that skipped resolveMod has been a dead
 *  binding in production (library-picker, toggle-inspector). */
export function comboMatcher(combo: string): (e: KeyboardEvent) => boolean {
  const parts = resolveMod(combo.toLowerCase().trim()).split('+');
  const key = parts[parts.length - 1];
  const needsCtrl = parts.includes('ctrl');
  const needsAlt = parts.includes('alt');
  const needsShift = parts.includes('shift');
  const needsMeta = parts.includes('meta');
  const expectedCode = KEY_TO_CODE[key];
  // A punctuation key that needs Shift to TYPE ('<', '?', '{', …) arrives with
  // shiftKey=true and the character already encodes it — demanding "no shift"
  // made every such binding dead (the vim preset's `prefix <` / `>` never
  // fired). The character is the disambiguator, so shift is ignored for
  // symbol keys unless the combo names it explicitly. Letters/digits keep the
  // exact check ('k' must not fire on Shift+K — case pairs are distinct steps).
  const shiftAgnostic = !needsShift && key.length === 1 && !/^[a-z0-9]$/.test(key);
  return (e) => {
    const keyMatch = expectedCode
      ? e.code === expectedCode
      : (e.key === ' ' ? 'space' : e.key.toLowerCase()) === key;
    return (
      keyMatch &&
      e.ctrlKey === needsCtrl &&
      e.altKey === needsAlt &&
      (shiftAgnostic || e.shiftKey === needsShift) &&
      e.metaKey === needsMeta
    );
  };
}

/** True when a keydown matches a direct combo like "shift+e", "ctrl+j", or a
 *  bare "j". Modifiers must match exactly (so "j" doesn't fire on Ctrl+J).
 *  Prefix chords and digit-range combos never match here. Delegates to
 *  comboMatcher so there is exactly one matching implementation to drift. */
export function eventMatchesCombo(e: KeyboardEvent, combo: string | undefined): boolean {
  const trimmed = (combo ?? '').trim();
  if (!trimmed || /^prefix\s/i.test(trimmed)) return false;
  if (trimmed.toLowerCase().split('+').pop() === DIGIT_RANGE_TOKEN) return false;
  return comboMatcher(trimmed)(e);
}

/**
 * Build the "does the APP own this key?" predicate for an xterm pane's
 * attachCustomKeyEventHandler, derived from the LIVE keybinding config instead
 * of a hardcoded list. Returning true means xterm must ignore the key (return
 * false from the handler) so the app's own listeners can take it.
 *
 * The old hardcoded twins (TerminalPane / ClaudePane) had silently drifted
 * into relics: Ctrl+T/W/D//,? were blocked from the PTY for bindings that
 * moved to leader chords long ago — leaving them DEAD keys (neither app nor
 * shell), stealing readline's transpose/delete-word/EOF. Deriving from config
 * returns unbound keys to the shell and tracks presets and user rebinds
 * automatically.
 *
 * App-owned, by construction:
 *  - every direct (non-chord, non-scoped) binding that carries a real
 *    modifier (ctrl/alt/meta/mod) — shift alone is typing, never app nav;
 *  - bare F-keys bindings (f1 help);
 *  - digit-range bindings (jump-tab Ctrl+1-9, move-tab Ctrl+Shift+1-9);
 *  - the resolved chord leader when it's a modifier combo (Ctrl+Space must arm
 *    the chord from inside a terminal; the Linux lone-Alt tap needs nothing —
 *    it fires on window key-up and a bare modifier types nothing in a PTY);
 *  - the retained structural rules both panes and BrowserPane's guest matcher
 *    have always shared: wholesale Ctrl+Shift+* (pane resize/move and the
 *    Ctrl+Shift copy/paste pair), Alt+Arrows (pane nav) and Ctrl+Alt+←/→ (tab
 *    nav).
 *
 * Pane-local behaviors (the Ctrl+C/V clipboard quartet, F2 rename) stay in the
 * pane handler ahead of this predicate — they need pane state (selection).
 */
export function buildXtermAppKeyPredicate(
  shortcuts: Record<string, string>,
  prefix?: string,
): (e: KeyboardEvent) => boolean {
  const matchers: ((e: KeyboardEvent) => boolean)[] = [];
  const rangeCombos: string[] = [];
  for (const [action, combo] of Object.entries(shortcuts)) {
    const trimmed = (combo ?? '').trim();
    if (!trimmed || SCOPED_ACTIONS.has(action) || /^prefix\s/i.test(trimmed)) continue;
    if (parseDigitRangeCombo(trimmed)) {
      rangeCombos.push(trimmed);
      continue;
    }
    const parts = resolveMod(trimmed.toLowerCase()).split('+');
    const isFKey = parts.length === 1 && /^f\d{1,2}$/.test(parts[0]);
    const hasRealModifier = ['ctrl', 'alt', 'meta'].some((m) => parts.includes(m));
    if (!isFKey && !hasRealModifier) continue; // bare/shift-only keys are typing
    matchers.push(comboMatcher(trimmed));
  }
  const leader = (prefix ?? '').trim().toLowerCase();
  const isLoneModifierLeader = ['ctrl', 'alt', 'shift', 'meta'].includes(leader);
  if (leader && comboHasModifiers(leader) && !isLoneModifierLeader) {
    matchers.push(comboMatcher(leader));
  }
  return (e: KeyboardEvent) => {
    if (e.ctrlKey && e.shiftKey) return true; // resize/move + copy-paste family
    if (e.altKey && !e.ctrlKey && e.key.startsWith('Arrow')) return true;
    if (e.ctrlKey && e.altKey && (e.key === 'ArrowLeft' || e.key === 'ArrowRight')) return true;
    if (matchers.some((m) => m(e))) return true;
    return rangeCombos.some((c) => digitFromRangeEvent(e, c) !== null);
  };
}

/** The digit pressed (1–9) when a keydown matches a digit-range combo ("1-9",
 *  "ctrl+1-9"); null otherwise. Uses e.code so Shift-modified digits still
 *  resolve. */
export function digitFromRangeEvent(e: KeyboardEvent, combo: string | undefined): number | null {
  const spec = parseDigitRangeCombo(combo);
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

/** Actions that re-arm the layer for the repeat window after firing, so
 *  `prefix h h l` walks panes and `prefix ] ]` cycles tabs without re-pressing
 *  the leader. Movement only — structural actions (close, split) end the
 *  gesture. */
export const REPEAT_ACTIONS = new Set([
  'nav-left',
  'nav-right',
  'nav-up',
  'nav-down',
  'next-tab',
  'prev-tab',
  'cycle-pane',
  'swap-pane-left',
  'swap-pane-right',
  'chat-scroll-up',
  'chat-scroll-down',
]);

/** The literal byte(s) a `prefix prefix` passthrough writes to the terminal —
 *  what the leader WOULD have sent had the app not eaten it, so a nested tmux
 *  bound to the same prefix still works. Derived from the CONFIGURED combo
 *  (ctrl+space → NUL even on Linux, where the tap-Alt substitute armed the
 *  layer); combos with no terminal byte encoding return '' (no-op). */
export function leaderPassthroughBytes(prefix: string): string {
  const parts = resolveMod((prefix ?? '').toLowerCase().trim()).split('+');
  const key = parts[parts.length - 1];
  const ctrlOnly = parts.includes('ctrl') && !parts.includes('alt') && !parts.includes('meta');
  if (!ctrlOnly) return '';
  if (key === 'space') return '\x00';
  if (/^[a-z]$/.test(key)) return String.fromCharCode(key.charCodeAt(0) - 96);
  return '';
}

export interface ChordConflict {
  kind: 'duplicate' | 'shadow';
  /** The space-joined chord path both parties share. */
  path: string;
  /** The action ids involved (two for a duplicate; the leaf action whose path
   *  is also a group for a shadow). */
  actions: string[];
}

/**
 * Detect chord-map conflicts the tree builder would otherwise resolve
 * silently (last-writer-wins): two actions on the SAME full path, and a leaf
 * whose path is also a group prefix of a longer chord (the leaf fires and the
 * submenu is unreachable — or vice versa, depending on build order). Case- and
 * modifier-aware: steps are compared canonically ('shift+k' ≠ 'k'). Feeds
 * Settings-save validation and the preset drift tests; buildChordTree stays
 * permissive so a bad config degrades instead of crashing the dispatcher.
 */
export function findChordConflicts(shortcuts: Record<string, string>): ChordConflict[] {
  const canonical = (step: string) => step.toLowerCase();
  const leaves = new Map<string, string[]>(); // path → actions
  const paths: { action: string; steps: string[] }[] = [];
  for (const [action, combo] of Object.entries(shortcuts)) {
    const m = /^prefix\s+(.+)$/i.exec((combo ?? '').trim());
    if (!m) continue;
    const steps = m[1].trim().split(/\s+/).map(canonical);
    paths.push({ action, steps });
    const key = steps.join(' ');
    leaves.set(key, [...(leaves.get(key) ?? []), action]);
  }
  const conflicts: ChordConflict[] = [];
  for (const [path, actions] of leaves) {
    if (actions.length > 1) conflicts.push({ kind: 'duplicate', path, actions });
  }
  for (const [path, actions] of leaves) {
    const prefixSteps = path.split(' ');
    const shadowed = paths.filter(
      (p) => p.steps.length > prefixSteps.length && prefixSteps.every((s, i) => p.steps[i] === s),
    );
    if (shadowed.length > 0) {
      conflicts.push({
        kind: 'shadow',
        path,
        actions: [...actions, ...shadowed.map((p) => p.action)],
      });
    }
  }
  return conflicts;
}

/**
 * Human-readable binding conflicts for the Settings save-time check: chord
 * conflicts (duplicates + prefix shadowing, via findChordConflicts) plus two
 * un-scoped actions sharing one DIRECT combo. Surface-scoped actions are
 * exempt from the direct check (fleet/inbox reuse y/n/j/k by design, matched
 * only inside their own overlay). Warnings, not errors — the dispatcher
 * degrades (first/last match wins) rather than breaking, but the user should
 * know which key stopped meaning what they think.
 */
export function findBindingConflicts(shortcuts: Record<string, string>): string[] {
  const warnings: string[] = [];
  for (const c of findChordConflicts(shortcuts)) {
    warnings.push(
      c.kind === 'duplicate'
        ? `“prefix ${c.path}” is bound to ${c.actions.map((a) => ACTION_LABELS[a] ?? a).join(' AND ')} — only one can fire`
        : `“prefix ${c.path}” (${ACTION_LABELS[c.actions[0]] ?? c.actions[0]}) shadows longer chords: ${c.actions
            .slice(1)
            .map((a) => ACTION_LABELS[a] ?? a)
            .join(', ')}`,
    );
  }
  const byCombo = new Map<string, string[]>();
  for (const [action, combo] of Object.entries(shortcuts)) {
    const trimmed = (combo ?? '').trim();
    if (!trimmed || /^prefix\s/i.test(trimmed) || SCOPED_ACTIONS.has(action)) continue;
    const key = resolveMod(trimmed.toLowerCase());
    byCombo.set(key, [...(byCombo.get(key) ?? []), action]);
  }
  for (const [combo, actions] of byCombo) {
    if (actions.length > 1) {
      warnings.push(
        `${formatCombo(combo)} is bound to ${actions.map((a) => ACTION_LABELS[a] ?? a).join(' AND ')} — only one can fire`,
      );
    }
  }
  return warnings;
}

export function shortcutFor(
  action: string | undefined,
  shortcuts: Record<string, string> | undefined,
  prefix?: string,
): string | undefined {
  if (!action || !shortcuts) return undefined;
  const combo = shortcuts[action];
  return combo ? formatBinding(combo, prefix) : undefined;
}
