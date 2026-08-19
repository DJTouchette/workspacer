/**
 * Canonical shared payload types for the high-traffic IPC surfaces.
 *
 * Main process imports these directly. The renderer imports them via a
 * relative path from src/renderer/src/… (e.g. '../../../main/shared/ipcTypes').
 *
 * IMPORTANT: do NOT import from Electron or Node-only modules here — this file
 * must be parseable by both the main tsc build and the renderer tsc build.
 */

// ── Git (review pane: git:status / diff / numstat / stage / …) ──

/** One changed file from `git status --porcelain`. `staged`/`unstaged` are the
 *  porcelain XY codes ("M" "A" "D" "R" "?" " " …). */
export interface GitFileStatus {
  path: string;
  /** Set only for renames/copies: the original path. */
  orig_path?: string;
  staged: string;
  unstaged: string;
}

export interface GitStatus {
  branch: string | null;
  files: GitFileStatus[];
  /** Upstream tracking branch ("origin/master"), null when none/gone.
   *  Optional: an older host over the hub bus may omit it. */
  upstream?: string | null;
  /** Commits ahead of / behind the upstream; both 0 when no upstream. */
  ahead?: number;
  behind?: number;
}

/** Per-file added/deleted line counts. Null counts mean a binary file. */
export interface GitNumstatEntry {
  path: string;
  added: number | null;
  deleted: number | null;
}

/** One commit from `git log`, newest first. */
export interface GitLogEntry {
  hash: string;
  subject: string;
  /** Author time, unix seconds. */
  authoredAt: number;
}

// ── Remote pairing tokens (capability-scoped hub tokens) ──

export type RemoteTokenScope = 'view' | 'triage' | 'operator';

export interface RemoteTokenRecord {
  token: string;
  scope: RemoteTokenScope;
  label?: string;
  created: string;
  /** Plugin ids whose contributed MCP-facade tools this token may use (opt-in
   *  per token; absent = none). Read by the facade only — the hub bus ignores
   *  it. TWIN: authtoken.Record.Plugins (services/hub/internal/authtoken). */
  plugins?: string[];
}

// ── Claude session snapshot (sent over claude-session:get / getAll / update) ──

export type SessionAmbientState =
  | 'idle'
  | 'thinking'
  | 'streaming'
  | 'waiting_input'
  | 'waiting_approval'
  /** Turn ended but spawned work (workflow / background subagent) still runs. */
  | 'background';

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
  /** Optional: the stream driver's optimistic user echo carries no wire
   *  timestamp; it is adopted from the transcript tailer's copy on convergence. */
  timestamp?: number;
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

/** One step of an agent's plan (Claude TodoWrite checklist, Codex plan). */
export interface PlanStep {
  content: string;
  status: 'pending' | 'in_progress' | 'completed';
  /** Present-tense "what I'm doing now" line for the in_progress step. */
  activeForm?: string;
}

/** The agent's current plan — last-write-wins full replacement. */
export interface SessionPlan {
  steps: PlanStep[];
  updatedAt: number | string;
}

export interface SubagentInfo {
  id: string;
  type: string;
  status: 'running' | 'complete';
  startedAt: number;
  completedAt?: number;
  description?: string;
  toolUseId?: string;
  model?: string;
  tokens?: number;
  costUSD?: number;
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
  costUSD?: number;
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
  totalCostUSD?: number;
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
  /** Where the agent currently works when that differs from `cwd` — e.g.
   *  inside a git worktree entered mid-session. Undefined while at home. */
  liveCwd?: string;
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
  /** Current plan/checklist (last-write-wins full replacement). */
  plan?: SessionPlan;

  ambientState: SessionAmbientState;
  lastActivity: number;
  totalToolCalls: number;
  usage: SessionUsage | null;
  /** Claude sessions only: 'stream' when the session runs on the headless
   *  stream-json managed adapter (no PTY). Absent/'pty' = classic PTY TUI. */
  transport?: 'pty' | 'stream';
  /** Federation: the peer hub this session lives on; absent = local. */
  hub?: string;
  /** Federation: true while that peer's federation link is down — the session
   *  renders as a tombstone ("hub unreachable") instead of vanishing. */
  hubOffline?: boolean;
}

// ── Federation (federation:peers) ──

