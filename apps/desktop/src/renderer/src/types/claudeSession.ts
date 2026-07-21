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
  contextUsedPct?: number;
  contextWindowSize?: number;
  totalInputTokens?: number;
  totalOutputTokens?: number;
  costUSD?: number;
  fiveHourPct?: number;
  fiveHourResetsAt?: number;
  sevenDayPct?: number;
  sevenDayResetsAt?: number;
  /** Monthly overage/credit window (Claude stream `overage` type only). */
  monthlyPct?: number;
  monthlyResetsAt?: number;
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
  /** Origin — a plugin's marketplace source, or a memory entry's kind. */
  source?: string;
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

  status: 'starting' | 'active' | 'ended';
  conversation: ConversationTurn[];
  /** How many turns a background compaction dropped from the FRONT of
   *  `conversation` (see compactClaudeSnapshotForBackground). Global turn
   *  index = conversationOffset + array index — consumers that key or anchor
   *  by turn index must use the global form, or every key renumbers when a
   *  pane flips between compact (hidden) and full (active) snapshots.
   *  Undefined/0 on full snapshots from the main process. */
  conversationOffset?: number;
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
  /** Live statusLine telemetry (ctx%/cost/5h/7d), fed by /statusline/stream. */
  statusLine?: SessionStatusLine;

  /** Human label set at spawn time (e.g. by a supervisor naming a worker). */
  label?: string;
  /** Session id of the agent that spawned this one — drives nesting in the UI. */
  parentSessionId?: string;
  /** Coding-agent backend ('claude' | 'codex' | 'opencode' | 'pi'). Set at spawn
   *  time; lets an adopted card render the right provider label/logo. */
  provider?: string;
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
  };
  /** Current permission mode from hook payloads — tracks live changes (e.g.
   *  shift+tab in the TUI), unlike `settings.permissionMode` which is frozen
   *  at spawn. Claude sessions only; managed providers fire no hooks. */
  livePermissionMode?: string;
  /** Context compaction from the PreCompact/PostCompact hooks: `compacting` is
   *  true mid-compaction; `lastCompactAt` (ms) + `compactionCount` badge a
   *  recently-compacted / churning session. */
  compacting?: boolean;
  lastCompactAt?: number;
  compactionCount?: number;
}
