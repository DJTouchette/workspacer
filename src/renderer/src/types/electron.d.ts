export interface ElectronAPI {
  // Terminal
  createTerminal: (shell: string) => Promise<string>;
  writeTerminal: (id: string, data: string) => Promise<void>;
  resizeTerminal: (id: string, cols: number, rows: number) => Promise<void>;
  closeTerminal: (id: string) => Promise<void>;
  onTerminalOutput: (callback: (id: string, data: string) => void) => () => void;
  onTerminalExit: (callback: (id: string) => void) => () => void;

  // Config
  getConfig: () => Promise<any>;
  reloadConfig: () => Promise<any>;
  getConfigPath: () => Promise<string>;
}

declare global {
  interface Window {
    electronAPI: ElectronAPI;
  }
}