/** One peer hub main has observed via hub.peer.* lifecycle events. */
export interface FederationPeerInfo {
  name: string;
  connected: boolean;
  /** Unix ms of the peer's last observed liveness. */
  lastSeen?: number;
}

// ── Hub jobs (jobs:* IPC → hub jobs.* capabilities) ──

/** When a hub job fires. Exactly one shape per kind — see hub internal/jobs. */
export interface HubJobTrigger {
  kind: 'interval' | 'daily' | 'once' | 'manual';
  everyMinutes?: number;
  /** daily: "HH:MM", hub-local time. */
  at?: string;
  /** daily: weekdays 0=Sunday…6; empty/absent = every day. */
  days?: number[];
  /** once: RFC3339 timestamp. */
  once?: string;
}

/**
 * One pre-spawn step: run a shell command (or a bus call) and feed its output
 * to the model — substituted into the prompt at `{{output}}` / `{{output.N}}`,
 * appended as a fenced block if the prompt names neither.
 *
 * The guards are the point: they let the job decide there is nothing worth
 * waking a model for, and the hub then spawns NOTHING (the run records
 * `skipped`, and unlike an error it raises no notification). See
 * services/hub/internal/jobs/jobs.go — the hub validates all of this.
 */
export interface HubJobContextStep {
  kind: 'shell' | 'call';
  shell?: { command: string; cwd?: string };
  call?: { method: string; params?: unknown };
  /** No output → don't spawn. */
  skipIfEmpty?: boolean;
  /** Spawn only when the output matches this regexp (RE2, hub-compiled). */
  skipUnlessMatch?: string;
  /** A nonzero exit is data, not failure (grep finding nothing). Timeouts
   *  still fail either way. */
  ignoreExitCode?: boolean;
}

/** What a hub job does; exactly one of the payloads matches kind. */
export interface HubJobAction {
  kind: 'spawn' | 'call' | 'shell';
  spawn?: {
    cwd: string;
    prompt: string;
    /** Runs before the agent exists; may abort the run. Hub caps the list at 4. */
    context?: HubJobContextStep[];
    provider?: string;
    model?: string;
    effort?: string;
    permissionMode?: string;
  };
  call?: { method: string; params?: unknown };
  shell?: { command: string; cwd?: string };
}

/** One persisted hub job. Timestamps are unix ms. */
export interface HubJob {
  id: string;
  name: string;
  enabled: boolean;
  trigger: HubJobTrigger;
  action: HubJobAction;
  /** Set = an AGENT proposed this job (via the MCP facade's propose_job) and no
   *  human has armed it: the hub never schedules it and refuses jobs.run until
   *  a trusted write clears this field. Approving is exactly that write — see
   *  JobsSection's approve(), or `workspacer jobs approve <id>`. */
  proposedBy?: string;
  createdAt?: number;
  updatedAt?: number;
}

/** One execution record. */
export interface HubJobRun {
  jobId: string;
  startedAt: number;
  finishedAt?: number;
  status: 'ok' | 'error' | 'skipped';
  detail?: string;
}

