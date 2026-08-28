import * as path from 'path';
import { BrowserWindow } from 'electron';
import { agentNotifier } from './agentNotifier';
import { supervisorNudge } from './supervisorNudge';
import { checkBudget } from './budgetWatcher';
import {
  workflowWatcher,
  type WorkflowRunInfo,
  type WorkflowWatcherUpdate,
} from './workflowWatcher';
import {
  publishWorkflowRuns,
  publishSnapshot,
  forgetSession as forgetTelemetry,
} from './hubTelemetry';
import {
  applyHookEvent,
  applyStopEvent,
  applySessionEndEvent,
  normalizeBackgroundAmbient,
  sessionHasBackgroundWork,
} from './sessionStore/hookEventRouter';
import {
  applyConversationItems,
  type ConversationDeltaWire,
  type ConversationItemWire,
} from './sessionStore/conversationApplier';
import { SessionUsageAccumulator } from './sessionStore/usageAccumulator';
import {
  acknowledgeAnswer,
  bornWithEmptyPending,
  bornWithPending,
  detachPendingSlot,
  PendingSlot,
} from './sessionStore/pendingSlot';
import type { PendingFencedSession, SessionWithoutPending } from './sessionStore/pendingSlot';
import { CLAUDEMON_API_URL } from './claudemonDaemon';
import { writeHistory } from './sessionStore/analyticsWriter';
import { revokeSessionFacadeTokens } from './remoteTokens';

export type { WorkflowRunInfo, WorkflowAgentInfo, WorkflowPhaseInfo } from './workflowWatcher';

// ── Performance flags ──

/**
 * When true, rapid successive pushUpdate calls for the same session are
 * coalesced: the session is marked dirty and a single flush is scheduled via a
 * ~16 ms timer. When false, every call sends immediately (original behaviour).
 * Flip to false here to revert without a code change.
 */
const COALESCE_SNAPSHOT_UPDATES = true;

/**
 * Trailing-edge debounce window (ms) applied to statusLine ticks before they
 * trigger a full snapshot push. statusLine fires many times/sec; 250 ms keeps
 * the renderer at ~4 updates/sec for these informational fields.
 * Set to 0 to disable (immediate push, original behaviour).
 */
const STATUSLINE_DEBOUNCE_MS = 250;

/**
 * Ambient session activity, mostly driven by hook events now that claudemon
 * owns the hook ingestion. Kept compatible with the renderer's view-side type
 * (`src/renderer/src/types/claudeSession.ts`).
 */
export type SessionAmbientState =
  | 'idle'
  | 'thinking'
  | 'streaming'
  | 'waiting_input'
  | 'waiting_approval'
  // The agent's own turn ended but work it spawned is still running (a
  // Workflow run or an async background subagent) — never shown as 'idle'
  // so the fleet doesn't read "done" mid-workflow. Derived, not hook-driven:
  // see normalizeBackgroundAmbient in hookEventRouter.
  | 'background';

/** Launch settings requested at spawn/restart time (composer pill truth
 *  fallback — live statusLine/usage wins for the model when present). */
export interface SessionSpawnSettings {
  model?: string;
  effort?: string;
  permissionMode?: string;
  /** Claude only: whether the process was launched with
   *  `--dangerously-skip-permissions`. Claude gates *switching to*
   *  `bypassPermissions` on that flag — the stream transport's control protocol
   *  refuses it outright and the PTY TUI leaves the mode out of its shift+tab
   *  cycle — so this is what says whether "Full access" can be applied live or
   *  only by restarting. Absent on rows whose launch we didn't record. */
  bypassAvailable?: boolean;
  /** Claude only: the level an *absent* `--effort` resolves to, read from the
   *  settings chain at spawn (see claudeEffortDefault.ts). Claude reports its
   *  effective effort in no telemetry channel, so this is the only way the pill
   *  can name it. Undefined when nothing pins a level. */
  defaultEffort?: string;
}

/** Normalize a path for consistent map-key matching on Windows (backslash vs forward slash, case) */
function normalizeCwd(cwd: string): string {
  if (!cwd) return cwd;
  let normalized = path.resolve(cwd);
  if (process.platform === 'win32') {
    normalized = normalized.toLowerCase();
  }
  return normalized;
}

// ── Types ──

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
  /** Optional: the stream driver's optimistic user echo carries no wire
   *  timestamp; it is adopted from the transcript tailer's copy on convergence. */
  timestamp?: number;
  toolCalls?: ToolCall[];
  /** Set when this turn is a slash-command run (claudemon `slash_command`
   *  item), rendered as a command card instead of a user bubble. `output`
   *  arrives separately (`command_output`) and is folded in by the applier. */
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

/**
 * claudemon's `SessionState.pending` slot as serialized on `session.update`
 * frames (snake_case-tagged `Pending` enum in services/claudemon
 * session/state.rs). For managed providers (codex/opencode/pi) this is the
 * only approval/question payload — they fire no Claude hooks.
 */
export type ManagedPendingWire =
  | { kind: 'approval'; tool?: string | null; summary?: string | null; raw?: any }
  | { kind: 'question'; questions?: PendingQuestion[]; raw?: any };

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
  // Live enrichment from the subagent's transcript (workflowWatcher)
  description?: string;
  /** The Agent tool_use id that spawned this subagent — exact anchor for the timeline. */
  toolUseId?: string;
  model?: string;
  tokens?: number;
  costUSD?: number;
  toolCalls?: number;
  lastToolName?: string;
  lastToolSummary?: string;
}

/**
 * Live statusLine telemetry, fed by claudemon's `/statusline/stream` (the only
 * channel carrying Claude's context-%, authoritative cost, and 5h/7d rate-limit
 * windows). Mirrors `SessionStatusLine` in the renderer types. All fields
 * optional — Claude omits some (e.g. rate_limits only for Pro/Max accounts).
 */
