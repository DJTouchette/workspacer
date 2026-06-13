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
 * hub already supports (agents, terminal I/O, approvals, transcript) are wired;
 * everything else returns a safe default and warns once, to be filled in as the
 * hub RPC surface widens (Phase 3). Each stub is marked `HUB-TODO`.
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

export function createWebBackend(token: string): ElectronAPI {
  const client = new HubBusClient(token);
  client.start();

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

  // Fan hub events out to the renderer's onHubEvent subscribers. Full session
  // snapshots arrive as `agent.snapshot` events and are routed directly in
  // onClaudeSessionUpdate below.
  const hubEventHandlers = new Set<(ev: HubEventEnvelope) => void>();
  client.subscribe('*', (ev) => {
    for (const h of hubEventHandlers) h(ev);
  });

  const api: ElectronAPI = {
    platform: 'web' as unknown as NodeJS.Platform,

    // ── Shell terminals ──────────────────────────────────────────────────
    createTerminal: (shell, cwd, cols, rows) =>
      client.call<{ sessionId: string }>('terminals.create', { shell, cwd, cols, rows }).then((r) => r.sessionId),
    writeTerminal: (id, data) => { client.call('sessions.terminalInput', { sessionId: id, data }).catch(() => {}); },
    resizeTerminal: (id, cols, rows) => { reprime(id); return client.call<void>('sessions.terminalResize', { sessionId: id, cols, rows }); },
    closeTerminal: (id) => client.call<void>('sessions.detachTerminal', { sessionId: id }).then(() => {}),
    onTerminalOutput: (id, callback) => streamPty(client, reprimers, id, callback),
    onTerminalExit: () => { warnOnce('onTerminalExit'); return () => {}; }, // HUB-TODO: no exit event yet

    // ── Claude sessions ──────────────────────────────────────────────────
    spawnClaude: (opts) => client.call<{ sessionId: string }>('agents.spawn', opts).then((r) => r.sessionId),
    claudeListModels: () => client.call('claude.listModels', {}),
    claudeMessage: (sessionId, text) => client.call<{ ok: boolean; mode?: string }>('agents.sendMessage', { sessionId, text }),
    claudeApprove: (sessionId, decision, reason) => client.call<void>('claude.approve', { sessionId, decision, reason }).then(() => {}),
    claudeAnswer: (sessionId, payload) => client.call<void>('claude.answer', { sessionId, ...payload }).then(() => {}),
    claudeResize: (sessionId, cols, rows) => { reprime(sessionId); return client.call<void>('sessions.terminalResize', { sessionId, cols, rows }).then(() => {}); },
    claudeSignal: (sessionId, signal) => client.call<void>('claude.signal', { sessionId, signal }).then(() => {}),
    claudeClose: (sessionId) => client.call<void>('claude.signal', { sessionId, signal: 'SIGTERM' }).then(() => {}),
    attachClaude: (paneId, sessionId) => { viewerSessions.set(paneId, sessionId); return Promise.resolve(sessionId); },
    detachClaude: (paneId) => { viewerSessions.delete(paneId); return Promise.resolve(); }, // stream lifetime owned by onClaudeOutput's teardown
    claudeGate: (sessionId, on) => client.call<void>('claude.gate', { sessionId, on }).then(() => {}),
    claudeSummarize: (sessionId, steps) => client.call<string | null>('claude.summarize', { sessionId, steps }).catch(() => null),
    claudeWrite: (viewerKey, data) => { client.call('sessions.terminalInput', { sessionId: sessionFor(viewerKey), data }).catch(() => {}); },
    onClaudeOutput: (viewerKey, callback) => streamPty(client, reprimers, sessionFor(viewerKey), callback),

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
    analyticsSummary: () => client.call('analytics.summary', {}),
    analyticsRecent: (limit) => client.call('analytics.recent', { limit }),
    layoutsList: () => client.call('layouts.list', {}),
    layoutsSave: (layout) => client.call('layouts.save', layout),
    layoutsDelete: (id) => client.call<void>('layouts.delete', { id }).then(() => {}),

    // ── Claude discovery / profiles ──────────────────────────────────────
    claudeListSessionsForDir: (cwd) => client.call('claude.sessionsForDir', { cwd }),
    claudeProfilesList: () => client.call<ClaudeProfile[]>('claude.profiles.list', {}),
    claudeProfilesAdd: (name, configDir, extraArgs) => client.call<ClaudeProfile>('claude.profiles.add', { name, configDir, extraArgs }),
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
    getRemoteInfo: () => Promise.resolve({ enabled: true, token, remoteUrl: location.href, appUrl: location.href, busUrl: '' }),
    listHubPlugins: () => { warnOnce('listHubPlugins'); return Promise.resolve([]); },
    hubPublish: (event) => client.call<void>('__publish', event).then(() => {}).catch(() => {}),
    installPlugin: () => Promise.resolve({ ok: false, error: 'not available over hub' }),
    removePlugin: () => Promise.resolve({ ok: false, error: 'not available over hub' }),

    // ── Library ──────────────────────────────────────────────────────────
    libraryList: (cwd) => client.call('library.list', { cwd }),
    librarySave: (input) => client.call('library.save', input),
    libraryRemove: (scope, id, cwd, kind) => client.call<void>('library.remove', { scope, id, cwd, kind }).then(() => {}),
    onLibraryChanged: () => () => {}, // no hub library-change event yet; web won't auto-refresh

    // ── App info / dialogs ───────────────────────────────────────────────
    getCwd: () => client.call<string>('app.getCwd', {}),
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
    setActiveSession: () => { /* no ambient OS notifications on web */ },
    onFocusAgent: () => () => {},
  };

  return api;
}
