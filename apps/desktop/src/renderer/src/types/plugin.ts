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

/** Resolve a pane contribution's webview URL from its plugin's server port. */
export function pluginPaneURL(m: PluginManifest, pane: PluginPaneContribution): string {
  if (!m.server?.port) return 'about:blank';
  return `http://127.0.0.1:${m.server.port}${pane.path || '/'}`;
}
