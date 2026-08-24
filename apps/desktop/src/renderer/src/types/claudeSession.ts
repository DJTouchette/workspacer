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
  input: any;
  response?: any;
  status: 'running' | 'complete' | 'failed';
  startedAt: number;
  completedAt?: number;
}

export interface ConversationTurn {
  role: 'user' | 'assistant';
  content: string;
  timestamp: number;
  toolCalls?: ToolCall[];
  /** Set when this turn is a slash-command run — rendered as a command card
   *  (CommandCard) instead of a user bubble. */
  command?: { name: string; args?: string; output?: string; outputIsError?: boolean };
}

export interface FileChange {
  path: string;
  toolName: string;
  input: any;
  timestamp: number;
}

export interface PendingApproval {
  toolName: string;
  toolInput: any;
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
  // Live enrichment from the subagent's transcript (main-process workflowWatcher)
  description?: string;
  /** The Agent tool_use id that spawned this subagent — exact anchor for the timeline. */
  toolUseId?: string;
  model?: string;
  tokens?: number;
  /** Estimated USD cost, live-accumulated from the subagent's usage blocks. */
  costUSD?: number;
  toolCalls?: number;
  lastToolName?: string;
  lastToolSummary?: string;
}

// ── Workflow runs (mirrors src/main/services/workflowWatcher.ts) ──

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
  /** Estimated USD cost, live-accumulated from the agent's usage blocks. */
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
  /** Estimated USD cost — sum of the agents' live-accumulated costs. */
  totalCostUSD?: number;
}

/** Token / cost / context usage, parsed from the transcript in the main process. */
export interface SessionUsage {
  model: string | null;
  contextTokens: number; // latest turn's input side (point-in-time)
  contextLimit: number; // tokens the model's window holds
  totalInputTokens: number; // cumulative (incl. cache)
  totalOutputTokens: number; // cumulative
  costUSD: number; // cumulative
  /** Per-model split (main thread + subagent turns), keyed by model id. */
  models?: Record<string, { inputTokens: number; outputTokens: number; costUSD: number }>;
}

/**
 * Live statusLine telemetry, fed by claudemon's `/statusline/stream`. This is
 * the only source of Claude's authoritative context-%, cost, and the 5h/7d
 * rate-limit windows (none appear in the transcript-derived `SessionUsage`).
 * All fields optional — Claude omits some (rate_limits is Pro/Max-only).
 */
export interface SessionStatusLine {
  modelDisplay?: string;
  /** Provider-confirmed reasoning-effort level (Codex). Absent for Claude,
   *  which reports its effective effort in no channel — the composer falls back
   *  to the level it asked for (`liveEffort`) there. */
  effort?: string;
  contextUsedPct?: number;
  contextWindowSize?: number;
  totalInputTokens?: number;
  totalOutputTokens?: number;
  costUSD?: number;
  fiveHourPct?: number;
  fiveHourResetsAt?: number;
  /** How long the 5h window is, in minutes (300 for Claude; Codex reports its
   *  own figure). Absent when the provider does not say. */
  fiveHourWindowMins?: number;
  sevenDayPct?: number;
  sevenDayResetsAt?: number;
  /** How long the weekly window is, in minutes (10080 for Claude). */
  sevenDayWindowMins?: number;
  /** Monthly overage/credit window (Claude stream `overage` type only). */
  monthlyPct?: number;
  monthlyResetsAt?: number;
  /** How long the monthly window is, in minutes. Never set for Claude: a
   *  calendar month has no fixed length and no source reports one. */
  monthlyWindowMins?: number;
  /** Human warning when a window crosses its threshold (stream only). */
  rateLimitWarning?: string;
  /** Monthly overage disabled for lack of credits (stream only). */
  overageOutOfCredits?: boolean;
  /** Session capabilities from the stream init frame (stream only). */
  capabilities?: SessionCapabilities;
  receivedAt?: string;
}

/** Capabilities parsed from Claude's stream `init` frame (stream sessions). */
export interface SessionCapabilities {
  fastMode?: boolean;
  outputStyle?: string;
  apiKeySource?: string;
  mcpServers?: number;
  skills?: number;
  plugins?: number;
  agents?: number;
  memoryFiles?: number;
  /** Itemized inventory behind the counts (names, paths, size estimates). */
  inventory?: ContextInventoryInfo;
}

/** One named thing loaded into the session's context (stream sessions).
 *  `bytes`/`estTokens` are estimates from the backing file on disk (~4 chars
 *  per token) — absent when the item has no file we can find. */
export interface ContextItemInfo {
  name: string;
  path?: string;
  /** MCP server connection status ("connected" / "pending" / "failed"). */
  status?: string;
  /** Origin — a plugin's marketplace source, or a memory entry's kind. For a
   *  skill or agent, which root it resolved in: 'project', 'user', a plugin
   *  name, or 'built-in' for the ones compiled into the CLI (which have no file
   *  and so never carry a path or size). */
  source?: string;
  /** One-line `description:` from the item's frontmatter. Absent for built-ins. */
  description?: string;
  bytes?: number;
  estTokens?: number;
}

/** Itemized context inventory from the stream `init` frame. */
export interface ContextInventoryInfo {
  mcpServers: ContextItemInfo[];
  skills: ContextItemInfo[];
  agents: ContextItemInfo[];
  plugins: ContextItemInfo[];
  memoryFiles: ContextItemInfo[];
  tools: string[];
  slashCommands: string[];
  claudeCodeVersion?: string;
}

