import type { PaneType } from '../types/pane';
import type { PluginPane } from '../types/plugin';

/**
 * The pane-creation menus — the in-pane "Split into…" button and the "+"
 * new-tab dropdown — share their contents. What they list is derived here from
 * the `ui.paneMenu` setting and the currently-loaded plugin panes.
 *
 * Semantics of `ui.paneMenu`:
 *   - absent (undefined)  → the built-in set (DEFAULT_PANE_MENU) followed by
 *                           every contributed plugin pane. This is the default:
 *                           "what we have, plus all plugins."
 *   - an explicit array   → exactly those entries, in that order. Each id
 *                           resolves to a built-in (see MENU_BUILTIN_LABELS) or,
 *                           failing that, a loaded plugin pane by its `type`.
 *                           Ids matching neither (a stale plugin, or the retired
 *                           'notes' type) are silently dropped.
 */

/** Built-in pane types offerable in the creation menus, mapped to their label.
 *  An `ui.paneMenu` entry may only name one of these (anything else is treated
 *  as a plugin pane type). */
export const MENU_BUILTIN_LABELS: Partial<Record<PaneType, string>> = {
  claude: 'Claude Code',
  terminal: 'Terminal',
  browser: 'Browser',
  review: 'Review',
  library: 'Library',
};

/** The default built-in set + order when `ui.paneMenu` is unset. Mirrors the
 *  command palette's built-in "New X" actions (minus Library/Editor, which are
 *  special-cased there). 'notes' was intentionally dropped — it moved out into
 *  the Notes plugin. */
export const DEFAULT_PANE_MENU: PaneType[] = ['claude', 'terminal', 'browser', 'review'];

export type PaneMenuEntry =
  | { kind: 'builtin'; type: PaneType; label: string }
  | { kind: 'plugin'; pane: PluginPane; label: string };

function builtinEntry(type: PaneType): PaneMenuEntry {
  return { kind: 'builtin', type, label: MENU_BUILTIN_LABELS[type] ?? type };
}

function pluginEntry(pane: PluginPane): PaneMenuEntry {
  return { kind: 'plugin', pane, label: pane.title };
}

/**
 * Resolve the ordered list of pane-creation menu entries.
 *
 * @param paneMenu       `config.ui.paneMenu` — an explicit id list, or undefined
 *                       for the default (built-ins + all plugins).
 * @param pluginPanes    the currently-loaded plugin panes (from usePlugins()).
 */
export function buildPaneMenu(
  paneMenu: string[] | undefined,
  pluginPanes: PluginPane[],
): PaneMenuEntry[] {
  if (Array.isArray(paneMenu)) {
    const pluginByType = new Map(pluginPanes.map((p) => [p.type, p]));
    const out: PaneMenuEntry[] = [];
    for (const id of paneMenu) {
      // Built-in first, then a plugin pane by type — matching the documented
      // precedence. Otherwise a plugin whose pane type collides with a built-in
      // id (e.g. 'review') would shadow the built-in the user configured.
      if (Object.prototype.hasOwnProperty.call(MENU_BUILTIN_LABELS, id)) {
        out.push(builtinEntry(id as PaneType));
        continue;
      }
      const plugin = pluginByType.get(id);
      if (plugin) {
        out.push(pluginEntry(plugin));
      }
      // else: unknown/stale id (removed pane type, uninstalled plugin) → drop.
    }
    return out;
  }

  // Default: the built-in set, then every contributed plugin pane.
  return [...DEFAULT_PANE_MENU.map(builtinEntry), ...pluginPanes.map(pluginEntry)];
}
