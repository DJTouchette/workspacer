import type { PluginManifest } from './plugin';
import type { LibraryItem, LibrarySaveInput, LibraryKind } from './library';
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

  // Re-color the Windows native caption buttons to match the active theme.
  setTitleBarOverlay: (color: string, symbolColor: string) => void;

  // Terminal (non-Claude shells) — control on IPC, I/O on MessagePort
  createTerminal: (shell: string, cwd?: string, cols?: number, rows?: number, profileId?: string, resumeSessionId?: string) => Promise<string>;
  writeTerminal: (id: string, data: string) => void;
  resizeTerminal: (id: string, cols: number, rows: number) => Promise<void>;
  closeTerminal: (id: string) => Promise<void>;
  onTerminalOutput: (id: string, callback: (data: string) => void) => () => void;
  onTerminalExit: (callback: (id: string) => void) => () => void;

  // Claude sessions (delegated to claudemon daemon)
  spawnClaude: (opts: { cwd?: string; profileId?: string; model?: string; skipPermissions?: boolean; resumeSessionId?: string; cols?: number; rows?: number; supervisor?: boolean; mcpItemIds?: string[] }) => Promise<string>;
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
  claudeProfilesAdd: (name: string, configDir: string, extraArgs: string[], mcpItemIds?: string[]) => Promise<ClaudeProfile>;
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
  getRemoteInfo: () => Promise<{ enabled: boolean; token: string; remoteUrl: string; appUrl: string; busUrl: string; desktopBus?: boolean }>;
  listHubPlugins: () => Promise<PluginManifest[]>;
  hubPublish: (event: { type: string; source?: string; data?: unknown }) => Promise<void>;
  installPlugin: (url: string) => Promise<{ ok: boolean; plugin?: PluginManifest; error?: string }>;
  removePlugin: (id: string) => Promise<{ ok: boolean; error?: string }>;
  setPluginEnabled: (id: string, enabled: boolean) => Promise<{ ok: boolean; plugin?: PluginManifest; error?: string }>;
  /** Mint an ephemeral, capability-scoped bus token for an agent-scoped plugin
   *  pane (confines the webview to the agent's cwd). null on failure. */
  pluginPaneToken?: (pluginId: string, agentCwd?: string) => Promise<string | null>;
  /** Revoke a pane token minted by pluginPaneToken (on pane close). */
  revokePluginPaneToken?: (token: string) => Promise<void>;
  /** Saved values for a plugin's declared settings (defaults applied by the plugin). */
  getPluginSettings?: (pluginId: string) => Promise<Record<string, unknown>>;
  /** Persist (merge) plugin settings; returns the merged values. */
  setPluginSettings?: (pluginId: string, values: Record<string, unknown>) => Promise<Record<string, unknown>>;
  /** Fired when a plugin's settings change, so open panes re-apply them live. */
  onPluginSettingsChanged?: (callback: (pluginId: string, values: Record<string, unknown>) => void) => () => void;

  // Library (reusable prompts + skills)
  libraryList: (cwd?: string) => Promise<LibraryItem[]>;
  librarySave: (input: LibrarySaveInput) => Promise<LibraryItem>;
  libraryRemove: (scope: 'global' | 'project' | 'claude', id: string, cwd?: string, kind?: LibraryKind) => Promise<void>;
  onLibraryChanged: (callback: () => void) => () => void;

  // App info
  getCwd: () => Promise<string>;
  /** The dedicated supervisor home (~/.workspacer), created on demand. */
  getSupervisorHome: () => Promise<string>;

  // Dialog
  pickFolder: (defaultPath?: string) => Promise<string | null>;
  pickFiles: (defaultPath?: string) => Promise<string[]>;
  readFile: (filePath: string) => Promise<{ path: string; contents: string; size: number }>;
  writeFile: (filePath: string, contents: string) => Promise<{ ok: boolean }>;
  readDir: (dirPath: string) => Promise<{ path: string; entries: { name: string; path: string; isDir: boolean }[] }>;

  // Watch a single file for external changes; returns an unsubscribe function.
  watchFile: (
    path: string,
    onChange: (info: { path: string; eventType: 'change' | 'rename' }) => void,
  ) => () => void;

  // Project-wide text search (ripgrep). Paths are absolute; line/column 1-based.
  searchProject: (opts: {
    query: string;
    cwd: string;
    caseSensitive?: boolean;
    wholeWord?: boolean;
    regex?: boolean;
    maxResults?: number;
  }) => Promise<{ results: { file: string; matches: { line: number; column: number; text: string }[] }[]; truncated: boolean }>;

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
