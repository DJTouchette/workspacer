import type { PluginManifest } from './plugin';
import type { LibraryItem, LibrarySaveInput } from './library';
import type { AnalyticsSummary, SessionHistoryRecord } from './analytics';
import type { Layout, LayoutAgent } from './layout';
import type {
  ClaudeSessionSnapshot,
  AppConfig,
  AppConfigPartial,
  SessionData,
  LayoutInput,
  ProfileUpdate,
  ClaudeProfile,
} from '../../../main/shared/ipcTypes';

export interface SessionListEntry {
  name: string;
  filename: string;
  timestamp: string;
  paneCount: number;
  agentCount?: number;
}

/** The hub-owned shared layout document. `data` is the serialized workspace
 *  layout (agents + globals); `version` increments on every accepted write so
 *  clients can ignore their own stale echoes. `data` is null before the first
 *  client seeds the document. See `src/renderer/src/types/sharedLayout.ts`. */
export interface LayoutDoc<T = unknown> {
  version: number;
  data: T | null;
}

export interface ElectronAPI {
  // Host OS — 'win32' | 'darwin' | 'linux' | …
  platform: NodeJS.Platform;

  // Terminal (non-Claude shells) — control on IPC, I/O on MessagePort
  createTerminal: (shell: string, cwd?: string, cols?: number, rows?: number, profileId?: string, resumeSessionId?: string) => Promise<string>;
  writeTerminal: (id: string, data: string) => void;
  resizeTerminal: (id: string, cols: number, rows: number) => Promise<void>;
  closeTerminal: (id: string) => Promise<void>;
  onTerminalOutput: (id: string, callback: (data: string) => void) => () => void;
  onTerminalExit: (callback: (id: string) => void) => () => void;

  // Claude sessions (delegated to claudemon daemon)
  spawnClaude: (opts: { cwd?: string; profileId?: string; model?: string; skipPermissions?: boolean; resumeSessionId?: string; cols?: number; rows?: number; supervisor?: boolean }) => Promise<string>;
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
  claudeSummarize: (sessionId: string, steps: string[]) => Promise<string | null>;
  claudeWrite: (sessionId: string, data: string) => void;
  onClaudeOutput: (sessionId: string, callback: (data: string) => void) => () => void;

  // Config
  getConfig: () => Promise<AppConfig>;
  reloadConfig: () => Promise<AppConfig>;
  getConfigPath: () => Promise<string>;
  saveConfig: (partial: AppConfigPartial) => Promise<AppConfig>;

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
  layoutsSave: (layout: LayoutInput) => Promise<Layout>;
  layoutsDelete: (id: string) => Promise<void>;

  // Claude session discovery
  claudeListSessionsForDir: (cwd: string) => Promise<{ sessionId: string; timestamp: string; summary: string }[]>;

  // Claude profiles
  claudeProfilesList: () => Promise<ClaudeProfile[]>;
  claudeProfilesAdd: (name: string, configDir: string, extraArgs: string[]) => Promise<ClaudeProfile>;
  claudeProfilesUpdate: (id: string, updates: ProfileUpdate) => Promise<ClaudeProfile>;
  claudeProfilesRemove: (id: string) => Promise<void>;
  getClaudeSession: (sessionId: string) => Promise<ClaudeSessionSnapshot | null>;
  getAllClaudeSessions: () => Promise<ClaudeSessionSnapshot[]>;
  onClaudeSessionUpdate: (callback: (sessionId: string, snapshot: ClaudeSessionSnapshot) => void) => () => void;
  onHubEvent: (callback: (event: { id: string; type: string; source: string; time: string; data?: unknown }) => void) => () => void;
  onHubStatus: (callback: (status: { connected: boolean }) => void) => () => void;
  getHubStatus: () => Promise<{ connected: boolean }>;

  // Shared layout document (hub-owned; tmux-style mirror). Reads/writes the live
  // workspace layout so the desktop and the web remote mirror each other.
  layoutGet: () => Promise<LayoutDoc>;
  layoutSet: (data: unknown) => Promise<LayoutDoc>;
  onLayoutChanged: (callback: (doc: LayoutDoc) => void) => () => void;
  getRemoteInfo: () => Promise<{ enabled: boolean; token: string; remoteUrl: string; appUrl: string; busUrl: string }>;
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
  readFile: (filePath: string) => Promise<{ path: string; contents: string; size: number }>;
  writeFile: (filePath: string, contents: string) => Promise<{ ok: boolean }>;
  readDir: (dirPath: string) => Promise<{ path: string; entries: { name: string; path: string; isDir: boolean }[] }>;

  // Host filesystem browsing (web folder picker). Optional: only the web build
  // implements it — the desktop uses native OS dialogs instead.
  fsListDir?: (path?: string) => Promise<{ path: string; parent: string; home: string; dirs: string[] }>;


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
