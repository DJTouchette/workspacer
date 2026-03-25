import { contextBridge, ipcRenderer } from 'electron';

contextBridge.exposeInMainWorld('electronAPI', {
  // Terminal
  createTerminal: (shell: string): Promise<string> =>
    ipcRenderer.invoke('terminal:create', shell),

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
});
