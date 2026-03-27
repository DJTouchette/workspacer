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

  // Claude session
  createClaudeTerminal: (cwd?: string) => Promise<string>;
  getClaudeSessionByPty: (ptyId: string) => Promise<any>;
  getAllClaudeSessions: () => Promise<any[]>;
  onClaudeSessionUpdate: (callback: (ptyId: string, snapshot: any) => void) => () => void;

  // App info
  getCwd: () => Promise<string>;

  // Dialog
  pickFolder: (defaultPath?: string) => Promise<string | null>;
  pickFiles: (defaultPath?: string) => Promise<string[]>;

  // App lifecycle
  onBeforeQuit: (callback: () => void) => () => void;
}

declare global {
  interface Window {
    electronAPI: ElectronAPI;
  }
}
