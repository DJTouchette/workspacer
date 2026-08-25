// Plugin manifests, as served by the hub's /plugins endpoint, plus the
// UI-ready shapes derived from them.

import type { WidgetSize } from './widget';

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

/**
 * A widget contribution: a small, glanceable view for a project's widget board.
 * Mirrors Go plugin.WidgetContribution — keep in sync.
 *
 * A sibling of a pane, not a small one. See types/widget.ts for why the size set
 * is closed and what each class spans.
 */
export interface PluginWidgetContribution {
  id: string;
  title: string;
  icon?: string;
  path?: string;
  /** Supported footprints. Omitted means ['small'] — see widgetSizesOf. */
  sizes?: WidgetSize[];
}

/** A widget's declared sizes, defaulting to ['small'] as the Go side does. */
export function widgetSizesOf(w: PluginWidgetContribution): WidgetSize[] {
  return w.sizes && w.sizes.length > 0 ? w.sizes : ['small'];
}

export interface PluginHotkeyContribution {
  id: string;
  default: string;
  command: string; // "open-pane:<type>" | "emit:<eventType>"
}

export type PluginSettingType = 'boolean' | 'number' | 'string' | 'select';

/** What the hub reports for a SET secret setting instead of its value; writing
 *  this exact string back is ignored (means "unchanged"). Mirrors Go
 *  plugin.SecretPlaceholder — keep in sync. */
export const SECRET_PLACEHOLDER = '__WKS_SECRET__';

/** One configurable setting a plugin declares; the host renders a control for it. */
export interface PluginSettingDef {
  key: string;
  label: string;
  type: PluginSettingType;
  default?: unknown;
  options?: string[]; // for type 'select'
  help?: string;
  /** Sensitive string (PAT/API key): masked write-only input, value never
   *  echoed to clients — reads report SECRET_PLACEHOLDER when set. */
  secret?: boolean;
  /**
   * 'project' = one value per directory, stored by the host under
   * config.projects[<dir>].plugins[<pluginId>] and edited on the Projects
   * settings page. Absent or 'global' = one value for the plugin, edited on its
   * own settings page. A project setting can never be secret (the hub refuses
   * the combination at manifest load) — config.yaml holds no credentials.
   */
  scope?: 'global' | 'project';
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
  return typeof c === 'string' ? [] : (c.paths ?? []);
}

/** One MCP tool a plugin contributes to the agent facade (manifest `tools`). */
export interface PluginToolDef {
  name: string;
  description: string;
  inputSchema?: Record<string, unknown>;
  method: string;
}

export interface PluginManifest {
  id: string;
  name: string;
  apiVersion: string;
  /** Author-declared release version (semver-ish, e.g. "1.4.0"). Optional; when
   *  absent the plugin can be reinstalled but never reports an update available. */
  version?: string;
  server?: PluginServerSpec;
  panes?: PluginPaneContribution[];
  widgets?: PluginWidgetContribution[];
  hotkeys?: PluginHotkeyContribution[];
  settings?: PluginSettingDef[];
  provides?: string[];
  /** MCP tools this plugin contributes to the workspacer agent facade, each
   *  bound to a `provides` method. Served on the full (trusted-host) manifest;
   *  used by the spawn dialog's plugin-tools picker. */
  tools?: PluginToolDef[];
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
  /**
   * Origin of the hub serving this plugin's `ui` assets, stamped by whoever
   * fetched the manifest — main on the desktop, `webBackend` on `/app` and in
   * remote-client mode. Only the fetcher knows which hub this client is
   * actually talking to, and it is never safe to guess (see pluginContentURL).
   */
  uiBase?: string;
  /**
   * Base URL of a SIDECAR plugin's own HTTP server, as reachable BY THIS CLIENT
   * — stamped by the same fetcher. A sidecar listens on the hub machine's
   * loopback, so this is `http://127.0.0.1:<port>` when the hub is this machine
   * and ABSENT when it is not: from a browser elsewhere, that address is the
   * viewer's own computer, and framing whatever answers there is worse than
   * showing nothing. Absent ⇒ the pane explains itself instead.
   */
  serverBase?: string;
}

/** Result of checking one installed plugin against its install source, from the
 *  hub's /plugins/updates route. Mirrors the Go plugin.UpdateStatus. */