export interface SessionStatusLine {
  modelDisplay?: string;
  /** Provider-confirmed reasoning-effort level; absent for Claude. */
  effort?: string;
  contextUsedPct?: number;
  contextWindowSize?: number;
  totalInputTokens?: number;
  totalOutputTokens?: number;
  /** Cache-read subset of `totalInputTokens`, when the provider reports one.
   *  Codex does; Claude's statusLine carries no cache figures at all, and there
   *  the itemized transcript split (`SessionUsage.cache`) is the source. */
  cachedInputTokens?: number;
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
  /** Monthly overage/credit window (Claude stream `overage` type). Absent for
   *  the interactive statusLine and providers without a monthly window. */
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

/** Derive an absolute context-token count from statusLine's percentage +
 *  window size — the only context signal managed providers (codex/opencode/
 *  pi) ever emit, since they never produce a transcript `usage` item and so
 *  never run through usageAccumulator.applyUsage. Returns undefined when
 *  either piece is missing, so callers can distinguish "no data" from "zero
 *  used" instead of silently reporting 0. */
export function contextTokensFromStatusLine(sl?: SessionStatusLine): number | undefined {
  if (sl?.contextUsedPct === undefined || sl?.contextWindowSize === undefined) return undefined;
  // DERIVED, not counted — so it inherits every error in either input, and it
  // is the only token figure a managed (non-Claude) session has. A percentage
  // is bounded by definition but nothing upstream enforces that: claudemon
  // reads `used_percentage` straight off the provider's payload
  // (session/state.rs) and does not clamp it, and a provider that reports a
  // running total rather than a percentage would multiply the window by it.
  // Clamp to the window, so the worst case is a meter pegged at 100% instead
  // of a session claiming to hold forty times what it can.
  const pct = Math.min(100, Math.max(0, sl.contextUsedPct));
  return Math.round((pct / 100) * sl.contextWindowSize);
}

function normalizeManagedSubagent(raw: unknown): SubagentInfo | null {
  if (!raw || typeof raw !== 'object') return null;
  const r = raw as Record<string, unknown>;
  const id = typeof r.id === 'string' ? r.id : '';
  if (!id) return null;
  const status = r.status === 'running' ? 'running' : 'complete';
  const startedRaw = r.startedAt ?? r.started_at;
  const completedRaw = r.completedAt ?? r.completed_at;
  const num = (v: unknown): number | undefined =>
    typeof v === 'number' && Number.isFinite(v) ? v : undefined;
  const str = (v: unknown): string | undefined => (typeof v === 'string' ? v : undefined);
  const sub: SubagentInfo = {
    id,
    type: str(r.type) ?? str(r.agentType) ?? str(r.agent_type) ?? 'codex',
    status,
    startedAt: num(startedRaw) ?? Date.now(),
  };
  const completedAt = num(completedRaw);
  if (completedAt !== undefined) sub.completedAt = completedAt;
  const description = str(r.description);
  if (description) sub.description = description;
  const toolUseId = str(r.toolUseId) ?? str(r.tool_use_id);
  if (toolUseId) sub.toolUseId = toolUseId;
  const model = str(r.model);
  if (model) sub.model = model;
  const tokens = num(r.tokens);
  if (tokens !== undefined) sub.tokens = tokens;
  const costUSD = num(r.costUSD) ?? num(r.cost_usd);
  if (costUSD !== undefined) sub.costUSD = costUSD;
  const toolCalls = num(r.toolCalls) ?? num(r.tool_calls);
  if (toolCalls !== undefined) sub.toolCalls = toolCalls;
  const lastToolName = str(r.lastToolName) ?? str(r.last_tool_name);
  if (lastToolName) sub.lastToolName = lastToolName;
  const lastToolSummary = str(r.lastToolSummary) ?? str(r.last_tool_summary);
  if (lastToolSummary) sub.lastToolSummary = lastToolSummary;
  return sub;
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

export interface ClaudeSessionState {
  sessionId: string;
  cwd: string;
  /** Where the agent currently works when that differs from `cwd` — e.g.
   *  inside a git worktree entered mid-session. Undefined while at home. */
  liveCwd?: string;
  ptyId: string; // workspacer PTY id this session is bound to
  transcriptPath: string; // path to JSONL transcript file

  status: 'starting' | 'active' | 'ended';
  conversation: ConversationTurn[];
  activeToolCalls: ToolCall[];
  completedToolCalls: ToolCall[];
  fileChanges: FileChange[];
  pendingApproval: PendingApproval | null;
  pendingQuestions: PendingQuestion[] | null;
  subagents: SubagentInfo[];
  workflows: WorkflowRunInfo[];
  /** Current plan/checklist, replaced wholesale on each plan update (may be
   *  re-sent on resync). Undefined until the agent first writes a plan. */
  plan?: SessionPlan;

  ambientState: SessionAmbientState;
  /** Live background tasks (async subagents, `run_in_background` shells) from
   *  the daemon. Ambient work never claims the Working mode — this is the
   *  honest badge (see applyManagedMode / claudemonEventBridge). */
  backgroundTasks?: number;
  startedAt: number; // ms, when the session was first seen (for analytics duration)
  lastActivity: number;
  totalToolCalls: number;
  peakContext: number; // highest context-token reading seen (for analytics)
  usage: import('./modelUsage').SessionUsage | null; // token / cost / context, from the daemon's usage items
  statusLine?: SessionStatusLine; // live statusLine telemetry (ctx%/cost/rate-limits)
  // Adoption metadata — set before first hook arrives so adopted cards can be
  // named and nested under the agent that spawned them.
  label?: string;
  parentSessionId?: string;
  /** Wake eligibility, NOT a role: true when this session may receive fleet
   *  wakes — it gets nudged when another agent blocks on a decision or a
   *  worker finishes (see supervisorNudge). Its only input is `opts.manager`
   *  at spawn. */
  isWakeTarget?: boolean;
  /** Coding-agent backend ('claude' | 'codex' | 'opencode'), for analytics. */
  provider?: string;
  /** Claude sessions only: 'stream' when the session runs on the headless
   *  stream-json managed adapter (no PTY; GUI-only pane, hooks are
   *  enrichment-only). Absent/'pty' = the classic PTY TUI transport. */
  transport?: 'pty' | 'stream';
  /** Requested-at-spawn launch settings — what the composer pills show when no
   *  live telemetry (statusLine/usage model) is available yet. */
  settings?: SessionSpawnSettings;
  /** Current permission mode from hook payloads (`permission_mode` rides on
   *  PreToolUse/PostToolUse/UserPromptSubmit/Stop). Unlike `settings`, this
   *  tracks live changes — e.g. shift+tab cycling in the TUI. Claude sessions
   *  only; managed providers fire no hooks so it stays unset for them. */
  livePermissionMode?: string;
  /** Reasoning-effort level this session was last *asked* to run at, set when a
   *  live switch is accepted. Optimistic by necessity for Claude, which confirms
   *  an effort change nowhere; Codex's own confirmation arrives separately on the
   *  status line and wins over this. Unlike `settings.effort` it is not frozen at
   *  spawn. */
  liveEffort?: string;
  /** Guards against double history writes (Stop 1500ms timeout vs SessionEnd). */
  historyWritten?: boolean;
  /** True once the parent's own turn ended (Stop fired) while a background
   *  (async Agent/Task) subagent is still running — the pane holds 'streaming'
   *  instead of flipping idle mid-subagent, and the last SubagentStop rides the
   *  real idle in. PTY/hook sessions only (stream sessions own ambientState via
   *  the daemon's managed mode). Mirrors claudemon's `parent_turn_ended`. */
  parentTurnEnded?: boolean;
  /** Context compaction, from the PreCompact/PostCompact hooks: `compacting` is
   *  true while Claude is rewriting context; `lastCompactAt` (ms) and
   *  `compactionCount` badge a recently-compacted / churning session. */
  compacting?: boolean;
  lastCompactAt?: number;
  compactionCount?: number;
  /** Structured-result contract this session was dispatched with: the JSON
   *  Schema its `spawn_agent` carried as `resultSchema`. Held so the
   *  worker-finished wake can validate the block the worker emits against the
   *  shape its dispatcher asked for (shared/structuredResult). Undefined for
   *  every ordinary dispatch — a schema is opt-in and purely additive. */
  resultSchema?: Record<string, unknown>;
  /** Federation: the peer hub this session lives on; absent = local. Remote
   *  sessions are fed by hub-stamped agent.snapshot events (federationBridge),
   *  never by local hooks/deltas, and skip every local side effect (history,
   *  eviction timers, facade tokens, notifier, supervisor nudges). */
  hub?: string;
  /** Federation: true while that peer's link is down (tombstone state). */
  hubOffline?: boolean;
  /**
   * This session's own orphan truth, recomputed fresh on every snapshot leaving
   * the store (see `refreshOrphanStatus`) — never written anywhere else, so it
   * can't drift from `managerTombstones`/live rows the way a value cached at
   * adopt time can. Present only when `parentSessionId` names a session
   * confirmed gone; `confirmedManager` mirrors `OrphanCandidate.confirmedManager`
   * for that same parent (true = a tombstone proved it was a manager, false =
   * a bare dangling parent id). Undefined while the parent is alive, unset, or
   * this is a federated remote session (its parent is the PEER's fact).
   */
  orphan?: { confirmedManager: boolean };
}

// Serialisable snapshot sent over IPC
export type ClaudeSessionSnapshot = Omit<ClaudeSessionState, never>;

/**
 * The federated remote-session wire shape: a peer desktop's compacted
 * ClaudeSessionSnapshot (compactClaudeSnapshotForBackground output), arriving
 * either as a hub-stamped `agent.snapshot` event or a `hub:<peer>/
 * sessions.snapshots` result row. Same field names as our own snapshot —
 * both ends run the same publisher — plus the compaction bookkeeping fields
 * and the `sparse` marker on layout-ghost stopped rows.
 */
export type RemoteSnapshotWire = Partial<ClaudeSessionSnapshot> & {
  sessionId?: string;
  /** Peer-synthesized stopped row from its layout document — not a live session. */
  sparse?: boolean;
  conversationOffset?: number;
  conversationUserOffset?: number;
};

/**
 * What survives a MANAGER's eviction, so its successor can be told who it is
 * replacing instead of guessing.
 *
 * Deliberately NOT a field on ClaudeSessionSnapshot: a tombstone is not a
 * session. It renders nowhere, it is not pushed to the renderer, and above all
 * it must never be reachable as a wake destination (see orphanCandidates) —
 * three properties a resurrected row would quietly lose.
 *
 * Only the four questions succession actually asks: was this a manager, when
 * did it die, what was it called, where did it work.
 */
export interface DeadManagerTombstone {
  sessionId: string;
  label?: string;
  cwd: string;
  /** ms, when the row was evicted (SessionEnd + grace, or close_session). */
  endedAt: number;
  provider?: string;
}

/**
 * A dead parent that still has live children — the `fromSessionId`
 * `agents.reparent` wants, plus what a human or an agent needs to decide
 * WHICH one was theirs.
 *
 * `confirmedManager` is the whole point of the tombstone: false means the id
 * is merely dangling (derived from the children, as before — it could be a
 * dead worker that spawned subagents), true means the store watched a session
 * marked `isWakeTarget` die.
 */
export interface OrphanCandidate {
  sessionId: string;
  label: string | null;
  cwd: string | null;
  endedAt: number | null;
  confirmedManager: boolean;
  children: Array<{
    sessionId: string;
    label: string | null;
    cwd: string;
    /** 'pending' = dispatched, not yet registered (spawnMeta only). */
    state: SessionAmbientState | 'pending';
  }>;
}

/** Hard cap on retained tombstones (see pruneManagerTombstones). */
const MAX_MANAGER_TOMBSTONES = 32;

// ── Store ──

class ClaudeSessionStore {
  // The fenced view, not `ClaudeSessionState`: every `this.sessions.get()` in
  // this class therefore hands back a session whose pending slot is READONLY,
  // so no method here can park or resolve a card without going through a
  // `PendingSlot` that has declared which feed it is. `readonly` is invisible
  // to assignability, so the rows still pass unchanged to the notifier, the
  // watcher, `applyHookEvent` and `publishSnapshot`. (Those collaborators only
  // read the slot — see the writer census in ./sessionStore/pendingSlot.)
  private sessions = new Map<string, PendingFencedSession>();
  private mainWindow: BrowserWindow | null = null;
  // Latest workflow/subagent filesystem state per session, re-merged whenever
  // either the watcher ticks or a hook event mutates the subagent list.
  private watcherUpdates = new Map<string, WorkflowWatcherUpdate>();
  // Accumulator owns lastUsageKey + knownModels dedup state.
  private usageAccumulator = new SessionUsageAccumulator();
  // Pre-spawn metadata keyed by pinned session id. Recorded before the first
  // hook arrives so adopted cards carry a name and parent from the start.
  private spawnMeta = new Map<
    string,
    {
      label?: string;
      parentSessionId?: string;
      isWakeTarget?: boolean;
      provider?: string;
      transport?: 'pty' | 'stream';
      settings?: SessionSpawnSettings;
      resultSchema?: Record<string, unknown>;
    }
  >();
  // Last-applied conversation sequence per session (gap detection for the
  // daemon's delta stream) and sessions with a snapshot resync in flight.
  private convSeq = new Map<string, number>();
  // Federation: last folded remote-conversation sequence per REMOTE session
  // (fed by federationBridge's sessions.conversation fetches). Presence also
  // marks the session's `conversation` as item-owned — window pushes stop
  // overwriting it (see upsertRemoteSession). Cleared when the session drops.
  private remoteConvSeq = new Map<string, number>();
  private resyncing = new Set<string>();
  // Coalescing: sessions with a pending flush scheduled (COALESCE_SNAPSHOT_UPDATES).
  private pendingFlush = new Map<string, NodeJS.Timeout>();
  // Debounce: per-session statusLine debounce timers (STATUSLINE_DEBOUNCE_MS).
  private statusLineTimers = new Map<string, NodeJS.Timeout>();
  // Grace period: per-session SessionEnd eviction timers. Held so a restart can
  // cancel one — a restart reuses the session id (`resumeSessionId` pins it), so
  // an uncancellable timer scheduled by the dying life fires 30 s into the
  // successor's life and deletes a session that is running. See cancelEviction.
  private evictionTimers = new Map<string, NodeJS.Timeout>();
  // Debounce: per-managed-session analytics snapshot timers. Managed (codex /
  // opencode) sessions don't fire Claude Stop/SessionEnd hooks, so we snapshot
  // their history off the conversation stream instead (see scheduleManagedHistory).
  private managedHistoryTimers = new Map<string, NodeJS.Timeout>();
  // Succession: what a MANAGER leaves behind when its row is evicted, keyed by
  // its (now dead) session id. Written only by evictNow — the one teardown
  // path — and read only by orphanCandidates. Retention is "still has live
  // children", so this is bounded by the fleet, not by uptime; see
  // pruneManagerTombstones for the backstop cap.
  private managerTombstones = new Map<string, DeadManagerTombstone>();

  /** Record name/parent for a session about to be spawned, keyed by its pinned
   *  id. Consumed when the session first registers (see createSession), so an
   *  adopted card can be named and nested under its spawner. */
  setSpawnMeta(
    sessionId: string,
    meta: {
      label?: string;
      parentSessionId?: string;
      isWakeTarget?: boolean;
      provider?: string;
      transport?: 'pty' | 'stream';
      settings?: SessionSpawnSettings;
      /** Structured-result schema (spawn_agent's resultSchema) — see
       *  ClaudeSessionState.resultSchema. */
      resultSchema?: Record<string, unknown>;
    },
  ): void {
    if (!sessionId) return;
    // A spawn onto this id supersedes whatever life scheduled an eviction for
    // it; this is the earliest point in a restart, before any hook has landed.
    this.cancelEviction(sessionId);
    this.spawnMeta.set(sessionId, meta);
    // A restart-with-settings re-spawns onto an id that may still have a live
    // entry — refresh its settings in place so the pills track the request.
    const existing = this.sessions.get(sessionId);
    if (existing && meta.settings) {
      existing.settings = { ...existing.settings, ...meta.settings };
      // A restart can change the window (`opus` → `opus[1m]`), and the new
      // life's first usage turn is a while away — re-resolve now.
      SessionUsageAccumulator.refreshContextLimit(existing);
      // The new life's telemetry starts empty. `livePermissionMode` holds the
      // *previous* life's hook value and the composer pill prefers it over
      // `settings`, so leaving it behind makes a restart-to-change-the-mode look
      // like it did nothing until the next hook happens to land.
      if (meta.settings.permissionMode) existing.livePermissionMode = undefined;
      this.pushUpdate(existing);
    }
  }

  /** Record a model a *live* switch (`claude:setModel`) asked this session to
   *  move to. Optimistic by necessity — the provider confirms asynchronously on
   *  the status line — but leaving `settings.model` pinned to the spawn value
   *  is strictly worse: it would keep claiming a 1M window after a switch down
   *  to a 200k model (and vice-versa) until telemetry catches up. No-op for
   *  unknown ids. */
  noteRequestedModel(sessionId: string, model: string): void {
    if (!sessionId || !model) return;
    const session = this.sessions.get(sessionId);
    if (!session) return;
    session.settings = { ...session.settings, model };
    SessionUsageAccumulator.refreshContextLimit(session);
    this.pushUpdate(session);
  }

  /** Eagerly register a freshly-spawned managed (codex/opencode/pi) session so
   *  its GUI pane has a snapshot to render right away. Managed backends fire no
   *  Claude hooks and emit no conversation delta until they first produce output,
   *  so without this the pane sits on the empty "connecting / no session" state
   *  until the first message. Idempotent — a no-op once the session exists (the
   *  conversation/statusline streams take over from there). Picks up the
   *  provider/label from any prior setSpawnMeta via createSession. */
  ensureManagedSession(sessionId: string, cwd: string): void {
    if (!sessionId || this.sessions.has(sessionId)) return;
    const session = this.createSession(sessionId, normalizeCwd(cwd));
    this.pushUpdate(session);
  }

  /** Session ids currently marked as supervisors (live sessions only). */
  supervisorSessionIds(): string[] {
    const ids: string[] = [];
    for (const s of this.sessions.values()) if (s.isWakeTarget) ids.push(s.sessionId);
    return ids;
  }

  /**
   * Hand every dispatch a retiring manager still has in flight to its
   * SUCCESSOR, so the wakes keep arriving.
   *
   * Fleet wakes are PARENT-KEYED: a worker-finished wake routes only to the
   * worker's own live `isWakeTarget` parent (nudgeParentOnFinish below), and the
   * dropped-wake backstop keys off the same field (`sweepMissedFinishes`
   * matches `c.parentSessionId === manager.sessionId`). So replacing a Fleet
   * Manager used to ORPHAN every worker it had dispatched: the successor could
   * never receive their reports, and the only workaround was a manual ritual
   * where the outgoing manager begged each worker to leave results on disk.
   * Re-pointing `parentSessionId` is the whole fix — and because the backstop
   * re-reads the field on every sweep, it needs no separate re-pointing.
   *
   * `parentSessionId` IS the routing key and nothing else. This changes routing
   * deliberately and completely: after this call the old manager hears nothing
   * further about these workers, by design. It records no lineage — a second
   * meaning on this field would fire "Worker finished" wakes at the wrong
   * session.
   *
   * Also moved: dispatches still in `spawnMeta` (spawned, no hook yet — the
   * most orphan-prone case of all, a worker whose whole life is ahead of it),
   * and any finished-wake already queued inside supervisorNudge's coalesce
   * window for the old parent, which would otherwise be delivered to a manager
   * on its way out.
   *
   * Deliberately ALLOWED while the old manager is still alive: that is the
   * normal handoff — an outgoing manager is by definition mid-turn when it
   * hands over, so refusing on a live predecessor would refuse the only case
   * this exists for. Refused, loudly, when the successor cannot actually
   * receive wakes (unknown, ended, or not a manager), because quietly
   * re-pointing workers at a session that no wake can reach is strictly worse
   * than the orphaning it was meant to fix.
   *
   * Returns the ids actually moved: `moved` for live sessions, `pending` for
   * not-yet-registered dispatches.
   */
  reparentChildren(
    oldManagerId: string,
    newManagerId: string,
  ): { moved: string[]; pending: string[] } {
    if (!oldManagerId || !newManagerId) {
      throw new Error('reparent_children: both the outgoing and the new manager id are required');
    }
    if (oldManagerId === newManagerId) {
      throw new Error(`reparent_children: ${newManagerId} is already the parent — nothing to move`);
    }
    // Live rows and pending spawns only — NEVER managerTombstones. A tombstone
    // proves a manager is DEAD, which is the one thing that disqualifies it as
    // a destination: nudgeParentOnFinish refuses an ended parent, so adopting
    // ONTO one would silence the very workers this is rescuing. The tombstones
    // answer `fromSessionId` and nothing else.
    const successor = this.sessions.get(newManagerId);
    const successorMeta = this.spawnMeta.get(newManagerId);
    if (!successor && !successorMeta) {
      throw new Error(
        `reparent_children: no session ${newManagerId} — a wake can only be routed to a session ` +
          `this process knows about`,
      );
    }
    if (successor?.status === 'ended') {
      throw new Error(
        `reparent_children: ${newManagerId} has ended — moving workers to a dead parent would ` +
          `orphan them exactly as leaving them was`,
      );
    }
    // A parent that isn't a manager is a black hole for wakes: nudgeParentOnFinish
    // requires `parent.isWakeTarget`, so this would silence every dispatch
    // instead of rerouting it — and silently, which is the worst shape.
    if (!(successor?.isWakeTarget ?? successorMeta?.isWakeTarget)) {
      throw new Error(
        `reparent_children: ${newManagerId} is not a manager (isWakeTarget) — worker-finished ` +
          `wakes are only ever delivered to a supervisor parent`,
      );
    }

    const moved: string[] = [];
    for (const session of this.sessions.values()) {
      if (session.parentSessionId !== oldManagerId) continue;
      // The successor may itself have been dispatched BY the outgoing manager
      // (the usual way a manager is replaced). Never let it become its own
      // parent — nudgeParentOnFinish would then wake it about itself.
      if (session.sessionId === newManagerId) continue;
      // Federated sessions are fed by their own hub's snapshots; their parent
      // is that peer's fact and the next snapshot would overwrite us anyway.
      if (session.hub) continue;
      session.parentSessionId = newManagerId;
      moved.push(session.sessionId);
      this.pushUpdate(session);
    }

    const pending: string[] = [];
    for (const [sessionId, meta] of this.spawnMeta) {
      if (meta.parentSessionId !== oldManagerId || sessionId === newManagerId) continue;
      meta.parentSessionId = newManagerId;
      pending.push(sessionId);
    }

    // A wake for these workers may already be sitting in the coalesce window
    // addressed to the old manager. Re-address it rather than let it land on a
    // manager that is being retired.
    supervisorNudge.reassignPendingFinish(oldManagerId, newManagerId);

    // The per-worker "nothing new to report" signature (supervisorNudge's
    // lastReportedReply) is deliberately NOT cleared. A worker that already
    // reported before the handoff has an unchanged reply, and re-delivering
    // that identical payload to the successor is precisely the duplicate wake
    // PER_TURN_WAKE_FINDING.md 1b calls noise; its result belongs in the
    // handoff brief. Anything the worker does AFTER the move produces a
    // different reply and wakes the successor normally.

    // The predecessor's tombstone (if it had one) has just lost its children,
    // so it stops being an orphan candidate immediately rather than at the next
    // read — "already adopted" and "still waiting" must not look alike.
    this.pruneManagerTombstones();
    return { moved, pending };
  }

  /**
   * The dead parents that still have live children — the successor's half of
   * `agents.reparent` when there was no handoff file to read an id off.
   *
   * A manager that CRASHES writes no handoff, and ~30 s after its SessionEnd
   * its row is evicted, so all that was left of it was a dangling
   * `parentSessionId` on the workers. That is enough to find a GROUP and
   * nothing else: it does not say the parent was a manager (a worker can spawn
   * agents too), and with two dangling groups it does not say which was yours.
   * The tombstone recorded at eviction answers the first question outright and
   * gives the second the only evidence there is — a label, a cwd and a time of
   * death to match against what the successor was told to take over.
   *
   * It REPORTS; it never picks. Automatic adoption was rejected when
   * `agents.reparent` landed because a wrong guess silently re-points a live
   * worker's wakes into a conversation that never dispatched it, and the
   * tombstone weakens that objection without removing it: `confirmedManager`
   * narrows the candidates to real managers, but two managers can die with
   * live children, and only the successor knows which brief it was handed.
   * So this returns every candidate, ranked, and the choice stays a deliberate
   * call with an id in it.
   *
   * Candidates come from two sources on purpose:
   *   - a tombstone (`confirmedManager: true`) — the store watched an
   *     `isWakeTarget` session die;
   *   - a bare dangling id (`confirmedManager: false`) — children point at a
   *     parent this process has no row and no tombstone for. Unprovable, but
   *     hiding it would lose the orphans that predate the tombstone (a manager
   *     evicted before this build, or one whose whole life the app missed).
   *
   * A parent whose row is still resident but `ended` (inside the eviction
   * grace) is a candidate too — `nudgeParentOnFinish` already refuses to wake
   * an ended parent, so those workers are orphaned in every sense that matters,
   * 30 s before the tombstone exists.
   *
   * Federated children are excluded: a remote row's `parentSessionId` names a
   * session on the PEER, and `reparentChildren` refuses to move it anyway.
   */
  orphanCandidates(): OrphanCandidate[] {
    this.pruneManagerTombstones();

    const byParent = new Map<string, OrphanCandidate['children']>();
    const addChild = (parentId: string, child: OrphanCandidate['children'][number]): void => {
      const kids = byParent.get(parentId);
      if (kids) kids.push(child);
      else byParent.set(parentId, [child]);
    };
    for (const s of this.sessions.values()) {
      if (!s.parentSessionId || s.hub || s.status === 'ended') continue;
      addChild(s.parentSessionId, {
        sessionId: s.sessionId,
        label: s.label ?? null,
        cwd: s.cwd,
        state: s.ambientState,
      });
    }
    // Dispatched but not yet registered — the most orphan-prone case there is,
    // and the one reparentChildren also moves (its `pending` half).
    for (const [sessionId, meta] of this.spawnMeta) {
      if (!meta.parentSessionId) continue;
      addChild(meta.parentSessionId, {
        sessionId,
        label: meta.label ?? null,
        cwd: '',
        state: 'pending',
      });
    }

    const candidates: OrphanCandidate[] = [];
    for (const [parentId, children] of byParent) {
      const row = this.sessions.get(parentId);
      if (row && row.status !== 'ended') continue; // still alive: nothing is orphaned
      const tomb = this.managerTombstones.get(parentId);
      candidates.push({
        sessionId: parentId,
        label: tomb?.label ?? row?.label ?? null,
        cwd: tomb?.cwd ?? row?.cwd ?? null,
        endedAt: tomb?.endedAt ?? null,
        confirmedManager: Boolean(tomb) || row?.isWakeTarget === true,
        children,
      });
    }
    // Proven managers first, then the most recent death, then by id so the
    // order is stable for a caller comparing two reads.
    return candidates.sort(
      (a, b) =>
        Number(b.confirmedManager) - Number(a.confirmedManager) ||
        (b.endedAt ?? 0) - (a.endedAt ?? 0) ||
        a.sessionId.localeCompare(b.sessionId),
    );
  }

  /**
   * Retention rule: a tombstone lives exactly as long as something it parented
   * is still around to be adopted. Nothing else is a bound — by age it would
   * expire while the workers it explains are still running, and unbounded it is
   * a map that grows once per manager for the life of the process.
   *
   * The cap is a backstop against pathology (a fleet that retires managers
   * faster than their workers finish), and it drops the OLDEST: a successor
   * spawning now is replacing one of the recent deaths.
   *
   * Note this deliberately re-reads the LIVE rows every time rather than
   * keeping a child count on the tombstone — a count would go stale the moment
   * a worker was adopted, and a tombstone that outlives its children starts
   * re-answering "who was my predecessor" with a manager that has nothing left
   * to hand over.
   */
  /**
   * How many manager tombstones are retained. Diagnostic, and the ONLY way the
   * retention rule can be observed at all: a childless tombstone reports
   * nothing through orphanCandidates (candidates are derived from the children,
   * so a parent with none is simply absent), which means a map that never
   * pruned would look identical from the outside while growing once per
   * manager for the life of the process. A guard has to be able to see it.
   */
  managerTombstoneCount(): number {
    this.pruneManagerTombstones();
    return this.managerTombstones.size;
  }

  private pruneManagerTombstones(): void {
    if (this.managerTombstones.size === 0) return;
    const parented = new Set<string>();
    for (const s of this.sessions.values()) {
      if (s.parentSessionId && !s.hub && s.status !== 'ended') parented.add(s.parentSessionId);
    }
    for (const meta of this.spawnMeta.values()) {
      if (meta.parentSessionId) parented.add(meta.parentSessionId);
    }
    for (const id of [...this.managerTombstones.keys()]) {
      if (!parented.has(id)) this.managerTombstones.delete(id);
    }
    if (this.managerTombstones.size <= MAX_MANAGER_TOMBSTONES) return;
    const oldestFirst = [...this.managerTombstones.values()].sort((a, b) => a.endedAt - b.endedAt);
    for (const t of oldestFirst.slice(0, this.managerTombstones.size - MAX_MANAGER_TOMBSTONES)) {
      this.managerTombstones.delete(t.sessionId);
    }
  }

  /**
   * Worker-finished wake (FLEET_MANAGER_SPIKE.md gap #2): when a session with
   * a supervisor/manager PARENT transitions working→idle, nudge that parent —
   * the dispatch came home. Called at every ambient-transition site right
   * after notifyOnTransition, which uses the same working→idle edge for the
   * user's own "finished" notification; blocks stay on onBlock's broadcast
   * path. The parent must be LIVE and marked isWakeTarget (managers set the
   * same flag) — a worker whose parent ended just goes quiet.
   */
  private nudgeParentOnFinish(session: ClaudeSessionState, prevAmbient: SessionAmbientState): void {
    const wasWorking =
      prevAmbient === 'thinking' || prevAmbient === 'streaming' || prevAmbient === 'background';
    if (!wasWorking || session.ambientState !== 'idle') return;
    const parentId = session.parentSessionId;
    if (!parentId) return;
    const parent = this.sessions.get(parentId);
    if (!parent?.isWakeTarget || parent.status === 'ended') return;
    const lastReply =
      [...session.conversation].reverse().find((t) => t.role === 'assistant')?.content ?? '';
    supervisorNudge.onFinished(session, parentId, lastReply);
  }

  setMainWindow(win: BrowserWindow): void {
    this.mainWindow = win;
    this.startWakeBackstop();
  }

  private wakeBackstop?: NodeJS.Timeout;
  /**
   * Periodic safety net for a dropped worker-finished wake: onFinished is
   * best-effort, so a manager can go dark. Every WAKE_BACKSTOP_MS, hand the
   * live session set to supervisorNudge, which re-nudges any idle manager whose
   * child finished without it acting. Idempotent; unref'd so it never holds the
   * process open.
   */
  private startWakeBackstop(): void {
    if (this.wakeBackstop) return;
    const WAKE_BACKSTOP_MS = 2 * 60_000;
    this.wakeBackstop = setInterval(() => {
      try {
        supervisorNudge.sweepMissedFinishes(Array.from(this.sessions.values()), Date.now());
      } catch {
        /* the sweep is a best-effort backstop */
      }
    }, WAKE_BACKSTOP_MS);
    this.wakeBackstop.unref?.();
  }

  // ── Hook event handler ──
  //
  // Events come in with claudemon's *canonical* session_id (post-aliasing —
  // the spawn UUID, not whatever id Claude Code internally generates). We
  // create or update the session entry under that id; nothing else binds.

  handleHookEvent(event: any): void {
    const hookName: string = event.hook_event_name ?? event.type ?? '';
    const sessionId: string = event.session_id ?? '';
    const cwd: string = normalizeCwd(event.cwd ?? '');

    if (!sessionId) return;

    // Any event that isn't the end of a life means this id is in use again, so
    // an eviction scheduled by a previous life must not survive to fire. A
    // restart reuses the id and lands well inside the 30 s grace period.
    if (hookName !== 'SessionEnd') this.cancelEviction(sessionId);

    let session = this.sessions.get(sessionId);
    if (!session) {
      session = this.createSession(sessionId, cwd);
    } else if (!session.cwd && cwd) {
      // Sessions first seen via a conversation delta are created with cwd ''
      // (the delta wire carries no cwd) — backfill it from the first hook that
      // has it, so project attribution, git-branch lookup and the agent-card
      // name work instead of sticking to the empty string forever.
      session.cwd = cwd;
    }

    // Live cwd: hooks carry the session's *current* working directory on
    // every event, and it moves when the agent enters/exits a git worktree
    // (EnterWorktree tool). `cwd` stays the spawn directory (spawn/restart
    // paths depend on it); `liveCwd` follows the agent so file opens, diffs
    // and git lookups resolve against where the work actually happens.
    // Cleared (undefined) when the agent returns home.
    if (cwd && session.cwd) {
      session.liveCwd = cwd === session.cwd ? undefined : cwd;
    }

    // Capture transcript path from first event that has it
    if (event.transcript_path && !session.transcriptPath) {
      session.transcriptPath = event.transcript_path;
      console.log(`[SessionStore] transcript: ${session.transcriptPath}`);
      // Start watching for workflow runs + subagent transcripts beside it
      workflowWatcher.attach(sessionId, session.transcriptPath, (update) => {
        this.applyWatcherUpdate(sessionId, update);
      });
    }
    // Keep the watcher's poll loop alive while hooks are flowing
    workflowWatcher.poke(sessionId);

    // Track the live permission mode — most hook payloads carry it, and it's
    // the only signal that follows shift+tab cycling in the TUI (the statusLine
    // JSON doesn't include it, and `settings` is frozen at spawn time).
    if (typeof event.permission_mode === 'string' && event.permission_mode) {
      session.livePermissionMode = event.permission_mode;
    }

    // Conversation content arrives via claudemon's transcript tailer
    // (applyConversationDelta) — no JSONL reads happen in this process.

    // A new turn re-arms the analytics snapshot. historyWritten was previously
    // set by the first Stop and never cleared, so only that first Stop ever
    // wrote history — every later turn's Stop bailed and the 'active' row kept
    // turn-1 usage until (if ever) a clean SessionEnd arrived. Skip re-arming
    // after SessionEnd so a stray late prompt can't let a pending Stop timer
    // overwrite the final 'ended' row with an 'active' one.
    if (hookName === 'UserPromptSubmit' && session.status !== 'ended') {
      session.historyWritten = false;
    }

    // Snapshot the ambient state so the notifier can detect transitions
    // (needs-you / done) after the switch below applies the new state.
    const prevAmbient = session.ambientState;

    // Handle Stop and SessionEnd here because they need store-level side-effects.
    if (hookName === 'Stop') {
      applyStopEvent(session);
      // Delayed history write: the final assistant message may still be in
      // flight on the conversation stream (claudemon keeps tailing briefly
      // after Stop), so give it a moment to land before snapshotting.
      // Guard with historyWritten so a SessionEnd that races doesn't double-write.
      setTimeout(() => {
        if (session!.historyWritten) return;
        session!.historyWritten = true;
        writeHistory(session!, 'active');
      }, 1500);
    } else if (hookName === 'SessionEnd') {
      applySessionEndEvent(session);
      workflowWatcher.detach(sessionId);
      forgetTelemetry(sessionId);
      supervisorNudge.forgetWorker(sessionId);
      // Always finalize to 'ended'. A Stop event earlier in this turn may have
      // already fired its delayed 'active' snapshot (setting historyWritten),
      // but that row is non-terminal — the analytics record upsert is keyed by
      // session_id, so writing 'ended' here overwrites it with the real
      // ended_at. Setting historyWritten first also stops any still-pending
      // Stop timer from reverting the row back to 'active' after us.
      session.historyWritten = true;
      writeHistory(session, 'ended');
      // Flush any coalesced update synchronously so the final state is sent
      // before the session is forgotten by the renderer.
      this.flushPending(sessionId);
      // Evict the session entry after a grace period so the maps don't grow
      // unboundedly. Every per-session Map/Set must be cleared, not just
      // `sessions` — convSeq and watcherUpdates otherwise retain one entry per
      // ended session for the whole process lifetime. Clearing the stale
      // convSeq also lets a resumed (reused-id) session start fresh instead of
      // forcing a spurious resync on its first delta (a reused id would inherit
      // the prior life's seq and read the first delta as a gap).
      this.cancelEviction(sessionId);
      const evict = setTimeout(() => {
        this.evictionTimers.delete(sessionId);
        // Belt and braces alongside cancelEviction: if anything revived this id
        // without going through a hook, the row is no longer 'ended' and this
        // timer belongs to a lifetime that is over. Deleting here would strip a
        // live agent of its label, parent and usage.
        if (this.sessions.get(sessionId)?.status !== 'ended') return;
        this.evictNow(sessionId);
      }, 30_000);
      evict.unref();
      this.evictionTimers.set(sessionId, evict);
    } else {
      applyHookEvent(session, event);
    }

    // A turn-end with a workflow / background subagent still running must
    // read 'background', not 'idle' — and the notifier has to see the
    // normalized state so "finished" only fires on a true idle.
    normalizeBackgroundAmbient(session);

    agentNotifier.notifyOnTransition(session, prevAmbient);
    this.nudgeParentOnFinish(session, prevAmbient);

    // Event-driven supervisor wake: when this agent just entered a real decision
    // point (approval or question), nudge any supervisor so it surfaces it now
    // rather than on its next poll. No-op when no supervisor is running. The
    // nudge itself is debounced (supervisorNudge.BLOCK_DEBOUNCE_MS) — most
    // blocks clear on their own within seconds, so onBlockCleared on the
    // matching un-block edge below is what makes that debounce work.
    const isBlocked = (s: SessionAmbientState) => s === 'waiting_approval' || s === 'waiting_input';
    if (isBlocked(session.ambientState) && !isBlocked(prevAmbient)) {
      supervisorNudge.onBlock(
        session,
        session.pendingApproval ? 'approval' : 'question',
        this.supervisorSessionIds(),
      );
    } else if (!isBlocked(session.ambientState) && isBlocked(prevAmbient)) {
      supervisorNudge.onBlockCleared(session.sessionId);
    }

    this.mergeWatcherData(session);
    this.pushUpdate(session);
  }

  /**
   * Fold a *managed* (Codex/OpenCode/Pi) session's mode into `ambientState`.
   * Pushed by `claudemonEventBridge` from claudemon's `/events` stream (the
   * `session.update` frames a managed adapter emits via `set_managed_mode`).
   * Managed backends fire no Claude hooks, so this is their only working / idle /
   * waiting signal — without it their status is stuck on the `'idle'` default.
   * No-op for unknown sessions or modes we don't surface.
   */
  applyManagedMode(
    sessionId: string,
    mode: string,
    meta?: {
      provider?: string;
      transport?: string;
      pending?: ManagedPendingWire | null;
      backgroundTasks?: number;
      subagents?: unknown[];
    },
  ): void {
    const session = this.sessions.get(sessionId);
    if (!session) return;
    // Ambient background work (a dev server, a poll loop, an async subagent)
    // rides its own count — the mode deliberately does not claim "working"
    // for it, so this is what the UI badges instead.
    if (typeof meta?.backgroundTasks === 'number') {
      session.backgroundTasks = meta.backgroundTasks;
    }
    if (Array.isArray(meta?.subagents)) {
      session.subagents = meta.subagents
        .map(normalizeManagedSubagent)
        .filter((sub): sub is SubagentInfo => !!sub);
    }
    // Daemon truth for sessions this process never spawned (adopted, or
    // restored after a desktop restart): backfill the backend identity so a
    // stream-transport Claude session still gates its pane correctly (no Term
    // view, structural /answer, hooks-enrichment-only). Never overwrite what
    // spawn metadata already recorded.
    if (meta?.provider && !session.provider) session.provider = meta.provider;
    if (meta?.transport === 'stream' && !session.transport) session.transport = 'stream';
    // The daemon's `pending` slot is the ONLY source for the approval/question
    // cards (needs-you dock, triage inbox, fleet-card buttons) for two cases:
    //   1. non-claude providers (codex/opencode/pi) fire no hooks at all;
    //   2. STREAM-transport Claude routes approvals through the control
    //      protocol (`can_use_tool`) rather than a PermissionRequest hook, so
    //      no hook ever populates pendingApproval for it.
    // Claude PTY keeps the hook-driven path — hookEventRouter owns these fields
    // for it, and driving them from both sources would race. Who owns the slot
    // is decided in ONE place (pendingSlotOwner, via the PendingSlot inside
    // applyManagedPending), not re-spelled here where it could drift from the
    // hook feed's answer. What is left for this call site is the orthogonal
    // question of whether the frame carried any pending info at all:
    // `undefined` means it did not (leave the slot alone); `null` means the
    // daemon says nothing is pending (resolve).
    if (meta && meta.pending !== undefined) {
      this.applyManagedPending(session, meta.pending);
    }
    let next: SessionAmbientState;
    switch (mode) {
      case 'responding':
        next = 'streaming';
        break;
      case 'approval':
        next = 'waiting_approval';
        break;
      case 'question':
        next = 'waiting_input';
        break;
      case 'input':
        // The daemon says ready-for-input, but spawned work (workflow /
        // background subagent) may still be running — don't read "idle" yet.
        next = sessionHasBackgroundWork(session) ? 'background' : 'idle';
        break;
      default:
        return; // 'unknown' / 'stopped' — leave the current state as-is
    }
    const prevAmbient = session.ambientState;
    session.ambientState = next;
    session.lastActivity = Date.now();
    if (next !== prevAmbient) {
      agentNotifier.notifyOnTransition(session, prevAmbient);
      this.nudgeParentOnFinish(session, prevAmbient);
      const isBlocked = (s: SessionAmbientState) =>
        s === 'waiting_approval' || s === 'waiting_input';
      if (isBlocked(next) && !isBlocked(prevAmbient)) {
        supervisorNudge.onBlock(
          session,
          next === 'waiting_approval' ? 'approval' : 'question',
          this.supervisorSessionIds(),
        );
      } else if (!isBlocked(next) && isBlocked(prevAmbient)) {
        supervisorNudge.onBlockCleared(session.sessionId);
      }
    }
    this.pushUpdate(session);
  }

  /**
   * Fold the daemon's `pending` slot into the approval/question card fields.
   * The daemon re-broadcasts Approval-mode frames on unrelated state changes,
   * so an unchanged card keeps its timestamp — bumping it would resurrect a
   * card the user already dismissed (the dock hides on dismissal timestamps).
   */
  private applyManagedPending(
    session: PendingFencedSession,
    pending: ManagedPendingWire | null,
  ): void {
    // The ownership check is the slot's, not this call site's: declaring the
    // feed IS the check. A daemon frame that reaches a hook-owned session (PTY
    // claude) or a peer's mirrored row writes nothing, and no future edit here
    // can forget to ask.
    const slot = new PendingSlot(session, 'daemon');
    if (pending?.kind === 'approval') {
      // Codex/OpenCode approval payloads carry the request params in `raw`
      // (no hook-style `tool_input` envelope); prefer the envelope when a
      // gateway-shaped payload has one, else show the params themselves.
      const raw = pending.raw ?? {};
      // Park, then resolve the other half: the daemon's slot holds ONE thing,
      // so an approval frame means no question is outstanding. (`parkApproval`
      // keeps an unchanged card's original timestamp — see PendingSlot.)
      slot.parkApproval({
        toolName: pending.tool ?? '',
        toolInput: raw.tool_input ?? raw,
        timestamp: Date.now(),
      });
      slot.resolveQuestions();
    } else if (pending?.kind === 'question') {
      slot.parkQuestions(
        (pending.questions ?? []).map((q) => ({
          question: q.question,
          header: q.header ?? undefined,
          multi_select: q.multi_select ?? false,
          options: q.options ?? [],
        })),
      );
      slot.resolveApproval();
    } else {
      // The daemon says nothing is pending — its `result` frame closed the turn.
      slot.resolveAll();
    }
  }

  // ── Conversation delta integration ──
  //
  // Fed by claudemonConversationBridge from claudemon's `/conversation/stream`.
  // The daemon owns transcript parsing; we just fold typed items into the
  // session. Sequence numbers detect missed frames (daemon restart, SSE lag):
  // on a gap we resync from the snapshot endpoint instead of guessing.

  applyConversationDelta(delta: ConversationDeltaWire): void {
    const sessionId = delta.session_id;
    if (!sessionId) return;
    let session = this.sessions.get(sessionId);
    if (!session) {
      // Deltas can outrun the first hook for adopted/external sessions —
      // create the entry the same way handleHookEvent would.
      session = this.createSession(sessionId, '');
    }

    if (delta.reset) {
      session.conversation = [];
      session.totalToolCalls = 0;
      this.convSeq.set(sessionId, 0);
    }

    const lastSeq = this.convSeq.get(sessionId) ?? 0;
    if (!delta.reset && delta.items.length === 0) {
      // Empty heartbeat — update stored seq and skip gap/resync logic entirely.
      this.convSeq.set(sessionId, delta.seq);
      return;
    }
    if (!delta.reset && delta.seq !== lastSeq + delta.items.length) {
      // Missed frames — rebuild from the daemon's snapshot.
      void this.resyncConversation(sessionId);
      return;
    }

    this.convSeq.set(sessionId, delta.seq);
    applyConversationItems(session, delta.items, (s, model, usage, key, sidechain) =>
      this.usageAccumulator.applyUsage(s, model, usage, key, sidechain),
    );
    session.lastActivity = Date.now();
    this.mergeWatcherData(session);
    checkBudget(session); // transcript-derived cost path (e.g. PTY sessions)
    this.pushUpdate(session);
    this.scheduleManagedHistory(session);
  }

  /** Snapshot a managed (codex/opencode) session into analytics. These backends
   *  don't fire Claude Stop/SessionEnd hooks, so we debounce a write off the
   *  conversation stream — the upsert is keyed by session_id, so repeated writes
   *  just refresh the row's usage. Claude sessions are skipped (they go through
   *  the hook path's Stop/SessionEnd writes). */
  private scheduleManagedHistory(session: ClaudeSessionState): void {
    const provider = session.provider;
    if (!provider || provider === 'claude') return;
    const sessionId = session.sessionId;
    const existing = this.managedHistoryTimers.get(sessionId);
    if (existing) clearTimeout(existing);
    const timer = setTimeout(() => {
      this.managedHistoryTimers.delete(sessionId);
      const s = this.sessions.get(sessionId);
      if (s) writeHistory(s, 'active');
    }, 2500);
    timer.unref?.();
    this.managedHistoryTimers.set(sessionId, timer);
  }

  /** Replace a session's conversation with the daemon's full parsed history. */
  private async resyncConversation(sessionId: string): Promise<void> {
    if (this.resyncing.has(sessionId)) return;
    this.resyncing.add(sessionId);
    try {
      const res = await fetch(`${CLAUDEMON_API_URL}/sessions/${sessionId}/conversation`);
      if (!res.ok) return;
      const snap = (await res.json()) as { seq: number; items: ConversationDeltaWire['items'] };
      const session = this.sessions.get(sessionId);
      if (!session) return;
      session.conversation = [];
      session.totalToolCalls = 0;
      applyConversationItems(session, snap.items ?? [], (s, model, usage, key, sidechain) =>
        this.usageAccumulator.applyUsage(s, model, usage, key, sidechain),
      );
      this.convSeq.set(sessionId, snap.seq ?? 0);
      this.mergeWatcherData(session);
      this.pushUpdate(session);
    } catch (err) {
      console.warn(`[SessionStore] conversation resync failed for ${sessionId}:`, err);
    } finally {
      this.resyncing.delete(sessionId);
    }
  }

  // ── StatusLine integration ──
  //
  /** The provider label ('claude' | 'codex' | 'opencode' | …) for a known
   *  session, if any. Used to key per-account rate-limit dedup — distinct
   *  providers are distinct accounts with independent usage windows. */
  providerOf(sessionId: string): string | undefined {
    return this.sessions.get(sessionId)?.provider;
  }

  // Fed by claudemonStatusLineBridge from claudemon's `/statusline/stream`.
  // The id is already canonical (claudemon resolved the alias). We only attach
  // to a session we already know — the statusLine fires repeatedly, so if the
  // first hook hasn't created the session yet, the next tick lands. We don't
  // bump lastActivity: statusLine ticks aren't activity and shouldn't keep an
  // idle session looking busy.
  applyStatusLine(sessionId: string, statusLine: SessionStatusLine): void {
    if (!sessionId) return;
    const session = this.sessions.get(sessionId);
    if (!session) return;
    // Always record the latest value immediately (trailing-edge debounce).
    session.statusLine = statusLine;
    // Managed providers (codex/opencode/pi) never emit a transcript `usage`
    // item, so applyUsage (the only other peakContext writer) never runs for
    // them — without this, their peak-context analytics stay 0 forever.
    const slContextTokens = contextTokensFromStatusLine(statusLine);
    if (slContextTokens !== undefined && slContextTokens > session.peakContext) {
      session.peakContext = slContextTokens;
    }
    // The status line carries the provider's OWN context window
    // (`contextWindowSize`) — the one fact that settles 1M-vs-200k. It can land
    // after the session's last usage turn, so fold it in here too, or the bar
    // keeps the denominator the transcript's (marker-stripped) model id implied.
    SessionUsageAccumulator.refreshContextLimit(session);
    // Stream sessions get Claude's authoritative cost here — check the budget.
    checkBudget(session);
    if (STATUSLINE_DEBOUNCE_MS <= 0) {
      // Debounce disabled — original immediate-push behaviour.
      this.pushUpdate(session);
      return;
    }
    // Cancel any previously scheduled flush for this session's statusLine.
    const existing = this.statusLineTimers.get(sessionId);
    if (existing) clearTimeout(existing);
    const timer = setTimeout(() => {
      this.statusLineTimers.delete(sessionId);
      const s = this.sessions.get(sessionId);
      if (s) this.pushUpdate(s);
    }, STATUSLINE_DEBOUNCE_MS);
    this.statusLineTimers.set(sessionId, timer);
  }

  // ── Workflow watcher integration ──

  /** Callback from workflowWatcher's poll loop — fold in and broadcast. */
  private applyWatcherUpdate(sessionId: string, update: WorkflowWatcherUpdate): void {
    const session = this.sessions.get(sessionId);
    if (!session) return;
    this.watcherUpdates.set(sessionId, update);
    this.mergeWatcherData(session);
    // The watcher is what sees a workflow finish (no hook fires for that):
    // drop 'background' back to 'idle' here — or raise it if a run appeared —
    // and let the notifier fire "finished" on the true idle transition.
    const prevAmbient = session.ambientState;
    normalizeBackgroundAmbient(session);
    if (session.ambientState !== prevAmbient) {
      agentNotifier.notifyOnTransition(session, prevAmbient);
      this.nudgeParentOnFinish(session, prevAmbient);
    }
    this.pushUpdate(session);
  }

  /**
   * Merge the latest filesystem-derived workflow state into the session:
   * adopt workflow runs, enrich hook-driven subagents with live transcript
   * activity, and drop subagents that actually belong to a workflow run
   * (they render inside the run card instead).
   */
  private mergeWatcherData(session: ClaudeSessionState): void {
    const update = this.watcherUpdates.get(session.sessionId);
    if (!update) return;

    session.workflows = update.runs;
    // Republish run/agent transitions onto the hub bus for the rules engine.
    publishWorkflowRuns({ sessionId: session.sessionId, cwd: session.cwd }, update.runs);

    const stripPrefix = (s: string) => s.replace(/^agent-/, '');
    const workflowIds = new Set(update.workflowAgentIds);
    session.subagents = session.subagents.filter((s) => !workflowIds.has(stripPrefix(s.id)));
    for (const sub of session.subagents) {
      const activity = update.subagentActivity[stripPrefix(sub.id)];
      if (!activity) continue;
      if (activity.description) sub.description = activity.description;
      if (activity.toolUseId) sub.toolUseId = activity.toolUseId;
      if (activity.model) sub.model = activity.model;
      if (activity.tokens !== undefined) sub.tokens = activity.tokens;
      if (activity.costUSD !== undefined) sub.costUSD = activity.costUSD;
      if (activity.toolCalls !== undefined) sub.toolCalls = activity.toolCalls;
      if (activity.lastToolName) sub.lastToolName = activity.lastToolName;
      if (activity.lastToolSummary !== undefined) sub.lastToolSummary = activity.lastToolSummary;
    }
  }

  // ── Federation: remote (peer-hub) sessions ──
  //
  // Fed by federationBridge from hub-stamped agent.* events and peer seeding.
  // Remote sessions live in the same map so every read path (renderer getAll,
  // agents.list, sessions.snapshots) sees one fleet, but they are driven ONLY
  // by these methods: no hook events, deltas, statusLines or watcher updates
  // ever carry a remote id, so none of the local side-effect machinery
  // (history/analytics writes, eviction/Stop timers, facade tokens, notifier,
  // supervisor nudges) can touch them. snapshotGrantsFsRoot refuses hub-set
  // rows, so a remote cwd never becomes a local fs root.

  /** Upsert one remote session from the peer's snapshot wire shape. */
  upsertRemoteSession(hub: string, snap: RemoteSnapshotWire): void {
    if (!hub) return;
    const sessionId = snap?.sessionId;
    if (!sessionId || typeof sessionId !== 'string') return;
    const existing = this.sessions.get(sessionId);
    if (existing && (existing.hub ?? '') !== hub) {
      // A peer must never overwrite a LOCAL session (or another peer's) that
      // happens to share the id — refusing beats silently rebinding actions
      // (approve/answer routing keys off the stored hub).
      console.warn(
        `[SessionStore] refusing remote upsert from hub "${hub}" over ` +
          `${existing.hub ? `hub "${existing.hub}"` : 'a local'} session ${sessionId}`,
      );
      return;
    }
    if (snap.sparse) {
      this.upsertSparseRemoteSession(hub, sessionId, snap, existing);
      return;
    }
    // Once federationBridge has folded the peer's full conversation in
    // (remoteConvSeq set), the item stream owns `conversation` — a bounded
    // window push must not truncate the history back to twelve turns. The
    // window still refreshes every state field around it.
    const folded = this.remoteConvSeq.has(sessionId);
    // Typed WITHOUT the pending slot: naming either field in this literal is an
    // excess property, so the row's card can only come from the intent declared
    // in `bornWithPending` below. (The `...snap` spread still carries the wire
    // values at runtime; `bornWithPending` overwrites them before the fill.)
    const draft: SessionWithoutPending = {
      ...(existing ?? {}),
      ...snap,
      sessionId,
      cwd: snap.cwd ?? existing?.cwd ?? '',
      ptyId: snap.ptyId ?? sessionId,
      // A remote transcript path names a file on the PEER machine; blank it so
      // no local consumer (watchers, file links) treats it as openable here.
      transcriptPath: '',
      status: snap.status ?? existing?.status ?? 'active',
      conversation:
        folded && existing
          ? existing.conversation
          : (snap.conversation ?? existing?.conversation ?? []),
      activeToolCalls: snap.activeToolCalls ?? [],
      completedToolCalls: snap.completedToolCalls ?? [],
      fileChanges: snap.fileChanges ?? [],
      subagents: snap.subagents ?? [],
      workflows: snap.workflows ?? [],
      ambientState: snap.ambientState ?? existing?.ambientState ?? 'idle',
      startedAt: snap.startedAt ?? existing?.startedAt ?? Date.now(),
      lastActivity: snap.lastActivity ?? Date.now(),
      totalToolCalls: snap.totalToolCalls ?? 0,
      peakContext: snap.peakContext ?? 0,
      usage: snap.usage ?? null,
      hub,
      hubOffline: false,
    };
    // The peer's slot, mirrored. A full snapshot is authoritative in both
    // directions: a card it carries is parked, its absence is a resolve.
    const session = bornWithPending(draft, 'federation', existing, (slot) => {
      const wire = snap.pendingApproval;
      if (wire) slot.parkApproval(wire);
      else slot.resolveApproval();
      const questions = snap.pendingQuestions;
      if (questions) slot.parkQuestions(questions);
      else slot.resolveQuestions();
    });
    if (folded && existing) this.carryConversationAnchors(existing, session);
    this.sessions.set(sessionId, session);
    this.pushUpdate(session);
  }

  /**
   * A `sparse` remote row. Two producers share the marker:
   *
   *   - a headless brain (`workspacer serve`, no desktop) publishing its live
   *     claudemon rows with the desktop field names overlaid
   *     (services/hub/cmd/brain/enrich.go compatSnapshot) — these ARE live
   *     sessions and must become usable cards, or a brain-only peer's whole
   *     fleet is invisible here while /m shows it fine;
   *   - a peer desktop's layout-ghost stopped rows (hubCapabilities
   *     sessions.snapshots), status 'ended' — its local respawn affordance,
   *     not a live session.
   *
   * Mapped explicitly, never spread: the wire row carries claudemon's
   * snake_case originals (mode / session_id / pending / …) and the `sparse`
   * marker itself, none of which may leak into the renderer's snapshot.
   * Conversation stays whatever the federation fetch folded in (sparse rows
   * never carry one); approve/reply already route over the bus by `hub`.
   */
  private upsertSparseRemoteSession(
    hub: string,
    sessionId: string,
    snap: RemoteSnapshotWire,
    existing: ClaudeSessionState | undefined,
  ): void {
    if ((snap.status ?? 'active') === 'ended') {
      // Not a card here — but if we held it live, this is its end: same final
      // 'ended' push + drop the reseed gives sessions a peer stops reporting.
      if (existing) this.dropRemoteSession(existing);
      return;
    }
    const draft: SessionWithoutPending = {
      sessionId,
      cwd: snap.cwd ?? existing?.cwd ?? '',
      ptyId: existing?.ptyId ?? sessionId,
      transcriptPath: '',
      status: 'active',
      conversation: existing?.conversation ?? [],
      activeToolCalls: existing?.activeToolCalls ?? [],
      completedToolCalls: existing?.completedToolCalls ?? [],
      fileChanges: existing?.fileChanges ?? [],
      subagents: existing?.subagents ?? [],
      workflows: existing?.workflows ?? [],
      plan: snap.plan ?? existing?.plan,
      ambientState: snap.ambientState ?? existing?.ambientState ?? 'idle',
      startedAt: existing?.startedAt ?? snap.startedAt ?? Date.now(),
      lastActivity: snap.lastActivity ?? Date.now(),
      totalToolCalls: snap.totalToolCalls ?? existing?.totalToolCalls ?? 0,
      peakContext: existing?.peakContext ?? 0,
      usage: snap.usage ?? existing?.usage ?? null,
      label: snap.label ?? existing?.label,
      parentSessionId: snap.parentSessionId ?? existing?.parentSessionId,
      provider: snap.provider ?? existing?.provider,
      transport: snap.transport ?? existing?.transport,
      // isWakeTarget deliberately NOT mapped: supervisorSessionIds() feeds the
      // LOCAL supervisorNudge, which can only message local claudemon sessions.
      hub,
      hubOffline: false,
    };
    // The peer's slot, mirrored — same intent as the full path. Brain rows
    // re-send the same parked approval on unrelated updates and carry no
    // timestamp of their own, so an unchanged card must keep the one it was
    // first stamped with or the needs-you dock resurrects cards the user
    // already dismissed; that is `parkApproval`'s Keep rule, shared now with
    // the daemon feed instead of hand-rolled a second time here. Questions are
    // set explicitly by brain rows (null = cleared), so their absence resolves.
    const session = bornWithPending(draft, 'federation', existing, (slot) => {
      const wire = snap.pendingApproval;
      if (wire) {
        slot.parkApproval({
          toolName: wire.toolName ?? '',
          toolInput: wire.toolInput,
          suggestions: wire.suggestions,
          timestamp: wire.timestamp ?? Date.now(),
        });
      } else {
        slot.resolveApproval();
      }
      const questions = snap.pendingQuestions;
      if (questions) slot.parkQuestions(questions);
      else slot.resolveQuestions();
    });
    if (existing) this.carryConversationAnchors(existing, session);
    this.sessions.set(sessionId, session);
    this.pushUpdate(session);
  }

  /**
   * The wire window's `conversationOffset` / `conversationUserOffset` describe
   * ITS bounded slice, not the folded full history this session keeps — carry
   * the store's own anchors forward so ClaudePane's absolute-index keys and
   * optimistic-bubble counts stay honest. (Neither field is declared on
   * ClaudeSessionState — see bounds.ts for why the writes go through a
   * structural cast.)
   */
  private carryConversationAnchors(from: ClaudeSessionState, to: ClaudeSessionState): void {
    type Anchored = { conversationOffset?: number; conversationUserOffset?: number };
    const src = from as unknown as Anchored;
    const dst = to as unknown as Anchored;
    dst.conversationOffset = src.conversationOffset ?? 0;
    dst.conversationUserOffset = src.conversationUserOffset ?? 0;
  }

  /** One final 'ended' push so every consumer clears the card, then remove. */
  private dropRemoteSession(session: ClaudeSessionState): void {
    session.status = 'ended';
    session.hubOffline = false;
    this.pushUpdate(session);
    // Flush synchronously: flushSession reads from the map, so the delete
    // below would otherwise swallow the coalesced final update.
    this.flushPending(session.sessionId);
    this.sessions.delete(session.sessionId);
    this.remoteConvSeq.delete(session.sessionId);
  }

  /** Last folded remote-conversation sequence, if any (federation bookkeeping —
   *  federationBridge passes it back to the peer as `sinceSeq`). */
  remoteConversationSeq(sessionId: string): number | undefined {
    return this.remoteConvSeq.get(sessionId);
  }

  /**
   * Fold a remote session's conversation — the peer's `sessions.conversation`
   * result, claudemon's typed items — into the store, so the normal snapshot
   * push carries the real transcript to every consumer instead of the
   * compacted window the peer's agent.snapshot events hold. `rebuild` replaces
   * history from the top (first fetch, or a peer-side seq reset); otherwise
   * items append incrementally after the last folded seq. Once folded, window
   * pushes stop overwriting `conversation` (see upsertRemoteSession): the item
   * stream is the conversation's single source of truth, exactly as the delta
   * stream is for local sessions.
   */
  applyRemoteConversation(
    hub: string,
    sessionId: string,
    seq: number,
    items: ConversationItemWire[],
    rebuild: boolean,
  ): void {
    const session = this.sessions.get(sessionId);
    if (!session || session.hub !== hub) return;
    if (rebuild) {
      session.conversation = [];
      session.totalToolCalls = 0;
    }
    this.remoteConvSeq.set(sessionId, seq);
    if (!rebuild && items.length === 0) return; // nothing new — skip the push
    // No usage callback: the peer's own snapshot `usage` field is authoritative
    // (refreshed by every window push), and accumulating locally would record
    // the peer's models into this machine's seen-model state.
    applyConversationItems(session, items, () => {});
    this.pushUpdate(session);
  }

  /** Peer link went down: tombstone its sessions (keep them, flag them). */
  markHubPeerOffline(hub: string): void {
    if (!hub) return;
    for (const s of this.sessions.values()) {
      if (s.hub === hub && !s.hubOffline) {
        s.hubOffline = true;
        this.pushUpdate(s);
      }
    }
  }

  /**
   * Replace a peer's remote sessions wholesale (connect / reconnect seed).
   * Sessions the peer no longer reports get one final 'ended' push — so the
   * renderer clears the card instead of holding a stale tombstone — and are
   * then dropped; the rest are upserted, which also clears `hubOffline`.
   */
  reseedRemoteSessions(hub: string, snaps: RemoteSnapshotWire[]): void {
    if (!hub) return;
    // A row counts as live whether rich or sparse — a headless-brain peer's
    // whole fleet is sparse, and dropping those here made such peers invisible
    // on the desktop. Ended rows (a peer desktop's layout-ghost stopped rows,
    // a brain's stopped claudemon rows) are not cards and not kept.
    const keep = new Set<string>();
    for (const snap of snaps) {
      if (snap?.sessionId && snap.status !== 'ended') keep.add(snap.sessionId);
    }
    for (const s of Array.from(this.sessions.values())) {
      if (s.hub !== hub || keep.has(s.sessionId)) continue;
      this.dropRemoteSession(s);
    }
    for (const snap of snaps) {
      if (snap?.sessionId && snap.status !== 'ended') this.upsertRemoteSession(hub, snap);
    }
  }

  /**
   * Light-touch ambient update from a hub-stamped `agent.state_changed` (the
   * peer hub's Go claudemon bridge emits these even when no desktop runs there
   * to publish full snapshots). Deliberately NO agentNotifier / supervisorNudge
   * here (v1): the peer's own desktop already notifies, and remote attention
   * still surfaces through the snapshot fields on every consumer.
   */
  applyRemoteStateChange(hub: string, sessionId: string, mode: string): void {
    const session = this.sessions.get(sessionId);
    if (!session || session.hub !== hub) return;
    let next: SessionAmbientState;
    switch (mode) {
      case 'responding':
        next = 'streaming';
        break;
      case 'approval':
        next = 'waiting_approval';
        break;
      case 'question':
        next = 'waiting_input';
        break;
      case 'input':
        // Same question applyManagedMode asks two hundred lines above, and for
        // the same reason: the peer says ready-for-input, but a workflow, an
        // async subagent or a background shell mirrored onto this row may still
        // be running. This arm read a bare 'idle' and was the one mode applier
        // that skipped the check — a federated row went "done" while its work
        // carried on.
        next = sessionHasBackgroundWork(session) ? 'background' : 'idle';
        break;
      default:
        return; // 'unknown' / 'stopped' — leave as-is; the snapshot feed owns ends
    }
    if (session.ambientState === next) return;
    session.ambientState = next;
    session.lastActivity = Date.now();
    this.pushUpdate(session);
  }

  // ── Queries ──

  // The clone is shallow BY DESIGN for everything except the pending slot: a
  // snapshot is read constantly (the fleet list, the hub facade, every IPC
  // fetch) and deep-copying a full conversation on each one is not affordable.
  // The slot is the exception because it is the one field with an invariant —
  // exactly one feed owns it — and a shallow clone handed every caller a live
  // reference to it. See `detachPendingSlot`.
  getSnapshot(sessionId: string): ClaudeSessionSnapshot | null {
    const session = this.sessions.get(sessionId);
    if (!session) return null;
    this.refreshOrphanStatus(session);
    return { ...session, ...detachPendingSlot(session) };
  }

  getAllSnapshots(): ClaudeSessionSnapshot[] {
    return Array.from(this.sessions.values()).map((s) => {
      this.refreshOrphanStatus(s);
      return { ...s, ...detachPendingSlot(s) };
    });
  }

  // ── Internals ──

  /**
   * Forget a session and every per-session map keyed on it. The body of the
   * SessionEnd eviction timer, extracted so `closeSession` can run the SAME
   * teardown on demand — a second, hand-copied cleanup would be the way one of
   * these maps quietly starts leaking again.
   *
   * Every per-session Map/Set must be cleared, not just `sessions`: convSeq and
   * watcherUpdates otherwise retain one entry per ended session for the whole
   * process lifetime. Clearing the stale convSeq also lets a resumed (reused-id)
   * session start fresh instead of reading its first delta as a gap.
   */
  private evictNow(sessionId: string): void {
    // The row is the ONLY record that this session was a manager (isWakeTarget
    // and label live in this process's memory — claudemon has no such fields),
    // so a manager's identity has to be copied out HERE or it is gone. This is
    // the single teardown path on purpose: the SessionEnd timer and
    // close_session both come through it, and a manager dismissed by hand
    // orphans its workers exactly as a crashed one does.
    const dying = this.sessions.get(sessionId);
    if (dying?.isWakeTarget && !dying.hub) {
      this.managerTombstones.set(sessionId, {
        sessionId,
        label: dying.label,
        cwd: dying.cwd,
        endedAt: Date.now(),
        provider: dying.provider,
      });
    }
    this.sessions.delete(sessionId);
    this.usageAccumulator.forget(sessionId);
    this.convSeq.delete(sessionId);
    this.watcherUpdates.delete(sessionId);
    this.resyncing.delete(sessionId);
    this.spawnMeta.delete(sessionId);
    // The session's MCP-facade token (if it had one) is a live bearer secret in
    // tokens.json; the session is over, so cut it off. A respawn onto this id
    // re-mints. Boot-time sweepSessionFacadeTokens catches sessions that ended
    // while the desktop wasn't running.
    try {
      revokeSessionFacadeTokens(sessionId);
    } catch (err) {
      console.warn(`[claudeSessionStore] facade token revoke failed for ${sessionId}:`, err);
    }
    for (const timers of [this.statusLineTimers, this.managedHistoryTimers, this.pendingFlush]) {
      const t = timers.get(sessionId);
      if (t) {
        clearTimeout(t);
        timers.delete(sessionId);
      }
    }
    // Any live child of THIS session had its `orphan` field computed while this
    // row was still alive — that fact just flipped, and nothing else will push
    // an update for a child whose own state didn't change. Refresh them now so
    // the renderer's "Unwatched" chip lights up the moment the dispatcher dies
    // instead of waiting on the child's next unrelated hook tick.
    for (const child of this.sessions.values()) {
      if (child.parentSessionId === sessionId && !child.hub) this.pushUpdate(child);
    }
    // This session may have been the last child of an earlier dead manager —
    // and the tombstone just written is itself pointless unless children of it
    // survive. Both are the same question, asked here so the map is bounded by
    // the fleet whether or not anyone ever reads it.
    this.pruneManagerTombstones();
  }

  /**
   * `close_session`: dismiss a finished session's row on demand.
   *
   * The row of a worker that was SIGTERM'd lingers — the 30s eviction timer is
   * armed by a SessionEnd hook, and a killed process often emits none — so
   * "is it actually dead" was answered by sending it another signal and reading
   * the 404. This makes dismissal the first-class verb it should have been.
   *
   * REFUSES a session that is still WORKING. Dismissing a running agent would
   * strip it from list_agents while it kept burning tokens, which is the one
   * outcome worse than a lingering row: the manager would believe it was gone.
   * Stop it first (signal SIGTERM) — or dismiss it once it is idle, at which
   * point this also tears the daemon side down so "dismissed" is not a lie.
   *
   * Idempotent: a session already forgotten reports success, because a caller
   * asking "make this row go away" and being told "no such row" as an ERROR is
   * exactly the ambiguity this replaces.
   */
  closeSession(sessionId: string): { ok: true; removed: boolean; wasLive: boolean } {
    const session = this.sessions.get(sessionId);
    if (!session) return { ok: true, removed: false, wasLive: false };
    const working =
      session.status !== 'ended' &&
      (session.ambientState === 'thinking' ||
        session.ambientState === 'streaming' ||
        session.ambientState === 'background');
    if (working) {
      throw new Error(
        `close_session: ${sessionId} is still working (${session.ambientState}). Dismissing it ` +
          `would hide a running agent from list_agents while it kept spending — stop it first ` +
          `(signal SIGTERM), then close it.`,
      );
    }
    const wasLive = session.status !== 'ended';
    this.cancelEviction(sessionId);
    this.evictNow(sessionId);
    return { ok: true, removed: true, wasLive };
  }

  /** Drop any pending SessionEnd eviction for this id — it belongs to a
   *  lifetime that has been superseded. Safe to call when none is pending. */
  private cancelEviction(sessionId: string): void {
    const t = this.evictionTimers.get(sessionId);
    if (!t) return;
    clearTimeout(t);
    this.evictionTimers.delete(sessionId);
  }

  private createSession(sessionId: string, cwd: string): PendingFencedSession {
    // A reused id means the previous life's eviction is still armed.
    this.cancelEviction(sessionId);
    // Typed without the pending slot for the same reason the remote rows are:
    // the fields cannot be named here, so `bornWithEmptyPending` below is the
    // only statement in this file of what a new row's slot holds.
    const draft: SessionWithoutPending = {
      sessionId,
      cwd,
      ptyId: sessionId, // legacy field — renderer keys by this; we make it == sessionId
      transcriptPath: '',
      status: 'active',
      conversation: [],
      activeToolCalls: [],
      completedToolCalls: [],
      fileChanges: [],
      subagents: [],
      workflows: [],
      ambientState: 'idle',
      startedAt: Date.now(),
      lastActivity: Date.now(),
      totalToolCalls: 0,
      peakContext: 0,
      usage: null,
    };
    const session = bornWithEmptyPending(draft);
    // Apply any pre-registered spawn metadata (label, parentSessionId) so the
    // snapshot is enriched before the first push to the renderer.
    const meta = this.spawnMeta.get(sessionId);
    if (meta) {
      session.label = meta.label;
      session.parentSessionId = meta.parentSessionId;
      session.isWakeTarget = meta.isWakeTarget;
      session.provider = meta.provider;
      session.transport = meta.transport;
      session.settings = meta.settings;
      session.resultSchema = meta.resultSchema;
      this.spawnMeta.delete(sessionId);
    }
    this.sessions.set(sessionId, session);
    return session;
  }

  /**
   * Record the reasoning-effort level a live switch just asked for.
   *
   * Unlike the permission mode, nothing will ever come along and confirm this
   * for a Claude session — its effective effort appears in no hook, no status
   * line and no init frame — so this note IS the pill's truth there. Codex does
   * confirm, on `thread/settings/updated`, and that lands on the status line and
   * takes precedence.
   */
  noteEffort(sessionId: string, effort: string): void {
    const session = this.sessions.get(sessionId);
    if (!session) return;
    session.liveEffort = effort;
    this.pushUpdate(session);
  }

  /**
   * Record a permission-mode change confirmed by claudemon's live switch.
   * Hooks will carry the same value on the next event; this keeps the pill
   * honest in the meantime (there is no hook for the switch itself).
   */
  notePermissionMode(sessionId: string, mode: string): void {
    const session = this.sessions.get(sessionId);
    if (!session) return;
    session.livePermissionMode = mode;
    this.pushUpdate(session);
  }

  /**
   * Optimistically clear the pending question set once an answer was ACCEPTED
   * — by the daemon (`POST /answer` returned) or by the peer that owns a remote
   * row (`claude.answer` returned). Without it the answered questions linger on
   * every surface (pane dock, inbox, fleet cards) until the confirmation lands,
   * which reads as the picker re-prompting.
   *
   * The one write to the slot that is deliberately NOT ownership-gated, because
   * it is not this feed guessing about another's state — it is the resolution
   * of the exact request that feed parked. `acknowledgeAnswer` is the word for
   * that, and it is narrow (questions only, no-op when nothing is parked) so
   * being ungated stays safe; see ./sessionStore/pendingSlot.
   */
  clearPendingQuestions(sessionId: string): void {
    const session = this.sessions.get(sessionId);
    if (!session || !session.pendingQuestions) return;
    acknowledgeAnswer(session);
    this.pushUpdate(session);
  }

  /**
   * Recompute `session.orphan` from live rows + `managerTombstones` — the same
   * facts `orphanCandidates` reports, read in the other direction (child looks
   * up its own parent instead of a caller grouping by parent). Called at every
   * point a snapshot leaves the store (push, getSnapshot, getAllSnapshots), so
   * the field is always fresh at the moment it's read and there is no separate
   * state to keep in sync — it is thrown away and rebuilt every time.
   */
  private refreshOrphanStatus(session: ClaudeSessionState): void {
    const parentId = session.parentSessionId;
    // Federated sessions: parentSessionId (if set) names a session on the PEER,
    // not a row this process can judge — same exclusion orphanCandidates
    // applies to federated children.
    if (!parentId || session.hub) {
      session.orphan = undefined;
      return;
    }
    const row = this.sessions.get(parentId);
    if (row && row.status !== 'ended') {
      session.orphan = undefined; // parent alive: not orphaned
      return;
    }
    const tomb = this.managerTombstones.get(parentId);
    session.orphan = { confirmedManager: Boolean(tomb) || row?.isWakeTarget === true };
  }

  private pushUpdate(session: ClaudeSessionState): void {
    this.refreshOrphanStatus(session);
    if (!COALESCE_SNAPSHOT_UPDATES) {
      // Original immediate-send path (byte-for-byte identical behaviour).
      // Federation: never republish a REMOTE session onto the local bus — it
      // arrived FROM a peer (hub-stamped envelope), and publishSnapshot would
      // re-emit it as an unlabelled local agent.snapshot (a duplicate to every
      // bus client, and an event-loop seed if this hub is itself a peer).
      if (!session.hub) publishSnapshot(() => ({ ...session, ...detachPendingSlot(session) }));
      if (!this.mainWindow || this.mainWindow.isDestroyed()) return;
      this.mainWindow.webContents.send('claude-session:update', session.sessionId, { ...session });
      return;
    }
    // Coalescing path: schedule a single flush per session per ~16 ms window.
    if (!this.pendingFlush.has(session.sessionId)) {
      const id = session.sessionId;
      const timer = setTimeout(() => {
        this.pendingFlush.delete(id);
        this.flushSession(id);
      }, 16);
      this.pendingFlush.set(id, timer);
    }
  }

  /** Emit one IPC message for a session with its latest state. */
  private flushSession(sessionId: string): void {
    const session = this.sessions.get(sessionId);
    if (!session) return;
    this.refreshOrphanStatus(session);
    // Mirror onto the hub bus for the web build (no-op when remote sharing is
    // off). Passed as a factory so the object spread is skipped entirely when
    // the hub won't use it. Federation: remote sessions are never republished
    // (see the identical guard on the non-coalesced path above).
    if (!session.hub) publishSnapshot(() => ({ ...session, ...detachPendingSlot(session) }));
    if (!this.mainWindow || this.mainWindow.isDestroyed()) return;
    this.mainWindow.webContents.send('claude-session:update', session.sessionId, { ...session });
  }

  /**
   * Synchronously flush any pending coalesced update for a session. Call this
   * before session end so the final state is never dropped.
   */
  private flushPending(sessionId: string): void {
    const timer = this.pendingFlush.get(sessionId);
    if (timer) {
      clearTimeout(timer);
      this.pendingFlush.delete(sessionId);
      this.flushSession(sessionId);
    }
  }
}

export const claudeSessionStore = new ClaudeSessionStore();
