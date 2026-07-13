import { contextBridge, ipcRenderer } from 'electron';
import { IPC } from './shared/ipcChannels';
import type {
  ClaudeSessionSnapshot,
  AppConfig,
  AppConfigPartial,
  SessionData,
  LayoutInput,
  ProfileUpdate,
  GitStatus,
  GitNumstatEntry,
  GitLogEntry,
  RemoteTokenRecord,
  RemoteTokenScope,
} from './shared/ipcTypes';

// ── MessagePort storage (preload isolated world) ──
// Minimal type for the DOM MessagePort (main tsconfig lacks DOM lib)
interface IPort {
  start(): void;
  close(): void;
  postMessage(data: any): void;
  addEventListener(type: string, listener: (event: any) => void): void;
  removeEventListener(type: string, listener: (event: any) => void): void;
}

interface PortWaiter {
  resolve: (port: IPort) => void;
  reject: (err: Error) => void;
  timer: ReturnType<typeof setTimeout>;
}

const terminalPorts = new Map<string, IPort>();
const portWaiters = new Map<string, Array<PortWaiter>>();

/** Timeout (ms) before a getPort() promise is rejected if the port never arrives. */
const PORT_TIMEOUT_MS = 10_000;

function deliverPort(id: string, port: IPort): void {
  terminalPorts.set(id, port);
  port.start();
  const waiters = portWaiters.get(id);
  if (waiters) {
    portWaiters.delete(id);
    for (const { resolve, timer } of waiters) {
      clearTimeout(timer);
      resolve(port);
    }
  }
}

// Receive ports sent by main process after terminal creation (regular shells)
ipcRenderer.on(IPC.TERMINAL_PORT, (event, { id }: { id: string }) => {
  const port = event.ports[0] as unknown as IPort;
  if (!port) return;
  deliverPort(id, port);
});

// Receive Claude session byte-stream ports. Keyed by viewerKey:
//   - For spawned panes (1 viewer per session), viewerKey === sessionId.
//   - For attached panes, viewerKey === paneId so multiple viewers of the
//     same session each get their own port.
ipcRenderer.on(IPC.CLAUDE_PORT, (event, payload: { sessionId: string; viewerKey?: string }) => {
  const port = event.ports[0] as unknown as IPort;
  if (!port) return;
  deliverPort(payload.viewerKey ?? payload.sessionId, port);
});

function getPort(id: string): Promise<IPort> {
  const existing = terminalPorts.get(id);
  if (existing) return Promise.resolve(existing);
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      const list = portWaiters.get(id);
      if (list) {
        const remaining = list.filter((w) => w.timer !== timer);
        if (remaining.length > 0) {
          portWaiters.set(id, remaining);
        } else {
          portWaiters.delete(id);
        }
      }
      reject(new Error(`getPort timeout: port for '${id}' never arrived`));
    }, PORT_TIMEOUT_MS);
    const waiter: PortWaiter = { resolve, reject, timer };
    const list = portWaiters.get(id) ?? [];
    list.push(waiter);
    portWaiters.set(id, list);
  });
}

