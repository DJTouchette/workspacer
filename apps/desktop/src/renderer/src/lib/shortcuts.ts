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
 * Resolve the human-readable shortcut for an action from the (already merged
 * with defaults) shortcuts map. Returns undefined when the action has no
 * binding, so callers can omit the badge entirely.
 */
export function shortcutFor(
  action: string | undefined,
  shortcuts: Record<string, string> | undefined,
): string | undefined {
  if (!action || !shortcuts) return undefined;
  const combo = shortcuts[action];
  return combo ? formatCombo(combo) : undefined;
}
