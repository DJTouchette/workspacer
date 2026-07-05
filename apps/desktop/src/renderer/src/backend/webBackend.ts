/**
 * Web build of the `window.electronAPI` surface.
 *
 * In Electron, `src/main/preload.ts` populates `window.electronAPI` via the
 * contextBridge. In a browser there is no preload, so this factory builds an
 * equivalent object backed by the hub-bus WebSocket and the bootstrap in
 * `install.ts` assigns it to `window.electronAPI`. The React app is unchanged —
 * it still calls `window.electronAPI.*` and never learns which transport it got.
 *
 * Coverage tracks the hub capability surface (`hubCapabilities.ts`). Methods the
 * hub supports (agents, terminal I/O, approvals, transcript, config, git, files,
 * provider model/detection discovery, terminal-exit + library-change events) are
 * wired; the remainder returns a safe default and warns once, to be filled in as
 * the hub RPC surface widens (Phase 3). Each remaining stub is marked `HUB-TODO`.
 * The still-stubbed surface is host-trusted/local-only work — plugin
 * install/inspect, pane-token minting, native OS dialogs/notifications, the quit
 * handshake — that a web/remote viewer can't perform against someone else's host.
 */

import type { ElectronAPI, SessionListEntry } from '../types/electron';
import type {
  ClaudeSessionSnapshot,
  AppConfig,
  AppConfigPartial,
  ClaudeProfile,
} from '../../../main/shared/ipcTypes';
import { HubBusClient, type HubEventEnvelope } from './hubBusClient';

/** Decode a base64 PTY chunk into a binary string (1 char = 1 byte), matching
 *  the MessagePort contract the desktop's onTerminalOutput delivers. */
function decodePtyChunk(data: unknown): string {
  if (typeof data !== 'string') return '';
  try {
    return atob(data);
  } catch {
    return '';
  }
}

const warned = new Set<string>();
function warnOnce(method: string): void {
  if (warned.has(method)) return;
  warned.add(method);
  // eslint-disable-next-line no-console
  console.warn(`[webBackend] ${method}() is not yet available over the hub bus — returning a safe default (HUB-TODO).`);
}

/**
 * Bracket a live PTY stream for one viewer: attach → subscribe → keepalive,
 * and return a teardown that detaches and unsubscribes. Used by both the Claude
 * and shell terminal output subscriptions, which are the byte sinks that bound
 * a stream's lifetime in the web build.
 */
function streamPty(
  client: HubBusClient,
  reprimers: Map<string, () => void>,
  sessionId: string,
  callback: (data: string) => void,
): () => void {
  const unsub = client.subscribe(`pty.bytes.${sessionId}`, (ev: HubEventEnvelope) => {
    callback(decodePtyChunk(ev.data));
  });
  // attachTerminal makes claudemon replay its ring buffer (the current screen).
  const attach = () => client.call('sessions.attachTerminal', { sessionId }).catch(() => {});
  attach();

  // Re-prime hook: the app keeps every agent pane mounted and only toggles
  // visibility, so a terminal can initialize while its pane is hidden / zero
  // size — which eats the initial replay (it's written into a 0-col grid). When
  // the pane is shown it resizes; we treat that as the cue to re-attach and
  // replay the current screen onto the now-correctly-sized terminal. Debounced
  // so a burst of fit/resize events triggers a single replay once it settles.
  let reprimeTimer: ReturnType<typeof setTimeout> | null = null;
  reprimers.set(sessionId, () => {
    if (reprimeTimer) clearTimeout(reprimeTimer);
    reprimeTimer = setTimeout(attach, 120);
  });

  // The hub lease expires after ~20s; refresh well inside that window.
  const keepalive = setInterval(() => {
    client.call('sessions.terminalKeepalive', { sessionId }).catch(() => {});
  }, 10000);
  return () => {
    if (reprimeTimer) clearTimeout(reprimeTimer);
    reprimers.delete(sessionId);
    clearInterval(keepalive);
    unsub();
    client.call('sessions.detachTerminal', { sessionId }).catch(() => {});
  };
}

