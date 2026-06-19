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

/** "ctrl+shift+a" → "Ctrl+Shift+A". */
export function formatCombo(combo: string): string {
  return combo
    .split('+')
    .map((p) => PART_DISPLAY[p] ?? (p.length === 1 ? p.toUpperCase() : p.charAt(0).toUpperCase() + p.slice(1)))
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
/** Short, human labels for each bindable action. Shared by the help overlay,
 *  the settings editor, and the chord hint. */
export const ACTION_LABELS: Record<string, string> = {
  'prev-agent': 'Previous agent',
  'next-agent': 'Next agent',
  'next-attention': 'Agent needing you',
  'spawn-agent': 'Spawn agent',
  'prev-tab': 'Previous tab',
  'next-tab': 'Next tab',
  'move-tab-left': 'Move tab left',
  'move-tab-right': 'Move tab right',
  'cycle-view': 'Cycle view mode',
  'nav-left': 'Focus pane left',
  'nav-right': 'Focus pane right',
  'nav-up': 'Focus pane up',
  'nav-down': 'Focus pane down',
  'new-terminal': 'New terminal',
  'new-browser': 'New browser',
  'new-claude': 'New Claude',
  'split': 'Split pane',
  'quick-split': 'Quick split',
  'close-pane': 'Close pane',
  'open-file': 'Open file',
  'rename-tab': 'Rename tab',
  'toggle-sidebar': 'Toggle sidebar',
  'toggle-terminal': 'Toggle terminal',
  'toggle-inbox': 'Toggle inbox',
  'toggle-fleet': 'Toggle fleet deck',
  'toggle-inspector': 'Toggle inspector',
  'library-picker': 'Library picker',
  'command-palette': 'Command palette',
  'save-session': 'Save session',
  'settings': 'Settings',
  'toggle-help': 'Toggle help',
};

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
      if (!child) { child = { step, node: { children: [] } }; node.children.push(child); }
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
        : a.isGroup ? -1 : 1,
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

export function shortcutFor(
  action: string | undefined,
  shortcuts: Record<string, string> | undefined,
  prefix?: string,
): string | undefined {
  if (!action || !shortcuts) return undefined;
  const combo = shortcuts[action];
  return combo ? formatBinding(combo, prefix) : undefined;
}
