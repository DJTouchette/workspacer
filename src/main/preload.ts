import { contextBridge, ipcRenderer } from 'electron';

contextBridge.exposeInMainWorld('electronAPI', {
  // Terminal
  createTerminal: (shell: string, cwd?: string, cols?: number, rows?: number): Promise<string> =>
    ipcRenderer.invoke('terminal:create', shell, cwd, cols, rows),

  writeTerminal: (id: string, data: string): Promise<void> =>
    ipcRenderer.invoke('terminal:write', id, data),

  resizeTerminal: (id: string, cols: number, rows: number): Promise<void> =>
    ipcRenderer.invoke('terminal:resize', id, cols, rows),

  closeTerminal: (id: string): Promise<void> =>
    ipcRenderer.invoke('terminal:close', id),

  onTerminalOutput: (callback: (id: string, data: string) => void): (() => void) => {
    const handler = (_event: Electron.IpcRendererEvent, id: string, data: string) => {
      callback(id, data);
    };
    ipcRenderer.on('terminal:output', handler);
    return () => {
      ipcRenderer.removeListener('terminal:output', handler);
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

  // App lifecycle
  onBeforeQuit: (callback: () => void): (() => void) => {
    const handler = () => callback();
    ipcRenderer.on('app:before-quit', handler);
    return () => {
      ipcRenderer.removeListener('app:before-quit', handler);
    };
  },
});