export function createWebBackend(token: string, busUrl?: string): ElectronAPI {
  const client = new HubBusClient(token, busUrl);
  client.start();

  // Base for the hub's HTTP routes (e.g. /plugins/settings). The web build is
  // served by the hub, so an empty base resolves relative to the page origin;
  // when an explicit bus URL is given (ws[s]://host/bus) we derive the matching
  // http[s]://host origin from it. The token authorizes the guarded routes.
  const hubHttpBase = busUrl
    ? busUrl.replace(/^ws/, 'http').replace(/\/bus\/?(\?.*)?$/, '')
    : '';
  const hubAuth = { Authorization: `Bearer ${token}` };

  // Claude panes key their byte stream + input by a "viewerKey": the sessionId
  // for a pane that spawned the session, but the *paneId* for a pane attached to
  // an already-running session (so multiple viewers can coexist). The desktop
  // preload resolves that key to a MessagePort; on the bus everything is keyed by
  // sessionId, so we map viewerKey → sessionId here (populated by attachClaude).
  const viewerSessions = new Map<string, string>();
  const sessionFor = (viewerKey: string): string => viewerSessions.get(viewerKey) ?? viewerKey;

  // sessionId → debounced "re-attach + replay" trigger, registered by each live
  // PTY stream (see streamPty). Fired on resize so a freshly-shown pane repaints.
  const reprimers = new Map<string, () => void>();
  const reprime = (sessionId: string): void => reprimers.get(sessionId)?.();

  // After a reconnect the bus re-asserts topic subscriptions, but the per-stream
  // attachTerminal call (which makes claudemon replay the current screen) is not
  // re-issued — so every mirrored terminal would sit frozen until a manual
  // refresh. Re-prime each live PTY stream to re-attach and repaint.
  client.onReconnect(() => {
    for (const reprime of reprimers.values()) reprime();
  });

  // Fan hub events out to the renderer's onHubEvent subscribers. Full session
  // snapshots arrive as `agent.snapshot` events and are routed directly in
  // onClaudeSessionUpdate below.
  const hubEventHandlers = new Set<(ev: HubEventEnvelope) => void>();
  client.subscribe('*', (ev) => {
    for (const h of hubEventHandlers) h(ev);
  });

  const api: ElectronAPI = {
    platform: 'web' as unknown as NodeJS.Platform,

    // No native window chrome in the browser mirror.
    setTitleBarOverlay: () => {},

    // ── Shell terminals ──────────────────────────────────────────────────
    createTerminal: (shell, cwd, cols, rows) =>
      client.call<{ sessionId: string }>('terminals.create', { shell, cwd, cols, rows }).then((r) => r.sessionId),
    writeTerminal: (id, data) => { client.call('sessions.terminalInput', { sessionId: id, data }).catch(() => {}); },
    resizeTerminal: (id, cols, rows) => { reprime(id); return client.call<void>('sessions.terminalResize', { sessionId: id, cols, rows }); },
    closeTerminal: (id) => client.call<void>('sessions.detachTerminal', { sessionId: id }).then(() => {}),
    onTerminalOutput: (id, callback) => streamPty(client, reprimers, id, callback),
    // Terminal exit arrives as a flat `pty.exit` bus event carrying { sessionId };
    // the desktop fires one global callback that each pane filters by its own id,
    // so we mirror that — one subscription per listener, dispatched by sessionId.
    onTerminalExit: (callback) =>
      client.subscribe('pty.exit', (ev) => {
        const id = (ev.data as { sessionId?: string } | undefined)?.sessionId;
        if (id) callback(id);
      }),

    // ── Claude sessions ──────────────────────────────────────────────────
    spawnClaude: (opts) => client.call<{ sessionId: string }>('agents.spawn', opts).then((r) => r.sessionId),
    claudeListModels: () => client.call('claude.listModels', {}),
    // Reads a local transcript file; not available over the hub bus (web mirror).
    workflowAgentTranscript: async () => null,
    workflowAgentConversation: async () => null,
    // Live per-provider discovery over the bus (providers.* capabilities): the
    // managed provider's model catalog and PATH-detection status, so the web
    // Spawn dialog matches the desktop instead of falling back to free-text.
    providerListModels: (provider, cwd) => client.call('providers.listModels', { provider, cwd }),
    providerCheckAll: () => client.call('providers.checkAll', {}),
    claudeMessage: (sessionId, text) => client.call<{ ok: boolean; mode?: string }>('agents.sendMessage', { sessionId, text }),
    claudeSetPermissionMode: (sessionId, mode) =>
      client.call<{ ok: boolean; mode?: string; error?: string }>('claude.setPermissionMode', { sessionId, mode }),
    claudeSetModel: (sessionId, model, effort) =>
      client.call<{ ok: boolean; error?: string }>('claude.setModel', { sessionId, model, effort }),
    claudeHandoffBrief: (sessionId) =>
      client.call<{ ok: boolean; markdown?: string; path?: string; error?: string }>('claude.handoffBrief', { sessionId }),
    claudeHandoffAgentBrief: (sessionId) =>
      client.call<{ ok: boolean; path?: string; fallback?: boolean; error?: string }>('claude.handoffAgentBrief', { sessionId }),
    claudeApprove: (sessionId, decision, reason) => client.call<void>('claude.approve', { sessionId, decision, reason }).then(() => {}),
    claudeAnswer: (sessionId, payload) => client.call<void>('claude.answer', { sessionId, ...payload }).then(() => {}),
    claudeResize: (sessionId, cols, rows) => { reprime(sessionId); return client.call<void>('sessions.terminalResize', { sessionId, cols, rows }).then(() => {}); },
    claudeSignal: (sessionId, signal) => client.call<void>('claude.signal', { sessionId, signal }).then(() => {}),
    claudeClose: (sessionId) => client.call<void>('claude.signal', { sessionId, signal: 'SIGTERM' }).then(() => {}),
    attachClaude: (paneId, sessionId) => { viewerSessions.set(paneId, sessionId); return Promise.resolve(sessionId); },
    detachClaude: (paneId) => { viewerSessions.delete(paneId); return Promise.resolve(); }, // stream lifetime owned by onClaudeOutput's teardown
    claudeGate: (sessionId, on) => client.call<void>('claude.gate', { sessionId, on }).then(() => {}),
    claudeWrite: (viewerKey, data) => { client.call('sessions.terminalInput', { sessionId: sessionFor(viewerKey), data }).catch(() => {}); },
    onClaudeOutput: (viewerKey, callback) => streamPty(client, reprimers, sessionFor(viewerKey), callback),

    // ── Files (editor pane) ──────────────────────────────────────────────
    readFile: (filePath) => client.call<{ path: string; contents: string; size: number }>('fs.read', { path: filePath }),
    writeFile: (filePath, contents) => client.call<{ ok: boolean }>('fs.write', { path: filePath, contents }),
    readDir: (dirPath) => client.call<{ path: string; entries: { name: string; path: string; isDir: boolean }[] }>('fs.listEntries', { path: dirPath }),

    // Start the host-side watch, then subscribe to the bus topic that watch
    // publishes (fs.changed, payload { path, eventType }) and filter by path.
    // Unsub stops the watch and drops the bus subscription.
    watchFile: (path, onChange) => {
      client.call('fs.watch', { path }).catch(() => {});
      const off = client.subscribe('fs.changed', (ev) => {
        const info = (ev.data ?? {}) as { path?: string; eventType?: 'change' | 'rename' };
        if (info.path === path && info.eventType) onChange({ path: info.path, eventType: info.eventType });
      });
      return () => {
        off();
        client.call('fs.unwatch', { path }).catch(() => {});
      };
    },

    searchProject: (opts) =>
      client.call<{ results: { file: string; matches: { line: number; column: number; text: string }[] }[]; truncated: boolean }>('search.project', opts),

    // ── Git (review pane) ────────────────────────────────────────────────
    // The hub capabilities wrap their payloads ({ diff }, { files }, { output });
    // unwrap here so both transports present the same flat shapes to GitClient.
    gitStatus: (cwd) => client.call<import('../../../main/shared/ipcTypes').GitStatus>('git.status', { cwd }),
    gitDiff: (cwd, path, staged, untracked) =>
      client.call<{ diff: string }>('git.diff', { cwd, path, staged, untracked }).then((r) => r.diff),
    gitNumstat: (cwd, staged) =>
      client.call<{ files: import('../../../main/shared/ipcTypes').GitNumstatEntry[] }>('git.numstat', { cwd, staged }).then((r) => r.files),
    gitStage: (cwd, path) => client.call<{ output: string }>('git.stage', { cwd, path }).then((r) => r.output),
    gitUnstage: (cwd, path) => client.call<{ output: string }>('git.unstage', { cwd, path }).then((r) => r.output),
    gitCommit: (cwd, message) => client.call<{ output: string }>('git.commit', { cwd, message }).then((r) => r.output),
    gitPush: (cwd) => client.call<{ output: string }>('git.push', { cwd }).then((r) => r.output),

    // ── Config ───────────────────────────────────────────────────────────
    getConfig: () => client.call<AppConfig>('config.get', {}),
    reloadConfig: () => client.call<AppConfig>('config.reload', {}),
    getConfigPath: () => client.call<string>('config.getPath', {}),
    saveConfig: (partial: AppConfigPartial) => client.call<AppConfig>('config.save', partial),

    // ── Sessions / analytics / layouts ───────────────────────────────────
    listSessions: () => client.call<SessionListEntry[]>('sessions.list', {}),
    loadSession: (filename) => client.call('sessions.load', { filename }),
    saveSession: (data) => client.call<string>('sessions.save', data),
    deleteSession: (filename) => client.call<void>('sessions.delete', { filename }).then(() => {}),
    analyticsSummary: (provider) => client.call('analytics.summary', { provider }),
    analyticsRecent: (limit, provider) => client.call('analytics.recent', { limit, provider }),
    layoutsList: () => client.call('layouts.list', {}),
    layoutsSave: (layout) => client.call('layouts.save', layout),
    layoutsDelete: (id) => client.call<void>('layouts.delete', { id }).then(() => {}),

    // ── Claude discovery / profiles ──────────────────────────────────────
    claudeListSessionsForDir: (cwd) => client.call('claude.sessionsForDir', { cwd }),
    claudeProfilesList: () => client.call<ClaudeProfile[]>('claude.profiles.list', {}),
    claudeProfilesAdd: (name, configDir, extraArgs, mcpItemIds) => client.call<ClaudeProfile>('claude.profiles.add', { name, configDir, extraArgs, mcpItemIds }),
    claudeProfilesUpdate: (id, updates) => client.call<ClaudeProfile>('claude.profiles.update', { id, updates }),
    claudeProfilesRemove: (id) => client.call<void>('claude.profiles.remove', { id }).then(() => {}),
    getClaudeSession: (sessionId) => client.call<ClaudeSessionSnapshot | null>('sessions.snapshot', { sessionId }),
    getAllClaudeSessions: () => client.call<ClaudeSessionSnapshot[]>('sessions.snapshots', {}).then((list) => list || []),
    onClaudeSessionUpdate: (callback) =>
      client.subscribe('agent.snapshot', (ev) => {
        const snap = ev.data as ClaudeSessionSnapshot | undefined;
        if (snap?.sessionId) callback(snap.sessionId, snap);
      }),

    // ── Hub plumbing ─────────────────────────────────────────────────────
    onHubEvent: (callback) => { hubEventHandlers.add(callback); return () => hubEventHandlers.delete(callback); },
    onHubStatus: (callback) => client.onStatus((connected) => callback({ connected })),

    // ── Shared layout document (hub-owned; tmux-style mirror) ────────────────
    // The hub provides layout.get/layout.set in-process and broadcasts
    // layout.changed; the desktop reaches these through main, the web reaches
    // them straight off the bus. Identical surface either way.
    layoutGet: () => client.call('layout.get', {}),
    layoutSet: (data) => client.call('layout.set', { data }),
    onLayoutChanged: (callback) =>
      client.subscribe('layout.changed', (ev) => callback(ev.data as { version: number; data: unknown })),
    getHubStatus: () => Promise.resolve({ connected: client.isConnected() }),
    getRemoteInfo: () => Promise.resolve({ enabled: true, token, remoteUrl: location.href, appUrl: location.href, busUrl: '', desktopBus: false }),
    // A web/remote client exists only because the host already enabled sharing,
    // and it can't restart the host's hub — so this is a no-op that reports on.
    setRemoteShare: () => { warnOnce('setRemoteShare'); return Promise.resolve({ enabled: true, token, remoteUrl: location.href, appUrl: location.href, busUrl: '', desktopBus: false }); },
    listHubPlugins: () => { warnOnce('listHubPlugins'); return Promise.resolve([]); },
    hubPublish: (event) => client.call<void>('__publish', event).then(() => {}).catch(() => {}),
    installPlugin: () => Promise.resolve({ ok: false, error: 'not available over hub' }),
    inspectPlugin: () => Promise.resolve({ ok: false, error: 'not available over hub' }),
    listExamplePlugins: () => { warnOnce('listExamplePlugins'); return Promise.resolve([]); },
    installExamplePlugin: () => Promise.resolve({ ok: false, error: 'not available over hub' }),
    removePlugin: () => Promise.resolve({ ok: false, error: 'not available over hub' }),
    setPluginEnabled: () => Promise.resolve({ ok: false, error: 'not available over hub' }),
    // Minting pane tokens is a trusted-host action (it talks to the hub's
    // guarded route); a web/remote client can't, so it keeps its existing token.
    pluginPaneToken: () => Promise.resolve(null),
    revokePluginPaneToken: () => Promise.resolve(),
    // Plugin settings live on the hub (the single source of truth, merged over
    // manifest defaults). The web client reads/writes them over the hub's guarded
    // HTTP route and hears about any edit — its own, the desktop's, or another
    // remote client's — on the plugin.settings.changed bus event.
    getPluginSettings: async (pluginId: string) => {
      try {
        const res = await fetch(`${hubHttpBase}/plugins/settings?pluginId=${encodeURIComponent(pluginId)}`, { headers: hubAuth });
        if (!res.ok) return {};
        const body = await res.json() as { values?: Record<string, unknown> };
        return body?.values ?? {};
      } catch {
        return {};
      }
    },
    setPluginSettings: async (pluginId: string, values: Record<string, unknown>) => {
      try {
        const res = await fetch(`${hubHttpBase}/plugins/settings`, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json', ...hubAuth },
          body: JSON.stringify({ pluginId, values }),
        });
        if (!res.ok) return values;
        const body = await res.json() as { values?: Record<string, unknown> };
        return body?.values ?? values;
      } catch {
        return values;
      }
    },
    onPluginSettingsChanged: (callback) =>
      client.subscribe('plugin.settings.changed', (ev) => {
        const d = ev.data as { id?: string; values?: Record<string, unknown> } | undefined;
        if (d?.id) callback(d.id, d.values ?? {});
      }),

    // ── Library ──────────────────────────────────────────────────────────
    libraryList: (cwd) => client.call('library.list', { cwd }),
    librarySave: (input) => client.call('library.save', input),
    libraryRemove: (scope, id, cwd, kind) => client.call<void>('library.remove', { scope, id, cwd, kind }).then(() => {}),
    // The desktop mirrors every library mutation (and external edit its watcher
    // catches) onto the bus as a flat `library.changed` event; subscribe so the
    // web client auto-refreshes its prompt/skill list just like the desktop.
    onLibraryChanged: (callback) => client.subscribe('library.changed', () => callback()),

    // ── App info / dialogs ───────────────────────────────────────────────
    getCwd: () => client.call<string>('app.getCwd', {}),
    getSupervisorHome: () => client.call<string>('app.supervisorHome', {}),
    // No native OS dialog over the bus (it'd open on the host, not the viewer).
    // pickFolder opens our in-app host filesystem browser (WebFolderPicker,
    // mounted in App) by dispatching an event it resolves; fsListDir backs it.
    fsListDir: (p) => client.call('fs.listDir', { path: p }),
    pickFolder: (defaultPath) =>
      new Promise<string | null>((resolve) => {
        window.dispatchEvent(new CustomEvent('web:pick-folder', { detail: { defaultPath, resolve } }));
      }),
    pickFiles: () => {
      const p = window.prompt('File paths to attach (comma-separated, on the host):', '');
      if (!p || !p.trim()) return Promise.resolve([]);
      return Promise.resolve(p.split(',').map((s) => s.trim()).filter(Boolean));
    },
    importChromeCookies: () => Promise.resolve({ imported: 0, skipped: 0, errors: ['not available on web'] }),

    // ── Lifecycle / ambient ──────────────────────────────────────────────
    onBeforeQuit: () => () => {},
    notifyQuitSaved: () => {}, // no quit handshake in the browser
    setActiveSession: () => { /* no ambient OS notifications on web */ },
    onFocusAgent: () => () => {},
    onSystemNotice: () => () => {}, // host-process notices; not relevant to the web client
    openLogsFolder: () => { warnOnce('openLogsFolder'); return Promise.resolve({ ok: false, error: 'not available on web' }); },
  };

  return api;
}