contextBridge.exposeInMainWorld('electronAPI', {
  // Host OS — lets the renderer reserve space for native window controls
  // (the Windows titleBarOverlay caption buttons live in the top-right corner).
  platform: process.platform,

  // Re-color the Windows native caption buttons to match the active theme.
  // No-op off Windows (the main handler guards too).
  setTitleBarOverlay: (color: string, symbolColor: string): void =>
    ipcRenderer.send(IPC.WINDOW_SET_OVERLAY, { color, symbolColor }),

  // Terminal — control messages stay on IPC, I/O goes through MessagePort
  createTerminal: (shell: string, cwd?: string, cols?: number, rows?: number): Promise<string> =>
    ipcRenderer.invoke(IPC.TERMINAL_CREATE, shell, cwd, cols, rows),

  writeTerminal: (id: string, data: string): void => {
    const port = terminalPorts.get(id);
    if (port) port.postMessage(data);
  },

  resizeTerminal: (id: string, cols: number, rows: number): Promise<void> =>
    ipcRenderer.invoke(IPC.TERMINAL_RESIZE, id, cols, rows),

  closeTerminal: (id: string): Promise<void> =>
    ipcRenderer.invoke(IPC.TERMINAL_CLOSE, id).then(() => {
      const port = terminalPorts.get(id);
      if (port) {
        port.close();
        terminalPorts.delete(id);
      }
    }),

  // Per-terminal output via MessagePort — waits for port, then connects callback
  onTerminalOutput: (id: string, callback: (data: string) => void): (() => void) => {
    let cancelled = false;
    let handler: ((event: any) => void) | null = null;
    let portRef: IPort | null = null;
    getPort(id)
      .then((port) => {
        // Don't close on cancel: the port is cached in terminalPorts and shared
        // with writeTerminal and any re-subscriber. Its lifecycle is owned by
        // closeTerminal. Just skip attaching our listener.
        if (cancelled) return;
        portRef = port;
        handler = (event: any) => callback(event.data);
        port.addEventListener('message', handler);
      })
      .catch(() => {}); // port never arrived (timeout) — nothing to attach
    return () => {
      cancelled = true;
      if (portRef && handler) portRef.removeEventListener('message', handler);
    };
  },

  onTerminalExit: (callback: (id: string) => void): (() => void) => {
    const handler = (_event: Electron.IpcRendererEvent, id: string) => {
      callback(id);
    };
    ipcRenderer.on(IPC.TERMINAL_EXIT, handler);
    return () => {
      ipcRenderer.removeListener(IPC.TERMINAL_EXIT, handler);
    };
  },

  // Config
  getConfig: (): Promise<AppConfig> => ipcRenderer.invoke(IPC.CONFIG_GET),

  reloadConfig: (): Promise<AppConfig> => ipcRenderer.invoke(IPC.CONFIG_RELOAD),

  getConfigPath: (): Promise<string> => ipcRenderer.invoke(IPC.CONFIG_GET_PATH),

  saveConfig: (partial: AppConfigPartial): Promise<AppConfig> =>
    ipcRenderer.invoke(IPC.CONFIG_SAVE, partial),

  // Session
  listSessions: (): Promise<unknown[]> => ipcRenderer.invoke(IPC.SESSION_LIST),

  loadSession: (filename: string): Promise<unknown> =>
    ipcRenderer.invoke(IPC.SESSION_LOAD, filename),

  saveSession: (data: SessionData): Promise<string> => ipcRenderer.invoke(IPC.SESSION_SAVE, data),

  deleteSession: (filename: string): Promise<void> =>
    ipcRenderer.invoke(IPC.SESSION_DELETE, filename),

  // ── Analytics ──
  analyticsSummary: (provider?: string, since?: string): Promise<unknown> =>
    ipcRenderer.invoke(IPC.ANALYTICS_SUMMARY, provider, since),
  analyticsRecent: (limit?: number, provider?: string, since?: string): Promise<unknown[]> =>
    ipcRenderer.invoke(IPC.ANALYTICS_RECENT, limit, provider, since),

  // ── Layout templates ──
  layoutsList: (): Promise<unknown[]> => ipcRenderer.invoke(IPC.LAYOUTS_LIST),
  layoutsSave: (layout: LayoutInput): Promise<unknown> =>
    ipcRenderer.invoke(IPC.LAYOUTS_SAVE, layout),
  layoutsDelete: (id: string): Promise<void> => ipcRenderer.invoke(IPC.LAYOUTS_DELETE, id),

  // ── Claude sessions (delegated to claudemon daemon) ──
  spawnClaude: (opts: {
    cwd?: string;
    provider?: 'claude' | 'codex' | 'opencode' | 'pi';
    /** Claude only: 'pty' (classic TUI) or 'stream' (headless stream-json).
     *  Omitted = the config default (claude.transport). */
    transport?: 'pty' | 'stream';
    profileId?: string;
    model?: string;
    effort?: string;
    permissionMode?: string;
    skipPermissions?: boolean;
    resumeSessionId?: string;
    cols?: number;
    rows?: number;
    supervisor?: boolean;
    label?: string;
    parentSessionId?: string;
    mcpItemIds?: string[];
  }): Promise<string> => ipcRenderer.invoke(IPC.CLAUDE_SPAWN, opts),
  claudeListModels: (): Promise<{
    defaultModel: string;
    skipPermissionsDefault: boolean;
    defaultPermissionMode: string;
    aliases: Array<{ value: string; label: string; context?: string }>;
    seen: string[];
  }> => ipcRenderer.invoke(IPC.CLAUDE_LIST_MODELS),
  workflowAgentTranscript: (
    sessionId: string,
    runId: string | null,
    agentId: string,
  ): Promise<{ role: string; text: string }[] | null> =>
    ipcRenderer.invoke(IPC.WORKFLOW_AGENT_TRANSCRIPT, sessionId, runId, agentId),
  workflowAgentConversation: (
    sessionId: string,
    runId: string | null,
    agentId: string,
  ): Promise<unknown[] | null> =>
    ipcRenderer.invoke(IPC.WORKFLOW_AGENT_CONVERSATION, sessionId, runId, agentId),
  providerListModels: (
    provider: 'codex' | 'opencode' | 'pi',
    cwd?: string,
  ): Promise<Array<{ id: string; label: string; default: boolean }>> =>
    ipcRenderer.invoke(IPC.PROVIDER_LIST_MODELS, provider, cwd),
  providerCheckAll: (): Promise<
    Array<{ provider: string; found: boolean; resolvedPath: string | null; customBin: string }>
  > => ipcRenderer.invoke(IPC.PROVIDER_CHECK_ALL),
  claudeMessage: (sessionId: string, text: string): Promise<{ ok: boolean; mode?: string }> =>
    ipcRenderer.invoke(IPC.CLAUDE_MESSAGE, sessionId, text),
  claudeSetPermissionMode: (
    sessionId: string,
    mode: string,
  ): Promise<{ ok: boolean; mode?: string; error?: string }> =>
    ipcRenderer.invoke(IPC.CLAUDE_SET_PERMISSION_MODE, sessionId, mode),
  claudeSetModel: (
    sessionId: string,
    model?: string,
    effort?: string,
  ): Promise<{ ok: boolean; error?: string }> =>
    ipcRenderer.invoke(IPC.CLAUDE_SET_MODEL, sessionId, model, effort),
  claudeHandoffBrief: (
    sessionId: string,
  ): Promise<{ ok: boolean; markdown?: string; path?: string; error?: string }> =>
    ipcRenderer.invoke(IPC.CLAUDE_HANDOFF_BRIEF, sessionId),
  claudeHandoffAgentBrief: (
    sessionId: string,
  ): Promise<{ ok: boolean; path?: string; fallback?: boolean; error?: string }> =>
    ipcRenderer.invoke(IPC.CLAUDE_HANDOFF_AGENT_BRIEF, sessionId),
  claudeApprove: (
    sessionId: string,
    decision: 'yes' | 'no' | 'always',
    reason?: string,
  ): Promise<void> => ipcRenderer.invoke(IPC.CLAUDE_APPROVE, sessionId, decision, reason),
  claudeAnswer: (
    sessionId: string,
    payload: { option?: number; text?: string; answers?: string[] },
  ): Promise<void> => ipcRenderer.invoke(IPC.CLAUDE_ANSWER, sessionId, payload),
  claudeResize: (sessionId: string, cols: number, rows: number): Promise<void> =>
    ipcRenderer.invoke(IPC.CLAUDE_RESIZE, sessionId, cols, rows),
  claudeSignal: (sessionId: string, signal: string): Promise<void> =>
    ipcRenderer.invoke(IPC.CLAUDE_SIGNAL, sessionId, signal),
  claudeClose: (sessionId: string): Promise<void> => {
    return ipcRenderer.invoke(IPC.CLAUDE_CLOSE, sessionId).then(() => {
      const port = terminalPorts.get(sessionId);
      if (port) {
        port.close();
        terminalPorts.delete(sessionId);
      }
    });
  },
  /** Subscribe a viewer pane to an existing daemon session — no spawn. */
  attachClaude: (paneId: string, sessionId: string): Promise<string> =>
    ipcRenderer.invoke(IPC.CLAUDE_ATTACH, paneId, sessionId),
  /** Disconnect an attached viewer without affecting the underlying session. */
  detachClaude: (paneId: string): Promise<void> =>
    ipcRenderer.invoke(IPC.CLAUDE_DETACH, paneId).then(() => {
      const port = terminalPorts.get(paneId);
      if (port) {
        port.close();
        terminalPorts.delete(paneId);
      }
    }),
  claudeGate: (sessionId: string, on: boolean): Promise<void> =>
    ipcRenderer.invoke(IPC.CLAUDE_GATE, sessionId, on),

  /** Same shape as onTerminalOutput — works on the Claude byte port. */
  onClaudeOutput: (sessionId: string, callback: (data: string) => void): (() => void) => {
    let cancelled = false;
    let handler: ((event: any) => void) | null = null;
    let portRef: IPort | null = null;
    getPort(sessionId)
      .then((port) => {
        // Don't close on cancel: the port is cached in terminalPorts and shared
        // with claudeWrite and any re-subscriber. Its lifecycle is owned by
        // detachClaude. Just skip attaching our listener.
        if (cancelled) return;
        portRef = port;
        handler = (event: any) => callback(event.data);
        port.addEventListener('message', handler);
      })
      .catch(() => {}); // port never arrived (timeout) — nothing to attach
    return () => {
      cancelled = true;
      if (portRef && handler) portRef.removeEventListener('message', handler);
    };
  },

  /** Write raw input to a Claude session via its MessagePort. */
  claudeWrite: (sessionId: string, data: string): void => {
    const port = terminalPorts.get(sessionId);
    if (port) port.postMessage(data);
  },

  // Claude session discovery
  claudeListSessionsForDir: (
    cwd: string,
  ): Promise<{ sessionId: string; timestamp: string; summary: string }[]> =>
    ipcRenderer.invoke(IPC.CLAUDE_SESSIONS_LIST_FOR_DIR, cwd),

  // Claude profiles
  claudeProfilesList: (): Promise<ProfileUpdate[]> => ipcRenderer.invoke(IPC.CLAUDE_PROFILES_LIST),
  claudeProfilesAdd: (
    name: string,
    configDir: string,
    extraArgs: string[],
    mcpItemIds?: string[],
  ): Promise<ProfileUpdate> =>
    ipcRenderer.invoke(IPC.CLAUDE_PROFILES_ADD, name, configDir, extraArgs, mcpItemIds),
  claudeProfilesUpdate: (id: string, updates: ProfileUpdate): Promise<ProfileUpdate> =>
    ipcRenderer.invoke(IPC.CLAUDE_PROFILES_UPDATE, id, updates),
  claudeProfilesRemove: (id: string): Promise<void> =>
    ipcRenderer.invoke(IPC.CLAUDE_PROFILES_REMOVE, id),

  getClaudeSession: (sessionId: string): Promise<ClaudeSessionSnapshot | null> =>
    ipcRenderer.invoke(IPC.CLAUDE_SESSION_GET, sessionId),

  getAllClaudeSessions: (): Promise<ClaudeSessionSnapshot[]> =>
    ipcRenderer.invoke(IPC.CLAUDE_SESSION_GET_ALL),

  onClaudeSessionUpdate: (
    callback: (sessionId: string, snapshot: ClaudeSessionSnapshot) => void,
  ): (() => void) => {
    const handler = (
      _event: Electron.IpcRendererEvent,
      sessionId: string,
      snapshot: ClaudeSessionSnapshot,
    ) => {
      callback(sessionId, snapshot);
    };
    ipcRenderer.on(IPC.CLAUDE_SESSION_UPDATE, handler);
    return () => {
      ipcRenderer.removeListener(IPC.CLAUDE_SESSION_UPDATE, handler);
    };
  },

  // Hub event bus — events forwarded from the hub daemon's WebSocket.
  onHubEvent: (
    callback: (event: {
      id: string;
      type: string;
      source: string;
      time: string;
      data?: unknown;
    }) => void,
  ): (() => void) => {
    const handler = (
      _e: Electron.IpcRendererEvent,
      ev: { id: string; type: string; source: string; time: string; data?: unknown },
    ) => callback(ev);
    ipcRenderer.on(IPC.HUB_EVENT, handler);
    return () => ipcRenderer.removeListener(IPC.HUB_EVENT, handler);
  },
  onHubStatus: (callback: (status: { connected: boolean }) => void): (() => void) => {
    const handler = (_e: Electron.IpcRendererEvent, s: { connected: boolean }) => callback(s);
    ipcRenderer.on(IPC.HUB_STATUS, handler);
    return () => ipcRenderer.removeListener(IPC.HUB_STATUS, handler);
  },
  listHubPlugins: (): Promise<unknown[]> => ipcRenderer.invoke(IPC.HUB_LIST_PLUGINS),
  hubPublish: (event: { type: string; source?: string; data?: unknown }): Promise<void> =>
    ipcRenderer.invoke(IPC.HUB_PUBLISH, event),
  getHubStatus: (): Promise<{ connected: boolean }> => ipcRenderer.invoke(IPC.HUB_GET_STATUS),

  // ── Git worktrees (agent isolation) ──
  worktreeInfo: (cwd: string): Promise<unknown> => ipcRenderer.invoke(IPC.WORKTREE_INFO, cwd),
  worktreeCreate: (opts: {
    repoCwd: string;
    name?: string;
    rootOverride?: string;
  }): Promise<unknown> => ipcRenderer.invoke(IPC.WORKTREE_CREATE, opts),

  // ── In-app updates (electron-updater; unsupported in dev/web) ──
  updatesGetStatus: (): Promise<unknown> => ipcRenderer.invoke(IPC.UPDATES_STATUS_GET),
  updatesCheck: (): Promise<unknown> => ipcRenderer.invoke(IPC.UPDATES_CHECK),
  updatesInstall: (): Promise<void> => ipcRenderer.invoke(IPC.UPDATES_INSTALL),
  onUpdateStatus: (callback: (status: unknown) => void): (() => void) => {
    const handler = (_e: Electron.IpcRendererEvent, status: unknown) => callback(status);
    ipcRenderer.on(IPC.UPDATES_STATUS, handler);
    return () => ipcRenderer.removeListener(IPC.UPDATES_STATUS, handler);
  },

  // ── Shared layout document (hub-owned; tmux-style mirror) ──
  layoutGet: (): Promise<{ version: number; data: unknown }> => ipcRenderer.invoke(IPC.LAYOUT_GET),
  layoutSet: (data: unknown): Promise<{ version: number; data: unknown }> =>
    ipcRenderer.invoke(IPC.LAYOUT_SET, data),
  onLayoutChanged: (callback: (doc: { version: number; data: unknown }) => void): (() => void) => {
    const handler = (_e: Electron.IpcRendererEvent, doc: { version: number; data: unknown }) =>
      callback(doc);
    ipcRenderer.on(IPC.LAYOUT_CHANGED, handler);
    return () => ipcRenderer.removeListener(IPC.LAYOUT_CHANGED, handler);
  },
  getRemoteInfo: (): Promise<{
    enabled: boolean;
    token: string;
    remoteUrl: string;
    appUrl: string;
    busUrl: string;
    desktopBus: boolean;
    hubAdopted: boolean;
    claudemonAdopted: boolean;
    remoteClient: { httpUrl: string; busUrl: string; token: string } | null;
  }> => ipcRenderer.invoke(IPC.HUB_GET_REMOTE_INFO),
  setRemoteShare: (
    enabled: boolean,
  ): Promise<{
    enabled: boolean;
    token: string;
    remoteUrl: string;
    appUrl: string;
    busUrl: string;
    desktopBus: boolean;
    hubAdopted: boolean;
    claudemonAdopted: boolean;
    remoteClient: { httpUrl: string; busUrl: string; token: string } | null;
  }> => ipcRenderer.invoke(IPC.HUB_SET_REMOTE_SHARE, enabled),
  remoteTokensList: (): Promise<RemoteTokenRecord[]> =>
    ipcRenderer.invoke(IPC.HUB_REMOTE_TOKENS_LIST),
  remoteTokenGetOrCreate: (scope: RemoteTokenScope, label?: string): Promise<RemoteTokenRecord> =>
    ipcRenderer.invoke(IPC.HUB_REMOTE_TOKEN_GET_OR_CREATE, scope, label),
  remoteTokenRevoke: (token: string): Promise<RemoteTokenRecord> =>
    ipcRenderer.invoke(IPC.HUB_REMOTE_TOKEN_REVOKE, token),
  // "Connect to remote server" (client mode): persist/clear the target hub.
  // Applied on relaunch — pair with appRelaunch() after a successful set.
  setRemoteServer: (
    setting: { url: string; token: string } | null,
  ): Promise<{ ok: boolean; error?: string }> =>
    ipcRenderer.invoke(IPC.HUB_SET_REMOTE_SERVER, setting),
  appRelaunch: (): Promise<void> => ipcRenderer.invoke(IPC.APP_RELAUNCH),
  tailscaleGetInfo: (): Promise<{
    available: boolean;
    magicName: string | null;
    serveActive: boolean;
    canServe: boolean;
    hint?: string;
  }> => ipcRenderer.invoke(IPC.TAILSCALE_GET_INFO),
  tailscaleSetServe: (enable: boolean): Promise<{ ok: boolean; error?: string; hint?: string }> =>
    ipcRenderer.invoke(IPC.TAILSCALE_SET_SERVE, enable),
  openLogsFolder: (): Promise<{ ok: boolean; error?: string }> =>
    ipcRenderer.invoke(IPC.LOGS_OPEN_FOLDER),
  // Put the bundled `workspacer` CLI on PATH (runs its own `install-cli`).
  installCli: (): Promise<{ ok: boolean; message: string }> => ipcRenderer.invoke(IPC.CLI_INSTALL),
  // Model pricing overrides (~/.workspacer/model-rates.json).
  pricingGetRates: () => ipcRenderer.invoke(IPC.PRICING_GET),
  pricingSaveOverrides: (overrides: unknown) => ipcRenderer.invoke(IPC.PRICING_SAVE, overrides),
  installPlugin: (url: string): Promise<{ ok: boolean; plugin?: unknown; error?: string }> =>
    ipcRenderer.invoke(IPC.HUB_INSTALL_PLUGIN, url),
  inspectPlugin: (url: string): Promise<{ ok: boolean; plugin?: unknown; error?: string }> =>
    ipcRenderer.invoke(IPC.HUB_INSPECT_PLUGIN, url),
  listExamplePlugins: (): Promise<unknown[]> => ipcRenderer.invoke(IPC.HUB_LIST_EXAMPLES),
  installExamplePlugin: (id: string): Promise<{ ok: boolean; plugin?: unknown; error?: string }> =>
    ipcRenderer.invoke(IPC.HUB_INSTALL_EXAMPLE, id),
  removePlugin: (id: string): Promise<{ ok: boolean; error?: string }> =>
    ipcRenderer.invoke(IPC.HUB_REMOVE_PLUGIN, id),
  setPluginEnabled: (
    id: string,
    enabled: boolean,
  ): Promise<{ ok: boolean; plugin?: unknown; error?: string }> =>
    ipcRenderer.invoke(IPC.HUB_SET_PLUGIN_ENABLED, { id, enabled }),
  // Per-pane scoped token for an agent-scoped plugin pane (confines the webview
  // to the agent's cwd). Returns null on failure → caller keeps the static token.
  pluginPaneToken: (pluginId: string, agentCwd?: string): Promise<string | null> =>
    ipcRenderer.invoke(IPC.HUB_PLUGIN_PANE_TOKEN, pluginId, agentCwd),
  revokePluginPaneToken: (token: string): Promise<void> =>
    ipcRenderer.invoke(IPC.HUB_PLUGIN_PANE_TOKEN_REVOKE, token),
  // Per-plugin settings (declared in the plugin manifest; values persisted here).
  getPluginSettings: (pluginId: string): Promise<Record<string, unknown>> =>
    ipcRenderer.invoke(IPC.HUB_PLUGIN_SETTINGS_GET, pluginId),
  setPluginSettings: (
    pluginId: string,
    values: Record<string, unknown>,
  ): Promise<Record<string, unknown>> =>
    ipcRenderer.invoke(IPC.HUB_PLUGIN_SETTINGS_SET, pluginId, values),
  onPluginSettingsChanged: (
    callback: (pluginId: string, values: Record<string, unknown>) => void,
  ): (() => void) => {
    const handler = (
      _e: Electron.IpcRendererEvent,
      pluginId: string,
      values: Record<string, unknown>,
    ) => callback(pluginId, values);
    ipcRenderer.on(IPC.HUB_PLUGIN_SETTINGS_CHANGED, handler);
    return () => ipcRenderer.removeListener(IPC.HUB_PLUGIN_SETTINGS_CHANGED, handler);
  },

  // ── Library (reusable prompts + skills) ──
  libraryList: (cwd?: string): Promise<unknown[]> => ipcRenderer.invoke(IPC.LIBRARY_LIST, cwd),
  librarySave: (input: unknown): Promise<unknown> => ipcRenderer.invoke(IPC.LIBRARY_SAVE, input),
  libraryRemove: (
    scope: 'global' | 'project' | 'claude',
    id: string,
    cwd?: string,
    kind?: 'prompt' | 'skill' | 'agent',
  ): Promise<void> => ipcRenderer.invoke(IPC.LIBRARY_REMOVE, scope, id, cwd, kind),
  onLibraryChanged: (callback: () => void): (() => void) => {
    const handler = () => callback();
    ipcRenderer.on(IPC.LIBRARY_CHANGED, handler);
    return () => ipcRenderer.removeListener(IPC.LIBRARY_CHANGED, handler);
  },

  // App info
  getCwd: (): Promise<string> => ipcRenderer.invoke(IPC.APP_GET_CWD),
  getSupervisorHome: (): Promise<string> => ipcRenderer.invoke(IPC.APP_SUPERVISOR_HOME),

  // Dialog
  pickFolder: (defaultPath?: string): Promise<string | null> =>
    ipcRenderer.invoke(IPC.DIALOG_PICK_FOLDER, defaultPath),
  pickFiles: (defaultPath?: string): Promise<string[]> =>
    ipcRenderer.invoke(IPC.DIALOG_PICK_FILES, defaultPath),

  // Files (editor pane)
  readFile: (filePath: string): Promise<{ path: string; contents: string; size: number }> =>
    ipcRenderer.invoke(IPC.FILE_READ, filePath),
  writeFile: (filePath: string, contents: string): Promise<{ ok: boolean }> =>
    ipcRenderer.invoke(IPC.FILE_WRITE, filePath, contents),
  readDir: (
    dirPath: string,
  ): Promise<{ path: string; entries: { name: string; path: string; isDir: boolean }[] }> =>
    ipcRenderer.invoke(IPC.FILE_LIST_DIR, dirPath),
  // Open a file with the OS default handler (file:// URL — browser for .html).
  fileOpenExternal: (filePath: string): Promise<{ ok: boolean; error?: string }> =>
    ipcRenderer.invoke(IPC.FILE_OPEN_EXTERNAL, filePath),
  // Reveal a file in the OS file manager.
  fileShowInFolder: (filePath: string): Promise<{ ok: boolean; error?: string }> =>
    ipcRenderer.invoke(IPC.FILE_SHOW_IN_FOLDER, filePath),
  // Open an http(s) URL in the OS default browser (scheme-checked in main).
  openExternalUrl: (url: string): Promise<{ ok: boolean; error?: string }> =>
    ipcRenderer.invoke(IPC.SHELL_OPEN_EXTERNAL, url),

  // Watch a single file. Starts the watch in main and listens on the push
  // channel, filtering by path so multiple watchers on different files don't
  // cross-talk. Returns an unsubscribe that stops the watch + drops the listener.
  watchFile: (
    path: string,
    onChange: (info: { path: string; eventType: 'change' | 'rename' }) => void,
  ): (() => void) => {
    ipcRenderer.invoke(IPC.FILE_WATCH, path);
    const handler = (
      _event: Electron.IpcRendererEvent,
      info: { path: string; eventType: 'change' | 'rename' },
    ) => {
      if (info.path === path) onChange(info);
    };
    ipcRenderer.on(IPC.FILE_CHANGED, handler);
    return () => {
      ipcRenderer.removeListener(IPC.FILE_CHANGED, handler);
      ipcRenderer.invoke(IPC.FILE_UNWATCH, path);
    };
  },

  // Project-wide text search (ripgrep), for the editor's search sidebar.
  searchProject: (opts: {
    query: string;
    cwd: string;
    caseSensitive?: boolean;
    wholeWord?: boolean;
    regex?: boolean;
    maxResults?: number;
  }): Promise<{
    results: { file: string; matches: { line: number; column: number; text: string }[] }[];
    truncated: boolean;
  }> => ipcRenderer.invoke(IPC.SEARCH_PROJECT, opts),

  // Git (review pane) — shells out to git in the main process.
  gitStatus: (cwd: string): Promise<GitStatus> => ipcRenderer.invoke(IPC.GIT_STATUS, cwd),
  gitLog: (cwd: string, limit?: number): Promise<GitLogEntry[]> =>
    ipcRenderer.invoke(IPC.GIT_LOG, cwd, limit),
  gitDiff: (cwd: string, path?: string, staged?: boolean, untracked?: boolean): Promise<string> =>
    ipcRenderer.invoke(IPC.GIT_DIFF, cwd, path, staged, untracked),
  gitNumstat: (cwd: string, staged?: boolean): Promise<GitNumstatEntry[]> =>
    ipcRenderer.invoke(IPC.GIT_NUMSTAT, cwd, staged),
  gitStage: (cwd: string, path?: string): Promise<string> =>
    ipcRenderer.invoke(IPC.GIT_STAGE, cwd, path),
  gitUnstage: (cwd: string, path?: string): Promise<string> =>
    ipcRenderer.invoke(IPC.GIT_UNSTAGE, cwd, path),
  gitCommit: (cwd: string, message: string): Promise<string> =>
    ipcRenderer.invoke(IPC.GIT_COMMIT, cwd, message),
  gitPush: (cwd: string): Promise<string> => ipcRenderer.invoke(IPC.GIT_PUSH, cwd),

  // Browser cookie import (Chrome or Edge)
  importChromeCookies: (
    domainFilter?: string[],
    method?: 'cdp' | 'direct',
    browser?: 'chrome' | 'edge',
  ): Promise<{ imported: number; skipped: number; errors: string[] }> =>
    ipcRenderer.invoke(IPC.CHROME_COOKIES_IMPORT, { domainFilter, method, browser }),

  // App lifecycle
  onBeforeQuit: (callback: () => void): (() => void) => {
    const handler = () => callback();
    ipcRenderer.on(IPC.APP_BEFORE_QUIT, handler);
    return () => {
      ipcRenderer.removeListener(IPC.APP_BEFORE_QUIT, handler);
    };
  },
  /** Ack that the quit-time session save finished — main holds teardown for it. */
  notifyQuitSaved: (): void => ipcRenderer.send(IPC.APP_QUIT_SAVED),

  // Notifications / ambient awareness
  /** Tell main which agent session is currently on screen (null = none). */
  setActiveSession: (sessionId: string | null): void =>
    ipcRenderer.send(IPC.NOTIFY_SET_ACTIVE_SESSION, sessionId),
  /** Fired when the user clicks an OS notification — carries the sessionId. */
  onFocusAgent: (callback: (sessionId: string) => void): (() => void) => {
    const handler = (_event: Electron.IpcRendererEvent, sessionId: string) => callback(sessionId);
    ipcRenderer.on(IPC.NOTIFY_FOCUS_AGENT, handler);
    return () => {
      ipcRenderer.removeListener(IPC.NOTIFY_FOCUS_AGENT, handler);
    };
  },
  onSystemNotice: (
    callback: (notice: {
      level: 'error' | 'warn' | 'info';
      title: string;
      detail?: string;
      key?: string;
    }) => void,
  ): (() => void) => {
    const handler = (
      _event: Electron.IpcRendererEvent,
      notice: { level: 'error' | 'warn' | 'info'; title: string; detail?: string; key?: string },
    ) => callback(notice);
    ipcRenderer.on(IPC.SYSTEM_NOTICE, handler);
    return () => {
      ipcRenderer.removeListener(IPC.SYSTEM_NOTICE, handler);
    };
  },
});
