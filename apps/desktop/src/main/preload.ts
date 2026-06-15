import { contextBridge, ipcRenderer } from 'electron';
import { IPC } from './shared/ipcChannels';
import type { ClaudeSessionSnapshot, AppConfig, AppConfigPartial, SessionData, LayoutInput, ProfileUpdate } from './shared/ipcTypes';

// ── MessagePort storage (preload isolated world) ──
// Minimal type for the DOM MessagePort (main tsconfig lacks DOM lib)
interface IPort {
  start(): void;
  close(): void;
  postMessage(data: any): void;
  addEventListener(type: string, listener: (event: any) => void): void;
  removeEventListener(type: string, listener: (event: any) => void): void;
}

const terminalPorts = new Map<string, IPort>();
const portWaiters = new Map<string, Array<(port: IPort) => void>>();

function deliverPort(id: string, port: IPort): void {
  terminalPorts.set(id, port);
  port.start();
  const waiters = portWaiters.get(id);
  if (waiters) {
    for (const resolve of waiters) resolve(port);
    portWaiters.delete(id);
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
  return new Promise((resolve) => {
    const list = portWaiters.get(id) ?? [];
    list.push(resolve);
    portWaiters.set(id, list);
  });
}

contextBridge.exposeInMainWorld('electronAPI', {
  // Host OS — lets the renderer reserve space for native window controls
  // (the Windows titleBarOverlay caption buttons live in the top-right corner).
  platform: process.platform,

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
      if (port) { port.close(); terminalPorts.delete(id); }
    }),

  // Per-terminal output via MessagePort — waits for port, then connects callback
  onTerminalOutput: (id: string, callback: (data: string) => void): (() => void) => {
    let handler: ((event: any) => void) | null = null;
    let portRef: IPort | null = null;
    getPort(id).then((port) => {
      portRef = port;
      handler = (event: any) => callback(event.data);
      port.addEventListener('message', handler);
    });
    return () => {
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
  getConfig: (): Promise<AppConfig> =>
    ipcRenderer.invoke(IPC.CONFIG_GET),

  reloadConfig: (): Promise<AppConfig> =>
    ipcRenderer.invoke(IPC.CONFIG_RELOAD),

  getConfigPath: (): Promise<string> =>
    ipcRenderer.invoke(IPC.CONFIG_GET_PATH),

  saveConfig: (partial: AppConfigPartial): Promise<AppConfig> =>
    ipcRenderer.invoke(IPC.CONFIG_SAVE, partial),

  // Session
  listSessions: (): Promise<unknown[]> =>
    ipcRenderer.invoke(IPC.SESSION_LIST),

  loadSession: (filename: string): Promise<unknown> =>
    ipcRenderer.invoke(IPC.SESSION_LOAD, filename),

  saveSession: (data: SessionData): Promise<string> =>
    ipcRenderer.invoke(IPC.SESSION_SAVE, data),

  deleteSession: (filename: string): Promise<void> =>
    ipcRenderer.invoke(IPC.SESSION_DELETE, filename),

  // ── Analytics ──
  analyticsSummary: (): Promise<unknown> =>
    ipcRenderer.invoke(IPC.ANALYTICS_SUMMARY),
  analyticsRecent: (limit?: number): Promise<unknown[]> =>
    ipcRenderer.invoke(IPC.ANALYTICS_RECENT, limit),

  // ── Layout templates ──
  layoutsList: (): Promise<unknown[]> =>
    ipcRenderer.invoke(IPC.LAYOUTS_LIST),
  layoutsSave: (layout: LayoutInput): Promise<unknown> =>
    ipcRenderer.invoke(IPC.LAYOUTS_SAVE, layout),
  layoutsDelete: (id: string): Promise<void> =>
    ipcRenderer.invoke(IPC.LAYOUTS_DELETE, id),

  // ── Claude sessions (delegated to claudemon daemon) ──
  spawnClaude: (opts: { cwd?: string; profileId?: string; model?: string; skipPermissions?: boolean; resumeSessionId?: string; cols?: number; rows?: number; supervisor?: boolean; label?: string; parentSessionId?: string }): Promise<string> =>
    ipcRenderer.invoke(IPC.CLAUDE_SPAWN, opts),
  claudeListModels: (): Promise<{ defaultModel: string; skipPermissionsDefault: boolean; aliases: Array<{ value: string; label: string }>; seen: string[] }> =>
    ipcRenderer.invoke(IPC.CLAUDE_LIST_MODELS),
  claudeMessage: (sessionId: string, text: string): Promise<{ ok: boolean; mode?: string }> =>
    ipcRenderer.invoke(IPC.CLAUDE_MESSAGE, sessionId, text),
  claudeApprove: (sessionId: string, decision: 'yes' | 'no' | 'always', reason?: string): Promise<void> =>
    ipcRenderer.invoke(IPC.CLAUDE_APPROVE, sessionId, decision, reason),
  claudeAnswer: (sessionId: string, payload: { option?: number; text?: string; answers?: string[] }): Promise<void> =>
    ipcRenderer.invoke(IPC.CLAUDE_ANSWER, sessionId, payload),
  claudeResize: (sessionId: string, cols: number, rows: number): Promise<void> =>
    ipcRenderer.invoke(IPC.CLAUDE_RESIZE, sessionId, cols, rows),
  claudeSignal: (sessionId: string, signal: string): Promise<void> =>
    ipcRenderer.invoke(IPC.CLAUDE_SIGNAL, sessionId, signal),
  claudeClose: (sessionId: string): Promise<void> => {
    return ipcRenderer.invoke(IPC.CLAUDE_CLOSE, sessionId).then(() => {
      const port = terminalPorts.get(sessionId);
      if (port) { port.close(); terminalPorts.delete(sessionId); }
    });
  },
  /** Subscribe a viewer pane to an existing daemon session — no spawn. */
  attachClaude: (paneId: string, sessionId: string): Promise<string> =>
    ipcRenderer.invoke(IPC.CLAUDE_ATTACH, paneId, sessionId),
  /** Disconnect an attached viewer without affecting the underlying session. */
  detachClaude: (paneId: string): Promise<void> =>
    ipcRenderer.invoke(IPC.CLAUDE_DETACH, paneId).then(() => {
      const port = terminalPorts.get(paneId);
      if (port) { port.close(); terminalPorts.delete(paneId); }
    }),
  claudeGate: (sessionId: string, on: boolean): Promise<void> =>
    ipcRenderer.invoke(IPC.CLAUDE_GATE, sessionId, on),

  /** Same shape as onTerminalOutput — works on the Claude byte port. */
  onClaudeOutput: (sessionId: string, callback: (data: string) => void): (() => void) => {
    let handler: ((event: any) => void) | null = null;
    let portRef: IPort | null = null;
    getPort(sessionId).then((port) => {
      portRef = port;
      handler = (event: any) => callback(event.data);
      port.addEventListener('message', handler);
    });
    return () => {
      if (portRef && handler) portRef.removeEventListener('message', handler);
    };
  },

  /** Write raw input to a Claude session via its MessagePort. */
  claudeWrite: (sessionId: string, data: string): void => {
    const port = terminalPorts.get(sessionId);
    if (port) port.postMessage(data);
  },

  // Claude session discovery
  claudeListSessionsForDir: (cwd: string): Promise<{ sessionId: string; timestamp: string; summary: string }[]> =>
    ipcRenderer.invoke(IPC.CLAUDE_SESSIONS_LIST_FOR_DIR, cwd),

  // Claude profiles
  claudeProfilesList: (): Promise<ProfileUpdate[]> => ipcRenderer.invoke(IPC.CLAUDE_PROFILES_LIST),
  claudeProfilesAdd: (name: string, configDir: string, extraArgs: string[]): Promise<ProfileUpdate> =>
    ipcRenderer.invoke(IPC.CLAUDE_PROFILES_ADD, name, configDir, extraArgs),
  claudeProfilesUpdate: (id: string, updates: ProfileUpdate): Promise<ProfileUpdate> =>
    ipcRenderer.invoke(IPC.CLAUDE_PROFILES_UPDATE, id, updates),
  claudeProfilesRemove: (id: string): Promise<void> =>
    ipcRenderer.invoke(IPC.CLAUDE_PROFILES_REMOVE, id),

  getClaudeSession: (sessionId: string): Promise<ClaudeSessionSnapshot | null> =>
    ipcRenderer.invoke(IPC.CLAUDE_SESSION_GET, sessionId),

  getAllClaudeSessions: (): Promise<ClaudeSessionSnapshot[]> =>
    ipcRenderer.invoke(IPC.CLAUDE_SESSION_GET_ALL),

  onClaudeSessionUpdate: (callback: (sessionId: string, snapshot: ClaudeSessionSnapshot) => void): (() => void) => {
    const handler = (_event: Electron.IpcRendererEvent, sessionId: string, snapshot: ClaudeSessionSnapshot) => {
      callback(sessionId, snapshot);
    };
    ipcRenderer.on(IPC.CLAUDE_SESSION_UPDATE, handler);
    return () => {
      ipcRenderer.removeListener(IPC.CLAUDE_SESSION_UPDATE, handler);
    };
  },

  // Hub event bus — events forwarded from the hub daemon's WebSocket.
  onHubEvent: (callback: (event: { id: string; type: string; source: string; time: string; data?: unknown }) => void): (() => void) => {
    const handler = (_e: Electron.IpcRendererEvent, ev: { id: string; type: string; source: string; time: string; data?: unknown }) => callback(ev);
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

  // ── Shared layout document (hub-owned; tmux-style mirror) ──
  layoutGet: (): Promise<{ version: number; data: unknown }> =>
    ipcRenderer.invoke(IPC.LAYOUT_GET),
  layoutSet: (data: unknown): Promise<{ version: number; data: unknown }> =>
    ipcRenderer.invoke(IPC.LAYOUT_SET, data),
  onLayoutChanged: (callback: (doc: { version: number; data: unknown }) => void): (() => void) => {
    const handler = (_e: Electron.IpcRendererEvent, doc: { version: number; data: unknown }) => callback(doc);
    ipcRenderer.on(IPC.LAYOUT_CHANGED, handler);
    return () => ipcRenderer.removeListener(IPC.LAYOUT_CHANGED, handler);
  },
  getRemoteInfo: (): Promise<{ enabled: boolean; token: string; remoteUrl: string; appUrl: string; busUrl: string }> =>
    ipcRenderer.invoke(IPC.HUB_GET_REMOTE_INFO),
  installPlugin: (url: string): Promise<{ ok: boolean; plugin?: unknown; error?: string }> =>
    ipcRenderer.invoke(IPC.HUB_INSTALL_PLUGIN, url),
  removePlugin: (id: string): Promise<{ ok: boolean; error?: string }> =>
    ipcRenderer.invoke(IPC.HUB_REMOVE_PLUGIN, id),

  // ── Library (reusable prompts + skills) ──
  libraryList: (cwd?: string): Promise<unknown[]> =>
    ipcRenderer.invoke(IPC.LIBRARY_LIST, cwd),
  librarySave: (input: unknown): Promise<unknown> =>
    ipcRenderer.invoke(IPC.LIBRARY_SAVE, input),
  libraryRemove: (scope: 'global' | 'project' | 'claude', id: string, cwd?: string, kind?: 'prompt' | 'skill' | 'agent'): Promise<void> =>
    ipcRenderer.invoke(IPC.LIBRARY_REMOVE, scope, id, cwd, kind),
  onLibraryChanged: (callback: () => void): (() => void) => {
    const handler = () => callback();
    ipcRenderer.on(IPC.LIBRARY_CHANGED, handler);
    return () => ipcRenderer.removeListener(IPC.LIBRARY_CHANGED, handler);
  },

  // App info
  getCwd: (): Promise<string> =>
    ipcRenderer.invoke(IPC.APP_GET_CWD),

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
  readDir: (dirPath: string): Promise<{ path: string; entries: { name: string; path: string; isDir: boolean }[] }> =>
    ipcRenderer.invoke(IPC.FILE_LIST_DIR, dirPath),

  // Watch a single file. Starts the watch in main and listens on the push
  // channel, filtering by path so multiple watchers on different files don't
  // cross-talk. Returns an unsubscribe that stops the watch + drops the listener.
  watchFile: (
    path: string,
    onChange: (info: { path: string; eventType: 'change' | 'rename' }) => void,
  ): (() => void) => {
    ipcRenderer.invoke(IPC.FILE_WATCH, path);
    const handler = (_event: Electron.IpcRendererEvent, info: { path: string; eventType: 'change' | 'rename' }) => {
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
  }): Promise<{ results: { file: string; matches: { line: number; column: number; text: string }[] }[]; truncated: boolean }> =>
    ipcRenderer.invoke(IPC.SEARCH_PROJECT, opts),


  // Browser cookie import (Chrome or Edge)
  importChromeCookies: (domainFilter?: string[], method?: 'cdp' | 'direct', browser?: 'chrome' | 'edge'): Promise<{ imported: number; skipped: number; errors: string[] }> =>
    ipcRenderer.invoke(IPC.CHROME_COOKIES_IMPORT, { domainFilter, method, browser }),

  // App lifecycle
  onBeforeQuit: (callback: () => void): (() => void) => {
    const handler = () => callback();
    ipcRenderer.on(IPC.APP_BEFORE_QUIT, handler);
    return () => {
      ipcRenderer.removeListener(IPC.APP_BEFORE_QUIT, handler);
    };
  },

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
});
