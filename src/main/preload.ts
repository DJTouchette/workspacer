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
  spawnClaude: (opts: { cwd?: string; profileId?: string; resumeSessionId?: string; cols?: number; rows?: number }): Promise<string> =>
    ipcRenderer.invoke('claude:spawn', opts),
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

  // App info
  getCwd: (): Promise<string> =>
    ipcRenderer.invoke('app:getCwd'),

  // Dialog
  pickFolder: (defaultPath?: string): Promise<string | null> =>
    ipcRenderer.invoke('dialog:pickFolder', defaultPath),
  pickFiles: (defaultPath?: string): Promise<string[]> =>
    ipcRenderer.invoke('dialog:pickFiles', defaultPath),

  // Issue Tracker
  trackerGetProviders: (): Promise<any[]> =>
    ipcRenderer.invoke('tracker:getProviders'),
  trackerGetAccounts: (): Promise<any[]> =>
    ipcRenderer.invoke('tracker:getAccounts'),
  trackerAddAccount: (provider: string, label: string, config: Record<string, string>, token: string): Promise<any> =>
    ipcRenderer.invoke('tracker:addAccount', provider, label, config, token),
  trackerUpdateAccount: (accountId: string, updates: any): Promise<any> =>
    ipcRenderer.invoke('tracker:updateAccount', accountId, updates),
  trackerRemoveAccount: (accountId: string): Promise<void> =>
    ipcRenderer.invoke('tracker:removeAccount', accountId),
  trackerListProjects: (accountId: string): Promise<any[]> =>
    ipcRenderer.invoke('tracker:listProjects', accountId),
  trackerListIssues: (accountId: string, options?: any): Promise<any[]> =>
    ipcRenderer.invoke('tracker:listIssues', accountId, options),
  trackerGetIssue: (accountId: string, issueKey: string): Promise<any> =>
    ipcRenderer.invoke('tracker:getIssue', accountId, issueKey),
  trackerSearchIssues: (accountId: string, query: string): Promise<any[]> =>
    ipcRenderer.invoke('tracker:searchIssues', accountId, query),
  trackerResolveIssueKey: (issueKey: string): Promise<any> =>
    ipcRenderer.invoke('tracker:resolveIssueKey', issueKey),
  trackerGetTransitions: (accountId: string, issueKey: string): Promise<any[]> =>
    ipcRenderer.invoke('tracker:getTransitions', accountId, issueKey),
  trackerTransitionIssue: (accountId: string, issueKey: string, transitionId: string): Promise<void> =>
    ipcRenderer.invoke('tracker:transitionIssue', accountId, issueKey, transitionId),

  // Cached queries (SQLite)
  cacheGetIssueLinks: (issueKey: string): Promise<any[]> =>
    ipcRenderer.invoke('cache:getIssueLinks', issueKey),
  cacheGetChildIssues: (parentKey: string): Promise<any[]> =>
    ipcRenderer.invoke('cache:getChildIssues', parentKey),
  cacheSearchIssues: (query: string): Promise<any[]> =>
    ipcRenderer.invoke('cache:searchIssues', query),
  cacheRecentPipelines: (limit?: number): Promise<any[]> =>
    ipcRenderer.invoke('cache:recentPipelines', limit),
  cacheRecentPRs: (limit?: number): Promise<any[]> =>
    ipcRenderer.invoke('cache:recentPRs', limit),

  // DevOps (Git + CI/CD)
  devopsGetProviders: (): Promise<any[]> => ipcRenderer.invoke('devops:getProviders'),
  devopsGetAccounts: (): Promise<any[]> => ipcRenderer.invoke('devops:getAccounts'),
  devopsAddAccount: (provider: string, label: string, config: Record<string, string>, token: string): Promise<any> =>
    ipcRenderer.invoke('devops:addAccount', provider, label, config, token),
  devopsRemoveAccount: (accountId: string): Promise<void> => ipcRenderer.invoke('devops:removeAccount', accountId),
  devopsListRepos: (accountId: string): Promise<any[]> => ipcRenderer.invoke('devops:listRepos', accountId),
  devopsListPRs: (accountId: string, options?: any): Promise<any[]> => ipcRenderer.invoke('devops:listPRs', accountId, options),
  devopsListPipelines: (accountId: string, options?: any): Promise<any[]> => ipcRenderer.invoke('devops:listPipelines', accountId, options),

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
});
