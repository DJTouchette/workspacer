import { contextBridge, ipcRenderer } from 'electron';

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
ipcRenderer.on('terminal:port', (event, { id }: { id: string }) => {
  const port = event.ports[0] as unknown as IPort;
  if (!port) return;
  deliverPort(id, port);
});

// Receive Claude session byte-stream ports. Keyed by viewerKey:
//   - For spawned panes (1 viewer per session), viewerKey === sessionId.
//   - For attached panes, viewerKey === paneId so multiple viewers of the
//     same session each get their own port.
ipcRenderer.on('claude:port', (event, payload: { sessionId: string; viewerKey?: string }) => {
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
  // Terminal — control messages stay on IPC, I/O goes through MessagePort
  createTerminal: (shell: string, cwd?: string, cols?: number, rows?: number): Promise<string> =>
    ipcRenderer.invoke('terminal:create', shell, cwd, cols, rows),

  writeTerminal: (id: string, data: string): void => {
    const port = terminalPorts.get(id);
    if (port) port.postMessage(data);
  },

  resizeTerminal: (id: string, cols: number, rows: number): Promise<void> =>
    ipcRenderer.invoke('terminal:resize', id, cols, rows),

  closeTerminal: (id: string): Promise<void> =>
    ipcRenderer.invoke('terminal:close', id).then(() => {
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
    ipcRenderer.on('terminal:exit', handler);
    return () => {
      ipcRenderer.removeListener('terminal:exit', handler);
    };
  },

  // Config
  getConfig: (): Promise<unknown> =>
    ipcRenderer.invoke('config:get'),

  reloadConfig: (): Promise<unknown> =>
    ipcRenderer.invoke('config:reload'),

  getConfigPath: (): Promise<string> =>
    ipcRenderer.invoke('config:getPath'),

  saveConfig: (partial: unknown): Promise<unknown> =>
    ipcRenderer.invoke('config:save', partial),

  // Session
  listSessions: (): Promise<unknown[]> =>
    ipcRenderer.invoke('session:list'),

  loadSession: (filename: string): Promise<unknown> =>
    ipcRenderer.invoke('session:load', filename),

  saveSession: (data: unknown): Promise<string> =>
    ipcRenderer.invoke('session:save', data),

  deleteSession: (filename: string): Promise<void> =>
    ipcRenderer.invoke('session:delete', filename),

  // ── Claude sessions (delegated to claudemon daemon) ──
  spawnClaude: (opts: { cwd?: string; profileId?: string; model?: string; skipPermissions?: boolean; resumeSessionId?: string; cols?: number; rows?: number }): Promise<string> =>
    ipcRenderer.invoke('claude:spawn', opts),
  claudeListModels: (): Promise<{ defaultModel: string; skipPermissionsDefault: boolean; aliases: Array<{ value: string; label: string }>; seen: string[] }> =>
    ipcRenderer.invoke('claude:listModels'),
  claudeMessage: (sessionId: string, text: string): Promise<{ ok: boolean; mode?: string }> =>
    ipcRenderer.invoke('claude:message', sessionId, text),
  claudeApprove: (sessionId: string, decision: 'yes' | 'no' | 'always', reason?: string): Promise<void> =>
    ipcRenderer.invoke('claude:approve', sessionId, decision, reason),
  claudeAnswer: (sessionId: string, payload: { option?: number; text?: string; answers?: string[] }): Promise<void> =>
    ipcRenderer.invoke('claude:answer', sessionId, payload),
  claudeResize: (sessionId: string, cols: number, rows: number): Promise<void> =>
    ipcRenderer.invoke('claude:resize', sessionId, cols, rows),
  claudeSignal: (sessionId: string, signal: string): Promise<void> =>
    ipcRenderer.invoke('claude:signal', sessionId, signal),
  claudeClose: (sessionId: string): Promise<void> => {
    return ipcRenderer.invoke('claude:close', sessionId).then(() => {
      const port = terminalPorts.get(sessionId);
      if (port) { port.close(); terminalPorts.delete(sessionId); }
    });
  },
  /** Subscribe a viewer pane to an existing daemon session — no spawn. */
  attachClaude: (paneId: string, sessionId: string): Promise<string> =>
    ipcRenderer.invoke('claude:attach', paneId, sessionId),
  /** Disconnect an attached viewer without affecting the underlying session. */
  detachClaude: (paneId: string): Promise<void> =>
    ipcRenderer.invoke('claude:detach', paneId).then(() => {
      const port = terminalPorts.get(paneId);
      if (port) { port.close(); terminalPorts.delete(paneId); }
    }),
  claudeGate: (sessionId: string, on: boolean): Promise<void> =>
    ipcRenderer.invoke('claude:gate', sessionId, on),

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
  claudeListSessionsForDir: (cwd: string): Promise<any[]> =>
    ipcRenderer.invoke('claude-sessions:listForDir', cwd),

  // Claude profiles
  claudeProfilesList: (): Promise<any[]> => ipcRenderer.invoke('claude-profiles:list'),
  claudeProfilesAdd: (name: string, configDir: string, extraArgs: string[]): Promise<any> =>
    ipcRenderer.invoke('claude-profiles:add', name, configDir, extraArgs),
  claudeProfilesUpdate: (id: string, updates: any): Promise<any> =>
    ipcRenderer.invoke('claude-profiles:update', id, updates),
  claudeProfilesRemove: (id: string): Promise<void> =>
    ipcRenderer.invoke('claude-profiles:remove', id),

  getClaudeSession: (sessionId: string): Promise<any> =>
    ipcRenderer.invoke('claude-session:get', sessionId),

  getAllClaudeSessions: (): Promise<any[]> =>
    ipcRenderer.invoke('claude-session:getAll'),

  onClaudeSessionUpdate: (callback: (sessionId: string, snapshot: any) => void): (() => void) => {
    const handler = (_event: Electron.IpcRendererEvent, sessionId: string, snapshot: any) => {
      callback(sessionId, snapshot);
    };
    ipcRenderer.on('claude-session:update', handler);
    return () => {
      ipcRenderer.removeListener('claude-session:update', handler);
    };
  },

  // Hub event bus — events forwarded from the hub daemon's WebSocket.
  onHubEvent: (callback: (event: { id: string; type: string; source: string; time: string; data?: unknown }) => void): (() => void) => {
    const handler = (_e: Electron.IpcRendererEvent, ev: any) => callback(ev);
    ipcRenderer.on('hub:event', handler);
    return () => ipcRenderer.removeListener('hub:event', handler);
  },
  onHubStatus: (callback: (status: { connected: boolean }) => void): (() => void) => {
    const handler = (_e: Electron.IpcRendererEvent, s: any) => callback(s);
    ipcRenderer.on('hub:status', handler);
    return () => ipcRenderer.removeListener('hub:status', handler);
  },
  listHubPlugins: (): Promise<any[]> => ipcRenderer.invoke('hub:listPlugins'),
  hubPublish: (event: { type: string; source?: string; data?: unknown }): Promise<void> =>
    ipcRenderer.invoke('hub:publish', event),
  getHubStatus: (): Promise<{ connected: boolean }> => ipcRenderer.invoke('hub:getStatus'),
  installPlugin: (url: string): Promise<{ ok: boolean; plugin?: any; error?: string }> =>
    ipcRenderer.invoke('hub:installPlugin', url),
  removePlugin: (id: string): Promise<{ ok: boolean; error?: string }> =>
    ipcRenderer.invoke('hub:removePlugin', id),

  // ── Library (reusable prompts + skills) ──
  libraryList: (cwd?: string): Promise<any[]> =>
    ipcRenderer.invoke('library:list', cwd),
  librarySave: (input: any): Promise<any> =>
    ipcRenderer.invoke('library:save', input),
  libraryRemove: (scope: 'global' | 'project' | 'claude', id: string, cwd?: string, kind?: 'prompt' | 'skill' | 'agent'): Promise<void> =>
    ipcRenderer.invoke('library:remove', scope, id, cwd, kind),
  onLibraryChanged: (callback: () => void): (() => void) => {
    const handler = () => callback();
    ipcRenderer.on('library:changed', handler);
    return () => ipcRenderer.removeListener('library:changed', handler);
  },

  // App info
  getCwd: (): Promise<string> =>
    ipcRenderer.invoke('app:getCwd'),

  // Dialog
  pickFolder: (defaultPath?: string): Promise<string | null> =>
    ipcRenderer.invoke('dialog:pickFolder', defaultPath),
  pickFiles: (defaultPath?: string): Promise<string[]> =>
    ipcRenderer.invoke('dialog:pickFiles', defaultPath),


  // Browser cookie import (Chrome or Edge)
  importChromeCookies: (domainFilter?: string[], method?: 'cdp' | 'direct', browser?: 'chrome' | 'edge'): Promise<{ imported: number; skipped: number; errors: string[] }> =>
    ipcRenderer.invoke('chrome-cookies:import', { domainFilter, method, browser }),

  // App lifecycle
  onBeforeQuit: (callback: () => void): (() => void) => {
    const handler = () => callback();
    ipcRenderer.on('app:before-quit', handler);
    return () => {
      ipcRenderer.removeListener('app:before-quit', handler);
    };
  },

  // Notifications / ambient awareness
  /** Tell main which agent session is currently on screen (null = none). */
  setActiveSession: (sessionId: string | null): void =>
    ipcRenderer.send('notify:set-active-session', sessionId),
  /** Fired when the user clicks an OS notification — carries the sessionId. */
  onFocusAgent: (callback: (sessionId: string) => void): (() => void) => {
    const handler = (_event: Electron.IpcRendererEvent, sessionId: string) => callback(sessionId);
    ipcRenderer.on('notify:focus-agent', handler);
    return () => {
      ipcRenderer.removeListener('notify:focus-agent', handler);
    };
  },
});
