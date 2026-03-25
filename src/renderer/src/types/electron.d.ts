export interface SessionListEntry {
  name: string;
  filename: string;
  timestamp: string;
  paneCount: number;
}

export interface ElectronAPI {
  // Terminal
  createTerminal: (shell: string, cwd?: string) => Promise<string>;
  writeTerminal: (id: string, data: string) => Promise<void>;
  resizeTerminal: (id: string, cols: number, rows: number) => Promise<void>;
  closeTerminal: (id: string) => Promise<void>;
  onTerminalOutput: (callback: (id: string, data: string) => void) => () => void;
  onTerminalExit: (callback: (id: string) => void) => () => void;

  // Config
  getConfig: () => Promise<any>;
  reloadConfig: () => Promise<any>;
  getConfigPath: () => Promise<string>;
  saveConfig: (partial: any) => Promise<any>;

  // Session
  listSessions: () => Promise<SessionListEntry[]>;
  loadSession: (filename: string) => Promise<any>;
  saveSession: (data: any) => Promise<string>;
  deleteSession: (filename: string) => Promise<void>;

  // App lifecycle
  onBeforeQuit: (callback: () => void) => () => void;
}

declare global {
  interface Window {
    electronAPI: ElectronAPI;
  }
}
