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

/** True if the combo carries any modifier (ctrl/alt/shift/meta). A modifier-less
 *  combo (e.g. a bare "`" or "space") is only safe as a global binding when it's
 *  guarded against editable contexts — see isEditableTarget. */
export function comboHasModifiers(combo: string): boolean {
  const parts = (combo ?? '').toLowerCase().trim().split('+');
  return parts.some((p) => p === 'ctrl' || p === 'alt' || p === 'shift' || p === 'meta');
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
    .map(
      (p) =>
        PART_DISPLAY[p] ??
        (p.length === 1 ? p.toUpperCase() : p.charAt(0).toUpperCase() + p.slice(1)),
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
   *  the Inbox drawer). Scoped actions are matched by that surface's own
   *  keydown listener while it's open — the global handler skips them, so a
   *  bare key like "j" never fires fleet actions from a workspace. */
  scope?: 'fleet' | 'inbox';
}

/** The canonical action list, in display order, grouped by section. */
export const ACTION_REGISTRY: ActionMeta[] = [
  // Agents
  { action: 'prev-agent', label: 'Previous agent', section: 'Agents' },
  { action: 'next-agent', label: 'Next agent', section: 'Agents' },
  { action: 'next-attention', label: 'Agent needing you', section: 'Agents' },
  { action: 'spawn-agent', label: 'Spawn agent', section: 'Agents' },
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
  { action: 'toggle-inbox', label: 'Toggle inbox', section: 'Panels & Overlays' },
  { action: 'toggle-fleet', label: 'Toggle fleet', section: 'Panels & Overlays' },
  { action: 'toggle-ui-mode', label: 'Toggle focus / full mode', section: 'Panels & Overlays' },
  { action: 'toggle-inspector', label: 'Toggle inspector', section: 'Panels & Overlays' },
  { action: 'library-picker', label: 'Library picker', section: 'Panels & Overlays' },
  // Tools
  { action: 'command-palette', label: 'Command palette', section: 'Tools' },
  { action: 'save-session', label: 'Save session', section: 'Tools' },
  { action: 'settings', label: 'Settings', section: 'Tools' },
  { action: 'toggle-help', label: 'Toggle help', section: 'Tools' },
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
  // Inbox (active only while the drawer is open)
  { action: 'inbox-move-down', label: 'Select next item', section: 'Inbox', scope: 'inbox' },
  { action: 'inbox-move-up', label: 'Select previous item', section: 'Inbox', scope: 'inbox' },
  { action: 'inbox-open', label: 'Open agent', section: 'Inbox', scope: 'inbox' },
  { action: 'inbox-approve-yes', label: 'Approve', section: 'Inbox', scope: 'inbox' },
  { action: 'inbox-approve-no', label: 'Deny', section: 'Inbox', scope: 'inbox' },
  {
    action: 'inbox-answer',
    label: 'Answer question (option)',
    section: 'Inbox',
    scope: 'inbox',
    digitRange: true,
  },
  { action: 'inbox-dismiss', label: 'Dismiss item', section: 'Inbox', scope: 'inbox' },
  { action: 'inbox-snooze', label: 'Snooze item', section: 'Inbox', scope: 'inbox' },
  { action: 'inbox-clear-reviewed', label: 'Clear all reviewed', section: 'Inbox', scope: 'inbox' },
];

/** Action ids that only bind inside their own surface (fleet/inbox); the
 *  global direct-binding matcher must skip these. */
export const SCOPED_ACTIONS = new Set(ACTION_REGISTRY.filter((a) => a.scope).map((a) => a.action));

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
  const parts = (combo ?? '').toLowerCase().trim().split('+');
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

/** True when a keydown matches a direct combo like "shift+e", "ctrl+j", or a
 *  bare "j". Modifiers must match exactly (so "j" doesn't fire on Ctrl+J).
 *  Prefix chords and digit-range combos never match here. */
export function eventMatchesCombo(e: KeyboardEvent, combo: string | undefined): boolean {
  const trimmed = (combo ?? '').toLowerCase().trim();
  if (!trimmed || /^prefix\s/.test(trimmed)) return false;
  const parts = trimmed.split('+');
  const key = parts[parts.length - 1];
  if (key === DIGIT_RANGE_TOKEN) return false;
  const eventKey = e.key === ' ' ? 'space' : e.key.toLowerCase();
  return (
    eventKey === key &&
    e.ctrlKey === parts.includes('ctrl') &&
    e.altKey === parts.includes('alt') &&
    e.shiftKey === parts.includes('shift') &&
    e.metaKey === parts.includes('meta')
  );
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

export function shortcutFor(
  action: string | undefined,
  shortcuts: Record<string, string> | undefined,
  prefix?: string,
): string | undefined {
  if (!action || !shortcuts) return undefined;
  const combo = shortcuts[action];
  return combo ? formatBinding(combo, prefix) : undefined;
}
