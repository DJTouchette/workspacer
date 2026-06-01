export interface SessionListEntry {
  name: string;
  filename: string;
  timestamp: string;
  paneCount: number;
}

export interface ElectronAPI {
  // Terminal (non-Claude shells) — control on IPC, I/O on MessagePort
  createTerminal: (shell: string, cwd?: string, cols?: number, rows?: number) => Promise<string>;
  writeTerminal: (id: string, data: string) => void;
  resizeTerminal: (id: string, cols: number, rows: number) => Promise<void>;
  closeTerminal: (id: string) => Promise<void>;
  onTerminalOutput: (id: string, callback: (data: string) => void) => () => void;
  onTerminalExit: (callback: (id: string) => void) => () => void;

  // Claude sessions (delegated to claudemon daemon)
  spawnClaude: (opts: { cwd?: string; profileId?: string; resumeSessionId?: string; cols?: number; rows?: number }) => Promise<string>;
  claudeMessage: (sessionId: string, text: string) => Promise<{ ok: boolean; mode?: string }>;
  claudeApprove: (sessionId: string, decision: 'yes' | 'no' | 'always', reason?: string) => Promise<void>;
  claudeAnswer: (sessionId: string, payload: { option?: number; text?: string; answers?: string[] }) => Promise<void>;
  claudeResize: (sessionId: string, cols: number, rows: number) => Promise<void>;
  claudeSignal: (sessionId: string, signal: string) => Promise<void>;
  claudeClose: (sessionId: string) => Promise<void>;
  attachClaude: (paneId: string, sessionId: string) => Promise<string>;
  detachClaude: (paneId: string) => Promise<void>;
  claudeGate: (sessionId: string, on: boolean) => Promise<void>;
  claudeWrite: (sessionId: string, data: string) => void;
  onClaudeOutput: (sessionId: string, callback: (data: string) => void) => () => void;

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

  // Claude session discovery
  claudeListSessionsForDir: (cwd: string) => Promise<{ sessionId: string; timestamp: string; summary: string }[]>;

  // Claude profiles
  claudeProfilesList: () => Promise<any[]>;
  claudeProfilesAdd: (name: string, configDir: string, extraArgs: string[]) => Promise<any>;
  claudeProfilesUpdate: (id: string, updates: any) => Promise<any>;
  claudeProfilesRemove: (id: string) => Promise<void>;
  getClaudeSession: (sessionId: string) => Promise<any>;
  getAllClaudeSessions: () => Promise<any[]>;
  onClaudeSessionUpdate: (callback: (sessionId: string, snapshot: any) => void) => () => void;

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

  // DevOps (Git + CI/CD)
  devopsGetProviders: () => Promise<any[]>;
  devopsGetAccounts: () => Promise<any[]>;
  devopsAddAccount: (provider: string, label: string, config: Record<string, string>, token: string) => Promise<any>;
  devopsRemoveAccount: (accountId: string) => Promise<void>;
  devopsListRepos: (accountId: string) => Promise<any[]>;
  devopsListPRs: (accountId: string, options?: any) => Promise<any[]>;
  devopsListPipelines: (accountId: string, options?: any) => Promise<any[]>;

  // Cached queries (SQLite)
  cacheGetIssueLinks: (issueKey: string) => Promise<any[]>;
  cacheGetChildIssues: (parentKey: string) => Promise<any[]>;
  cacheSearchIssues: (query: string) => Promise<any[]>;
  cacheRecentPipelines: (limit?: number) => Promise<any[]>;
  cacheRecentPRs: (limit?: number) => Promise<any[]>;

  // Browser cookie import
  importChromeCookies: (domainFilter?: string[], method?: 'cdp' | 'direct', browser?: 'chrome' | 'edge') => Promise<{ imported: number; skipped: number; errors: string[]; diagnostics?: Record<string, any> }>;

  // App lifecycle
  onBeforeQuit: (callback: () => void) => () => void;

  // Notifications / ambient awareness
  setActiveSession: (sessionId: string | null) => void;
  onFocusAgent: (callback: (sessionId: string) => void) => () => void;
}

declare global {
  interface Window {
    electronAPI: ElectronAPI;
  }
}
