/**
 * Canonical shared payload types for the high-traffic IPC surfaces.
 *
 * Main process imports these directly. The renderer imports them via a
 * relative path from src/renderer/src/… (e.g. '../../../main/shared/ipcTypes').
 *
 * IMPORTANT: do NOT import from Electron or Node-only modules here — this file
 * must be parseable by both the main tsc build and the renderer tsc build.
 */

// ── Claude session snapshot (sent over claude-session:get / getAll / update) ──

export type SessionAmbientState =
  | 'idle'
  | 'thinking'
  | 'streaming'
  | 'waiting_input'
  | 'waiting_approval';

export interface ToolCall {
  id: string;
  name: string;
  input: unknown;
  response?: unknown;
  status: 'running' | 'complete' | 'failed';
  startedAt: number;
  completedAt?: number;
}

export interface ConversationTurn {
  role: 'user' | 'assistant';
  content: string;
  timestamp: number;
  toolCalls?: ToolCall[];
}

export interface FileChange {
  path: string;
  toolName: string;
  input: unknown;
  timestamp: number;
}

export interface PendingApproval {
  toolName: string;
  toolInput: unknown;
  suggestions?: string[];
  timestamp: number;
}

export interface PendingQuestionOption {
  label: string;
  description?: string;
}

export interface PendingQuestion {
  question: string;
  header?: string;
  multi_select?: boolean;
  options: PendingQuestionOption[];
}

export interface SubagentInfo {
  id: string;
  type: string;
  status: 'running' | 'complete';
  startedAt: number;
  completedAt?: number;
  description?: string;
  model?: string;
  tokens?: number;
  toolCalls?: number;
  lastToolName?: string;
  lastToolSummary?: string;
}

export interface WorkflowPhaseInfo {
  title: string;
  detail?: string;
}

export interface WorkflowAgentInfo {
  id: string;
  label?: string;
  phaseTitle?: string;
  model?: string;
  status: 'queued' | 'running' | 'done' | 'failed';
  startedAt?: number;
  completedAt?: number;
  durationMs?: number;
  tokens: number;
  toolCalls: number;
  lastToolName?: string;
  lastToolSummary?: string;
  promptPreview?: string;
  resultPreview?: string;
}

export interface WorkflowRunInfo {
  runId: string;
  name?: string;
  description?: string;
  status: 'running' | 'completed' | 'failed';
  startedAt: number;
  completedAt?: number;
  durationMs?: number;
  phases: WorkflowPhaseInfo[];
  agents: WorkflowAgentInfo[];
  totalTokens?: number;
  totalToolCalls?: number;
}

export interface SessionUsage {
  model: string | null;
  /** Latest turn's input side (point-in-time). */
  contextTokens: number;
  /** Tokens the model's window holds. */
  contextLimit: number;
  /** Cumulative (incl. cache). */
  totalInputTokens: number;
  /** Cumulative. */
  totalOutputTokens: number;
  /** Cumulative USD. */
  costUSD: number;
}

/** Serialisable snapshot sent over IPC (claude-session:get / getAll / update). */
export interface ClaudeSessionSnapshot {
  sessionId: string;
  cwd: string;
  ptyId: string;

  status: 'starting' | 'active' | 'ended';
  conversation: ConversationTurn[];
  activeToolCalls: ToolCall[];
  completedToolCalls: ToolCall[];
  fileChanges: FileChange[];
  pendingApproval: PendingApproval | null;
  pendingQuestions: PendingQuestion[] | null;
  subagents: SubagentInfo[];
  workflows: WorkflowRunInfo[];

  ambientState: SessionAmbientState;
  lastActivity: number;
  totalToolCalls: number;
  usage: SessionUsage | null;
}

// ── App configuration (config:get / config:save) ──

/**
 * Partial config accepted by config:save. Only the keys the caller wants to
 * update need be present; they are deep-merged server-side.
 */
export type AppConfigPartial = Record<string, unknown>;

/**
 * Full application config returned by config:get / config:reload.
 * Mirrors the private Config interface in configService.ts — kept in sync
 * manually; the runtime shape is the authority.
 */
