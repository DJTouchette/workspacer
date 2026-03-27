export interface SessionListEntry {
  name: string;
  filename: string;
  timestamp: string;
  paneCount: number;
}

export interface ElectronAPI {
  // Terminal — control on IPC, I/O on MessagePort
  createTerminal: (shell: string, cwd?: string, cols?: number, rows?: number) => Promise<string>;
  writeTerminal: (id: string, data: string) => void;
  resizeTerminal: (id: string, cols: number, rows: number) => Promise<void>;
  closeTerminal: (id: string) => Promise<void>;
  onTerminalOutput: (id: string, callback: (data: string) => void) => () => void;
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

  // Issue Tracker
  trackerGetProviders: () => Promise<any[]>;
  trackerGetAccounts: () => Promise<any[]>;
  trackerAddAccount: (provider: string, label: string, config: Record<string, string>, token: string) => Promise<any>;
  trackerUpdateAccount: (accountId: string, updates: any) => Promise<any>;
  trackerRemoveAccount: (accountId: string) => Promise<void>;
  trackerListProjects: (accountId: string) => Promise<any[]>;
  trackerListIssues: (accountId: string, options?: any) => Promise<any[]>;
  trackerGetIssue: (accountId: string, issueKey: string) => Promise<any>;
  trackerSearchIssues: (accountId: string, query: string) => Promise<any[]>;
  trackerResolveIssueKey: (issueKey: string) => Promise<any>;
  trackerGetTransitions: (accountId: string, issueKey: string) => Promise<any[]>;
  trackerTransitionIssue: (accountId: string, issueKey: string, transitionId: string) => Promise<void>;

  // App lifecycle
  onBeforeQuit: (callback: () => void) => () => void;
}

declare global {
  interface Window {
    electronAPI: ElectronAPI;
  }
}
