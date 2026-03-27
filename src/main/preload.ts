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

// Receive ports sent by main process after terminal creation
ipcRenderer.on('terminal:port', (event, { id }: { id: string }) => {
  const port = event.ports[0] as unknown as IPort;
  if (!port) return;
  terminalPorts.set(id, port);
  port.start();
  // Resolve anyone waiting for this port
  const waiters = portWaiters.get(id);
  if (waiters) {
    for (const resolve of waiters) resolve(port);
    portWaiters.delete(id);
  }
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

  // Claude session
  createClaudeTerminal: (cwd?: string): Promise<string> =>
    ipcRenderer.invoke('terminal:createClaude', cwd),

  getClaudeSessionByPty: (ptyId: string): Promise<any> =>
    ipcRenderer.invoke('claude-session:getByPty', ptyId),

  getAllClaudeSessions: (): Promise<any[]> =>
    ipcRenderer.invoke('claude-session:getAll'),

  onClaudeSessionUpdate: (callback: (ptyId: string, snapshot: any) => void): (() => void) => {
    const handler = (_event: Electron.IpcRendererEvent, ptyId: string, snapshot: any) => {
      callback(ptyId, snapshot);
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
  cacheSyncNow: (): Promise<void> =>
    ipcRenderer.invoke('cache:syncNow'),
  cacheWatchRepo: (repoPath: string): Promise<void> =>
    ipcRenderer.invoke('cache:watchRepo', repoPath),

  // DevOps (Git + CI/CD)
  devopsGetProviders: (): Promise<any[]> => ipcRenderer.invoke('devops:getProviders'),
  devopsGetAccounts: (): Promise<any[]> => ipcRenderer.invoke('devops:getAccounts'),
  devopsAddAccount: (provider: string, label: string, config: Record<string, string>, token: string): Promise<any> =>
    ipcRenderer.invoke('devops:addAccount', provider, label, config, token),
  devopsRemoveAccount: (accountId: string): Promise<void> => ipcRenderer.invoke('devops:removeAccount', accountId),
  devopsListRepos: (accountId: string): Promise<any[]> => ipcRenderer.invoke('devops:listRepos', accountId),
  devopsListPRs: (accountId: string, options?: any): Promise<any[]> => ipcRenderer.invoke('devops:listPRs', accountId, options),
  devopsListPipelines: (accountId: string, options?: any): Promise<any[]> => ipcRenderer.invoke('devops:listPipelines', accountId, options),

  // App lifecycle
  onBeforeQuit: (callback: () => void): (() => void) => {
    const handler = () => callback();
    ipcRenderer.on('app:before-quit', handler);
    return () => {
      ipcRenderer.removeListener('app:before-quit', handler);
    };
  },
});
