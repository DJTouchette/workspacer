// Plugin manifests, as served by the hub's /plugins endpoint, plus the
// UI-ready shapes derived from them.

export interface PluginServerSpec {
  command: string;
  args?: string[];
  port?: number;
  health?: string;
}

export type PluginPaneScope = 'global' | 'agent' | 'both';

export interface PluginPaneContribution {
  type: string;
  title: string;
  icon?: string;
  path?: string;
  scope?: PluginPaneScope;
}

export interface PluginHotkeyContribution {
  id: string;
  default: string;
  command: string; // "open-pane:<type>" | "emit:<eventType>"
}

export interface PluginManifest {
  id: string;
  name: string;
  apiVersion: string;
  server?: PluginServerSpec;
  panes?: PluginPaneContribution[];
  hotkeys?: PluginHotkeyContribution[];
  provides?: string[];
  capabilities?: string[];
  emits?: string[];
  consumes?: string[];
  /** Install reference (GitHub URL / owner-repo) recorded at install time; enables one-click update. */
  source?: string;
  /** True when the plugin is installed but disabled (sidecar stopped, contributions withheld). */
  disabled?: boolean;
  /** Per-plugin bus token, injected by the trusted host into this plugin's webview URL
   *  so its page can connect to the hub bus scoped to its declared capabilities. */
  busToken?: string;
  /** Webview-only plugins: the subdirectory of static assets the hub serves at
   *  /plugins/ui/<id>/ (set instead of `server`). */
  ui?: string;
  /** Origin of the hub serving this plugin's `ui` assets, attached by main
   *  (it knows the hub address). The renderer builds the pane URL against it. */
  uiBase?: string;
}

/** A pane contribution resolved to a concrete webview URL. */
export interface PluginPane {
  pluginId: string;
  type: string;
  title: string;
  icon?: string;
  url: string;
  scope: PluginPaneScope;
  /** Per-plugin bus token to inject into the webview URL (see PluginManifest.busToken). */
  busToken?: string;
}

/** A hotkey contribution flattened for binding. */
export interface PluginHotkey {
  pluginId: string;
  id: string;
  combo: string;
  command: string;
}

/** Default hub origin (loopback) when main hasn't attached one — matches the
 *  hub's default --addr. */
const DEFAULT_HUB_ORIGIN = 'http://127.0.0.1:7895';

/**
 * Resolve a pane contribution's webview URL. A sidecar plugin loads from its own
 * server port; a webview-only plugin (no server, has `ui`) loads from the hub's
 * /plugins/ui/<id>/ route. The directory form (trailing slash) is used so the
 * hub serves index.html directly without a redirect that would drop the
 * busToken query.
 */
export function pluginPaneURL(m: PluginManifest, pane: PluginPaneContribution): string {
  if (m.server?.port) {
    return `http://127.0.0.1:${m.server.port}${pane.path || '/'}`;
  }
  if (m.ui) {
    const base = m.uiBase || DEFAULT_HUB_ORIGIN;
    const sub = pane.path && pane.path !== '/' ? (pane.path.startsWith('/') ? pane.path : `/${pane.path}`) : '/';
    return `${base}/plugins/ui/${encodeURIComponent(m.id)}${sub}`;
  }
  return 'about:blank';
}
