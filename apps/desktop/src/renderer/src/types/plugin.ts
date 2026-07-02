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

export type PluginSettingType = 'boolean' | 'number' | 'string' | 'select';

/** One configurable setting a plugin declares; the host renders a control for it. */
export interface PluginSettingDef {
  key: string;
  label: string;
  type: PluginSettingType;
  default?: unknown;
  options?: string[]; // for type 'select'
  help?: string;
}

/** One capability entry in a manifest: a bus method the plugin may call, with
 *  optional filesystem scoping (the object form the hub serves for path-scoped
 *  methods). Mirrors the hub's `capspec` Capability. */
export type PluginCapability = string | { method: string; paths?: string[] };

/** The method name of a capability entry, whichever form it takes. */
export function capabilityMethod(c: PluginCapability): string {
  return typeof c === 'string' ? c : c.method;
}

/** The declared filesystem roots of a capability entry (empty for verb-only). */
export function capabilityPaths(c: PluginCapability): string[] {
  return typeof c === 'string' ? [] : c.paths ?? [];
}

export interface PluginManifest {
  id: string;
  name: string;
  apiVersion: string;
  server?: PluginServerSpec;
  panes?: PluginPaneContribution[];
  hotkeys?: PluginHotkeyContribution[];
  settings?: PluginSettingDef[];
  provides?: string[];
  /** Capabilities the plugin may call. A bare string is verb-only; the object
   *  form carries the filesystem roots a path-scoped method (fs.*, search.project)
   *  is confined to. The hub serves the object form for path-scoped caps, so the
   *  permissions view can show the scope. */
  capabilities?: PluginCapability[];
  emits?: string[];
  consumes?: string[];
  /** Optional one-time build/setup command run on install (e.g. ["go","build",…]).
   *  Present on manifests served by the hub; used to derive runtime requirements. */
  install?: string[];
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

/** What a plugin needs on the machine, derived from its manifest. */
export interface PluginRequirement {
  label: string;
  /** True for a real prerequisite (runtime/toolchain) worth warning about. */
  warn: boolean;
}

/**
 * Best-effort guess of a plugin's build/runtime requirement from its manifest,
 * so the UI can warn before adding a sidecar that needs Python/Go/Rust/Node —
 * or explain why one is crash-looping. Webview-only plugins (no server) need
 * nothing. We can't truly detect what a prebuilt binary needs, so a server
 * command we don't recognize is reported generically.
 */
export function pluginRequirement(m: PluginManifest): PluginRequirement {
  const cmd = m.server?.command ?? '';
  const tool = m.install?.[0];
  if (tool === 'go') return { label: 'Needs Go toolchain', warn: true };
  if (tool === 'cargo') return { label: 'Needs Rust toolchain', warn: true };
  if (tool === 'npm' || tool === 'node' || tool === 'pnpm' || tool === 'yarn') return { label: 'Needs Node.js', warn: true };
  if (/(^|\/)python/i.test(cmd)) return { label: 'Needs Python 3', warn: true };
  if (/(^|\/)node(\.exe)?$/i.test(cmd)) return { label: 'Needs Node.js', warn: true };
  if (!m.server && m.ui) return { label: 'No dependencies', warn: false };
  if (m.server) return { label: `Runs ${cmd || 'a local server'}`, warn: true };
  return { label: 'No dependencies', warn: false };
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