export interface ClaudeSessionSnapshot {
  sessionId: string;
  cwd: string;
  /** Where the agent currently works when that differs from `cwd` — e.g.
   *  inside a git worktree entered mid-session. Undefined while at home. */
  liveCwd?: string;
  ptyId: string;
  /** Absolute path of Claude's transcript JSONL. Reveals which Claude config
   *  root (login/account) serves the session — see lib/claudeAccount.ts.
   *  Blanked ('') for remote (federated) sessions: it names the peer's fs. */
  transcriptPath?: string;

  status: 'starting' | 'active' | 'ended';
  conversation: ConversationTurn[];
  /** How many turns a background compaction dropped from the FRONT of
   *  `conversation` (see compactClaudeSnapshotForBackground). Global turn
   *  index = conversationOffset + array index — consumers that key or anchor
   *  by turn index must use the global form, or every key renumbers when a
   *  pane flips between compact (hidden) and full (active) snapshots.
   *  Nonzero on a full snapshot too, once the main-process cap has trimmed a
   *  very long session (sessionStore/bounds) — it means "turns dropped from the
   *  front", not "this is a background snapshot". */
  conversationOffset?: number;
  /** How many of the turns counted by `conversationOffset` were genuine user
   *  sends. Banked by both trimmers because it cannot be derived from the turn
   *  offset (which counts every role), and ClaudePane needs an absolute
   *  user-send tally to retire its optimistic bubbles — a window-relative count
   *  goes BACKWARDS when the head is trimmed, which reads as a thread reset. */
  conversationUserOffset?: number;
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
  /** Live background tasks (async subagents, `run_in_background` shells,
   *  workflows). Ambient work never claims the "Working" mode — a background
   *  dev server latching an idle agent as Working was the "agents say working
   *  when they're not" bug — so this count is the honest badge instead. */
  backgroundTasks?: number;
  lastActivity: number;
  totalToolCalls: number;
  usage: SessionUsage | null;
  /** Live statusLine telemetry (ctx%/cost/5h/7d), fed by /statusline/stream. */
  statusLine?: SessionStatusLine;

  /** Human label set at spawn time (e.g. by a supervisor naming a worker). */
  label?: string;
  /** Session id of the agent that spawned this one — drives nesting in the UI. */
  parentSessionId?: string;
  /** This session's own orphan truth, computed server-side (see
   *  claudeSessionStore's `refreshOrphanStatus`) from live rows + dead-manager
   *  tombstones — always fresh as of this snapshot, never a value the renderer
   *  derived itself. Present only when `parentSessionId` names a session
   *  confirmed gone; `confirmedManager` is true when a tombstone proved the
   *  dead parent really was a manager, false for a merely dangling parent id.
   *  Undefined while the parent is alive, unset, or this is a federated
   *  remote session. */
  orphan?: { confirmedManager: boolean };
  /** Coding-agent backend ('claude' | 'codex' | 'opencode' | 'pi'). Set at spawn
   *  time; lets an adopted card render the right provider label/logo. */
  provider?: string;
  /** Federation: the peer hub this session lives on. Absent = local session.
   *  Remote sessions arrive through the same snapshot/update flow; the
   *  renderer just tags their cards and withholds local cwd-bound surfaces. */
  hub?: string;
  /** Federation tombstone: the peer hub's link is down. The card stays (last
   *  known snapshot) but shows "hub offline — last seen …" instead of a live
   *  ambient state. Only meaningful alongside `hub`. */
  hubOffline?: boolean;
  /** Claude sessions only: 'stream' when the session runs on the headless
   *  stream-json managed adapter — no PTY, so the pane is GUI-only and answers
   *  go through POST /answer instead of keystrokes. Absent/'pty' = classic
   *  PTY TUI transport. */
  transport?: 'pty' | 'stream';
  /** Requested-at-spawn launch settings — the composer pills' fallback truth
   *  (live statusLine/usage model wins when present). */
  settings?: {
    model?: string;
    effort?: string;
    permissionMode?: string;
    /** Claude only: whether the process carries `--dangerously-skip-permissions`.
     *  Claude gates switching *to* `bypassPermissions` on that launch flag, so
     *  this decides whether the composer's "Full access" pick can go live or has
     *  to restart. Absent = we don't know how this row was launched. */
    bypassAvailable?: boolean;
    /** Claude only: what an absent `--effort` resolves to (settings chain, read
     *  at spawn). Lets the effort pill show the level rather than "Default".
     *  Codex's equivalent comes from its live model catalog instead. */
    defaultEffort?: string;
  };
  /** Current permission mode from hook payloads — tracks live changes (e.g.
   *  shift+tab in the TUI), unlike `settings.permissionMode` which is frozen
   *  at spawn. Claude sessions only; managed providers fire no hooks. */
  livePermissionMode?: string;
  /** Reasoning-effort level this session was last *asked* to run at, set when a
   *  live switch is accepted. Optimistic by necessity for Claude, which confirms
   *  an effort change nowhere; Codex's own confirmation arrives separately on the
   *  status line and wins over this. Unlike `settings.effort` it is not frozen at
   *  spawn. */
  liveEffort?: string;
  /** Context compaction from the PreCompact/PostCompact hooks: `compacting` is
   *  true mid-compaction; `lastCompactAt` (ms) + `compactionCount` badge a
   *  recently-compacted / churning session. */
  compacting?: boolean;
  lastCompactAt?: number;
  compactionCount?: number;
}