/** jobs.list row: the spec plus live scheduling state. */
export interface HubJobView extends HubJob {
  nextRunAt?: number;
  lastRun?: HubJobRun;
  running?: boolean;
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
// ── In-app notifications (notification center + toasts) ──

/** A notification delivered to the in-app notification center. Produced by the
 *  main-process notifiers (agent state, budget, capability calls), hub-bus
 *  `notify.post` events (plugins/remote), and renderer-internal posts via
 *  lib/notificationBus. */
export interface InAppNotification {
  id: string;
  level: 'info' | 'success' | 'warn' | 'error';
  title: string;
  body?: string;
  /** Origin label shown in the center: 'agent' | 'budget' | 'system' |
   *  'plugin:<id>' | whatever a bus caller reports about itself. */
  source: string;
  /** Click target: select this agent session… */
  sessionId?: string;
  /** …or open this pane type (plugins use their plugin pane type)… */
  paneType?: string;
  /** …or open an external URL. First present wins: sessionId → paneType → url. */
  url?: string;
  /** Stable key: a later notification with the same key replaces the earlier
   *  one instead of stacking (e.g. repeated alerts for the same condition). */
  key?: string;
  createdAt: number;
  /** Skip the transient toast — record in the center only. */
  silent?: boolean;
}

export interface AppConfig {
  ui: {
    animations: boolean;
    theme: string;
    /** User-made themes keyed by namespaced id ('custom:<slug>'). */
    customThemes?: Record<string, { name: string; base?: string; colors: Record<string, unknown> }>;
    cornerStyle: string;
    borderColor: string;
    fontFamily: string;
    fontSize: number;
    borderRadius: number;
    navBarHeight: number;
    /** Expanded sidebar width in px (user-dragged, clamped on read). */
    sidebarWidth?: number;
    paneHeaderHeight: number;
    showComposerSend?: boolean;
    guiFontScale?: number;
    diffView?: 'stacked' | 'inline' | 'split';
    mode?: 'fleet' | 'focus';
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
    /** Show transient in-app toast popups for new notifications. */
    inAppToasts: boolean;
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
    defaultPermissionMode?: string;
    defaultView: 'gui' | 'terminal';
    /** How new Claude sessions run: classic PTY TUI, or headless stream-json
     *  (managed adapter, GUI only). Default 'pty'. */
    transport?: 'pty' | 'stream';
  };
  supervisor: {
    model: string;
    summarizerModel: string;
    pollSeconds: number;
  };
  /** Superseded by `projects` — still read for backwards compatibility, never
   *  written. See renderer lib/projectRegistry. */
  directories: {
    recent: string[];
    favourites: string[];
  };
  scripts: Record<string, Array<{ name: string; command: string }>>;
  /** Per-directory project identity and state, keyed by normalized cwd exactly
   *  as `scripts` is. Declared here because this file is the main↔renderer
   *  contract and is kept in sync BY HAND — a field that lives only in
   *  configService's own `Config` is invisible to every consumer of AppConfig
   *  (the web backend included), which is the drift this type exists to prevent. */
  projects: Record<
    string,
    {
      label?: string;
      color?: string;
      icon?: string;
      favicon?: string;
      iconFile?: string;
      favourite?: boolean;
      lastOpened?: number;
      plugins?: Record<string, Record<string, unknown>>;
    }
  >;
  apps: Array<{ name: string; url: string; icon?: string }>;
  agents?: {
    defaultProvider?: string;
    defaultCwd?: string;
    binaries?: {
      claude?: string;
      codex?: string;
      opencode?: string;
      pi?: string;
    };
    /** Name new agents after their first exchange, like a chat service names a
     *  conversation. A rename you type always wins and is never overwritten. */
    autoTitle?: {
      /** Absent/true = on. */
      enabled?: boolean;
      /** Model for the one-shot title call (a cheap one; '' = claude default). */
      model?: string;
    };
  };
}

/** One resumable agent session from the daemon's full list (all providers),
 *  enriched with the desktop history DB's name/model/cost when known. */
export interface RecentAgentSession {
  sessionId: string;
  /** 'claude' | 'codex' | 'opencode' | 'pi' ('' from legacy rows ⇒ 'claude'). */
  provider: string;
  cwd: string;
  /** Daemon SessionMode on the wire ('stopped', 'input', 'responding', …). */
  mode: string;
  transport: 'pty' | 'stream';
  /** Stopped >7 days — hidden from the daemon's default list but resumable. */
  archived: boolean;
  /** Unix ms; 0 when the daemon row carried no parseable timestamp. */
  updatedAt: number;
  startedAt: number;
  /** Agent name from session_history ('' when the session predates it). */
  name: string;
  /** Provider auto-generated conversation title — claude's transcript ai-title
   *  (or first user message), codex's rollout first message. '' when unknown. */
  title: string;
  model: string;
  costUSD: number;
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
  /** Saved-session format version — see contracts/session-schema.json. Absent on
   *  files written before versioning; a value HIGHER than the reader's own means
   *  a newer build wrote it and it must not be overwritten. */
  schemaVersion?: number;
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
  /** Automatic-failover weight: 0/absent = manual only; heavier wins first
   *  when a session's account exhausts a usage window (lib/profileFailover). */
  weight?: number;
}

/** Partial update payload for claude-profiles:update. */
export type ProfileUpdate = Partial<Omit<ClaudeProfile, 'id'>>;