export interface PluginUpdateStatus {
  id: string;
  /** Version currently installed on disk (empty if the manifest declares none). */
  installed?: string;
  /** Version published at the install source (empty if that manifest declares none). */
  latest?: string;
  /** True when `latest` is a strict upgrade over `installed`. */
  hasUpdate: boolean;
  /** Set when the source could not be reached or read. */
  error?: string;
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
  if (tool === 'npm' || tool === 'node' || tool === 'pnpm' || tool === 'yarn')
    return { label: 'Needs Node.js', warn: true };
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

/** A widget contribution resolved to a concrete webview URL. */
export interface PluginWidget {
  pluginId: string;
  /** Plugin's display name, for the add-widget picker's grouping. */
  pluginName: string;
  /** Widget id, unique within its plugin. */
  id: string;
  title: string;
  icon?: string;
  url: string;
  /** Footprints this widget declared it can render at (never empty). */
  sizes: WidgetSize[];
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

/**
 * Resolve one of a plugin's declared paths to a concrete guest URL.
 *
 * Both bases used to be constants: a sidecar got `http://127.0.0.1:<its port>`
 * and a hub-served `ui` plugin `http://127.0.0.1:7895`. On the desktop that is
 * true by construction — main spawns the hub and every sidecar on this machine.
 * From a browser on ANOTHER machine, which is the entire point of `/app`, both
 * spellings name **the viewer's own loopback**: the pane loads nothing, or
 * frames some unrelated service that happens to answer on that port on the
 * user's own computer.
 *
 * So neither base is guessed here any more. `uiBase` and `serverBase` are
 * stamped by whoever fetched the manifest — the only party that knows which hub
 * this client is talking to and whether it is local — and a base that could not
 * be established yields `''`, which the pane renders as an explanation. The
 * directory form (trailing slash) is kept so the hub serves index.html without
 * a redirect that would drop the busToken query.
 *
 * `frameOrigin` overrides the hub-served base with a second spelling of the
 * SAME hub, so a browser can frame the plugin cross-origin instead of opaque —
 * see lib/pluginOrigin.ts for why that is the whole ballgame in a browser.
 *
 * Shared by panes and widgets so a plugin's two surfaces always resolve against
 * the same origin — which is also what lets their webviews share a renderer
 * process.
 */
function pluginContentURL(
  m: PluginManifest,
  path: string | undefined,
  frameOrigin?: string,
): string {
  const sub = path && path !== '/' ? (path.startsWith('/') ? path : `/${path}`) : '/';
  if (m.server?.port) {
    // No serverBase ⇒ this client cannot reach the sidecar's loopback port.
    return m.serverBase ? `${m.serverBase}${sub}` : '';
  }
  if (m.ui) {
    const base = frameOrigin || m.uiBase;
    return base ? `${base}/plugins/ui/${encodeURIComponent(m.id)}${sub}` : '';
  }
  // Neither: an unauthenticated caller gets the PUBLIC projection of a manifest
  // (plugin.PublicManifests), which withholds `server` and `ui` deliberately —
  // it names the contributions, never where they are served from.
  return '';
}

/** Resolve a pane contribution's guest URL. */
export function pluginPaneURL(
  m: PluginManifest,
  pane: PluginPaneContribution,
  frameOrigin?: string,
): string {
  return pluginContentURL(m, pane.path, frameOrigin);
}

/** Resolve a widget contribution's guest URL. */
export function pluginWidgetURL(
  m: PluginManifest,
  widget: PluginWidgetContribution,
  frameOrigin?: string,
): string {
  return pluginContentURL(m, widget.path, frameOrigin);
}

/**
 * Rehome a plugin URL that was resolved by SOME OTHER CLIENT onto the origin
 * this one can reach, keeping its path and query.
 *
 * A pane's URL is baked at open time and persisted in the shared layout
 * document, so a pane opened on the desktop carries that machine's loopback
 * (`http://127.0.0.1:7895/plugins/ui/…`, `http://127.0.0.1:9211/…`). Restore
 * that layout in a browser on another machine and the URL points at the
 * VIEWER's loopback — the exact hazard pluginContentURL now refuses to create.
 * Rewriting only the origin keeps everything the opener chose (which pane path,
 * which sessionId/cwd) while making the address true here.
 *
 * An unknown plugin (list still loading) is left alone rather than blanked: the
 * stored URL is the best guess available until the manifest lands.
 */
export function relocatePluginURL(
  url: string,
  m: PluginManifest | undefined,
  frameOrigin?: string,
): string {
  if (!url || !m) return url;
  const base = m.server?.port ? m.serverBase : frameOrigin || m.uiBase;
  if (!base) return m.server?.port || m.ui ? '' : url;
  try {
    const target = new URL(url);
    // Rebuild rather than assign `host`: assigning a host with no port leaves
    // the old port in place, which silently keeps the other machine's :7895.
    return new URL(`${target.pathname}${target.search}${target.hash}`, base).toString();
  } catch {
    return url;
  }
}
