import type { PluginManifest } from './plugin';
import type { LibraryItem, LibrarySaveInput } from './library';
import type { AnalyticsSummary, SessionHistoryRecord } from './analytics';
import type { Layout, LayoutAgent } from './layout';

export interface SessionListEntry {
  name: string;
  filename: string;
  timestamp: string;
  paneCount: number;
  agentCount?: number;
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
  spawnClaude: (opts: { cwd?: string; profileId?: string; model?: string; skipPermissions?: boolean; resumeSessionId?: string; cols?: number; rows?: number }) => Promise<string>;
  claudeListModels: () => Promise<{ defaultModel: string; skipPermissionsDefault: boolean; aliases: Array<{ value: string; label: string }>; seen: string[] }>;
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

  // Analytics (old-session metadata)
  analyticsSummary: () => Promise<AnalyticsSummary>;
  analyticsRecent: (limit?: number) => Promise<SessionHistoryRecord[]>;

  // Layout templates
  layoutsList: () => Promise<Layout[]>;
  layoutsSave: (layout: { id?: string; name: string; agents: LayoutAgent[] }) => Promise<Layout>;
  layoutsDelete: (id: string) => Promise<void>;

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
  onHubEvent: (callback: (event: { id: string; type: string; source: string; time: string; data?: unknown }) => void) => () => void;
  onHubStatus: (callback: (status: { connected: boolean }) => void) => () => void;
  getHubStatus: () => Promise<{ connected: boolean }>;
  listHubPlugins: () => Promise<PluginManifest[]>;
  hubPublish: (event: { type: string; source?: string; data?: unknown }) => Promise<void>;
  installPlugin: (url: string) => Promise<{ ok: boolean; plugin?: PluginManifest; error?: string }>;
  removePlugin: (id: string) => Promise<{ ok: boolean; error?: string }>;

  // Library (reusable prompts + skills)
  libraryList: (cwd?: string) => Promise<LibraryItem[]>;
  librarySave: (input: LibrarySaveInput) => Promise<LibraryItem>;
  libraryRemove: (scope: 'global' | 'project' | 'claude', id: string, cwd?: string, kind?: 'prompt' | 'skill' | 'agent') => Promise<void>;
  onLibraryChanged: (callback: () => void) => () => void;

  // App info
  getCwd: () => Promise<string>;

  // Dialog
  pickFolder: (defaultPath?: string) => Promise<string | null>;
  pickFiles: (defaultPath?: string) => Promise<string[]>;


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