export interface AppConfig {
  ui: {
    animations: boolean;
    theme: string;
    cornerStyle: string;
    borderColor: string;
    fontFamily: string;
    fontSize: number;
    borderRadius: number;
    navBarHeight: number;
    paneHeaderHeight: number;
    showComposerSend?: boolean;
    guiFontScale?: number;
    diffView?: 'stacked' | 'inline' | 'split';
  };
  terminal: {
    shell: string;
    shells: Array<{ name: string; path: string; label: string }>;
    fontFamily: string;
    fontSize: number;
    scrollback: number;
    cursorBlink: boolean;
    cursorStyle: string;
  };
  browser: {
    homepage: string;
    bookmarks: Array<{ name: string; url: string }>;
    hibernateAfter: number;
  };
  panes: {
    defaultWidth: number;
    gap: number;
    peek: number;
    insertPosition: string;
    tabPosition: string;
    viewMode: string;
    viewLevel?: string;
    default: Array<{ id: string; type: string; title: string; width: number; order: number }>;
  };
  keybindings: {
    prefix: string;
    chordHints?: boolean;
    shortcuts: Record<string, string>;
  };
  notifications: {
    enabled: boolean;
    notifyDone: boolean;
    onlyWhenUnwatched: boolean;
    sound: boolean;
  };
  editor: {
    engine: 'codemirror' | 'terminal';
    terminalCommand: string;
    vim?: boolean;
  };
  claude: {
    defaultModel: string;
    seenModels: string[];
    skipPermissionsDefault: boolean;
    defaultView: 'gui' | 'terminal';
  };
  supervisor: {
    model: string;
    summarizerModel: string;
    pollSeconds: number;
  };
  directories: {
    recent: string[];
    favourites: string[];
  };
  scripts: Record<string, Array<{ name: string; command: string }>>;
  apps: Array<{ name: string; url: string; icon?: string }>;
  session: {
    autoResume: boolean;
  };
}

// ── Session persistence (session:save) ──

export interface SessionPaneData {
  id: string;
  type: string;
  title: string;
  width: number;
  widthOverride?: number;
  shell?: string;
  cwd?: string;
  url?: string;
}

export interface SessionTabData {
  id: string;
  title: string;
  panes: SessionPaneData[];
  activePaneId: string;
  canvas?: { x: number; y: number; w: number; h: number };
  lastActiveAt?: number;
}

export interface SessionAgentData {
  id: string;
  name: string;
  global?: boolean;
  cwd: string;
  profileId?: string;
  model?: string;
  skipPermissions?: boolean;
  /** Library item ids (kind 'mcp') this agent was spawned with — re-passed on respawn. */
  mcpItemIds?: string[];
  sessionId?: string;
  tabs: SessionTabData[];
  activeTabId: string;
}

/** Payload for session:save. timestamp is filled in by the IPC handler. */
export interface SessionData {
  name: string;
  timestamp?: string;
  activeAgentId?: string;
  agents?: SessionAgentData[];
  /** Legacy flat layout — kept for backward compat. */
  activeTabId?: string;
  tabs?: SessionTabData[];
  activePaneId?: string;
  panes?: SessionPaneData[];
  /** PTY-id → claudemon-session-id mapping for CWD enrichment (renderer only). */
  ptyMapping?: Record<string, string>;
}

// ── Layout templates (layouts:save) ──

export interface LayoutPane {
  type: string;
  title: string;
  url?: string;
  shell?: string;
  cwd?: string;
}

export interface LayoutTab {
  title: string;
  panes: LayoutPane[];
}

export interface LayoutAgent {
  name: string;
  cwd: string;
  model?: string;
  tabs: LayoutTab[];
}

/** Full persisted layout (returned by layouts:list / layouts:save). */
export interface Layout {
  id: string;
  name: string;
  createdAt: string;
  agents: LayoutAgent[];
}

/** Payload for layouts:save. */
export type LayoutInput = { id?: string; name: string; agents: LayoutAgent[] };

// ── Claude profiles (claude-profiles:update) ──

export interface ClaudeProfile {
  id: string;
  name: string;
  configDir: string;
  extraArgs: string[];
  /** Library item ids (kind 'mcp') loaded by default when spawning with this profile. */
  mcpItemIds?: string[];
  isDefault: boolean;
}

/** Partial update payload for claude-profiles:update. */
export type ProfileUpdate = Partial<Omit<ClaudeProfile, 'id'>>;
