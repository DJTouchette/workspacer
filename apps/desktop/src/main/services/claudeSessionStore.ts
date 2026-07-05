import * as path from 'path';
import { BrowserWindow } from 'electron';
import { agentNotifier } from './agentNotifier';
import { supervisorNudge } from './supervisorNudge';
import {
  workflowWatcher,
  type WorkflowRunInfo,
  type WorkflowWatcherUpdate,
} from './workflowWatcher';
import { publishWorkflowRuns, publishSnapshot, forgetSession as forgetTelemetry } from './hubTelemetry';
import { applyHookEvent, applyStopEvent, applySessionEndEvent } from './sessionStore/hookEventRouter';
import { applyConversationItems, type ConversationDeltaWire } from './sessionStore/conversationApplier';
import { SessionUsageAccumulator } from './sessionStore/usageAccumulator';
import { CLAUDEMON_API_URL } from './claudemonDaemon';
import { writeHistory } from './sessionStore/analyticsWriter';

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
export type SessionAmbientState = 'idle' | 'thinking' | 'streaming' | 'waiting_input' | 'waiting_approval';

/** Launch settings requested at spawn/restart time (composer pill truth
 *  fallback — live statusLine/usage wins for the model when present). */
export interface SessionSpawnSettings {
  model?: string;
  effort?: string;
  permissionMode?: string;
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
  timestamp: number;
  toolCalls?: ToolCall[];
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
  contextUsedPct?: number;
  contextWindowSize?: number;
  totalInputTokens?: number;
  totalOutputTokens?: number;
  costUSD?: number;
  fiveHourPct?: number;
  fiveHourResetsAt?: number;
  sevenDayPct?: number;
  sevenDayResetsAt?: number;
  receivedAt?: string;
}

export interface ClaudeSessionState {
  sessionId: string;
  cwd: string;
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

  ambientState: SessionAmbientState;
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
  /** True for fleet-supervisor sessions — they get nudged when another agent
   *  blocks on a decision (see supervisorNudge). */
  isSupervisor?: boolean;
  /** Coding-agent backend ('claude' | 'codex' | 'opencode'), for analytics. */
  provider?: string;
  /** Requested-at-spawn launch settings — what the composer pills show when no
   *  live telemetry (statusLine/usage model) is available yet. */
  settings?: SessionSpawnSettings;
  /** Current permission mode from hook payloads (`permission_mode` rides on
   *  PreToolUse/PostToolUse/UserPromptSubmit/Stop). Unlike `settings`, this
   *  tracks live changes — e.g. shift+tab cycling in the TUI. Claude sessions
   *  only; managed providers fire no hooks so it stays unset for them. */
  livePermissionMode?: string;
  /** Guards against double history writes (Stop 1500ms timeout vs SessionEnd). */
  historyWritten?: boolean;
}

// Serialisable snapshot sent over IPC
export type ClaudeSessionSnapshot = Omit<ClaudeSessionState, never>;

// ── Store ──

class ClaudeSessionStore {
  private sessions = new Map<string, ClaudeSessionState>();
  private mainWindow: BrowserWindow | null = null;
  // Latest workflow/subagent filesystem state per session, re-merged whenever
  // either the watcher ticks or a hook event mutates the subagent list.
  private watcherUpdates = new Map<string, WorkflowWatcherUpdate>();
  // Accumulator owns lastUsageKey + knownModels dedup state.
  private usageAccumulator = new SessionUsageAccumulator();
  // Pre-spawn metadata keyed by pinned session id. Recorded before the first
  // hook arrives so adopted cards carry a name and parent from the start.
  private spawnMeta = new Map<string, { label?: string; parentSessionId?: string; isSupervisor?: boolean; provider?: string; settings?: SessionSpawnSettings }>();
  // Last-applied conversation sequence per session (gap detection for the
  // daemon's delta stream) and sessions with a snapshot resync in flight.
  private convSeq = new Map<string, number>();
  private resyncing = new Set<string>();
  // Coalescing: sessions with a pending flush scheduled (COALESCE_SNAPSHOT_UPDATES).
  private pendingFlush = new Map<string, NodeJS.Timeout>();
  // Debounce: per-session statusLine debounce timers (STATUSLINE_DEBOUNCE_MS).
  private statusLineTimers = new Map<string, NodeJS.Timeout>();
  // Debounce: per-managed-session analytics snapshot timers. Managed (codex /
  // opencode) sessions don't fire Claude Stop/SessionEnd hooks, so we snapshot
  // their history off the conversation stream instead (see scheduleManagedHistory).
  private managedHistoryTimers = new Map<string, NodeJS.Timeout>();

  /** Record name/parent for a session about to be spawned, keyed by its pinned
   *  id. Consumed when the session first registers (see createSession), so an
   *  adopted card can be named and nested under its spawner. */
  setSpawnMeta(sessionId: string, meta: { label?: string; parentSessionId?: string; isSupervisor?: boolean; provider?: string; settings?: SessionSpawnSettings }): void {
    if (!sessionId) return;
    this.spawnMeta.set(sessionId, meta);
    // A restart-with-settings re-spawns onto an id that may still have a live
    // entry — refresh its settings in place so the pills track the request.
    const existing = this.sessions.get(sessionId);
    if (existing && meta.settings) {
      existing.settings = { ...existing.settings, ...meta.settings };
      this.pushUpdate(existing);
    }
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
    for (const s of this.sessions.values()) if (s.isSupervisor) ids.push(s.sessionId);
    return ids;
  }

  setMainWindow(win: BrowserWindow): void {
    this.mainWindow = win;
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

    let session = this.sessions.get(sessionId);
    if (!session) {
      session = this.createSession(sessionId, cwd);
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
      // Evict the session entry after a grace period so the map doesn't grow unboundedly.
      setTimeout(() => {
        this.sessions.delete(sessionId);
        this.usageAccumulator.forget(sessionId);
      }, 30_000).unref();
    } else {
      applyHookEvent(session, event);
    }

    agentNotifier.notifyOnTransition(session, prevAmbient);

    // Event-driven supervisor wake: when this agent just entered a real decision
    // point (approval or question), nudge any supervisor so it surfaces it now
    // rather than on its next poll. No-op when no supervisor is running.
    const isBlocked = (s: SessionAmbientState) => s === 'waiting_approval' || s === 'waiting_input';
    if (isBlocked(session.ambientState) && !isBlocked(prevAmbient)) {
      supervisorNudge.onBlock(session, session.pendingApproval ? 'approval' : 'question', this.supervisorSessionIds());
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
  applyManagedMode(sessionId: string, mode: string): void {
    const session = this.sessions.get(sessionId);
    if (!session) return;
    let next: SessionAmbientState;
    switch (mode) {
      case 'responding': next = 'streaming'; break;
      case 'approval':   next = 'waiting_approval'; break;
      case 'question':   next = 'waiting_input'; break;
      case 'input':      next = 'idle'; break;
      default: return; // 'unknown' / 'stopped' — leave the current state as-is
    }
    const prevAmbient = session.ambientState;
    session.ambientState = next;
    session.lastActivity = Date.now();
    if (next !== prevAmbient) {
      agentNotifier.notifyOnTransition(session, prevAmbient);
      const isBlocked = (s: SessionAmbientState) => s === 'waiting_approval' || s === 'waiting_input';
      if (isBlocked(next) && !isBlocked(prevAmbient)) {
        supervisorNudge.onBlock(session, next === 'waiting_approval' ? 'approval' : 'question', this.supervisorSessionIds());
      }
    }
    this.pushUpdate(session);
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
    applyConversationItems(session, delta.items, (s, model, usage, key) =>
      this.usageAccumulator.applyUsage(s, model, usage, key));
    session.lastActivity = Date.now();
    this.mergeWatcherData(session);
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
      const snap = await res.json() as { seq: number; items: ConversationDeltaWire['items'] };
      const session = this.sessions.get(sessionId);
      if (!session) return;
      session.conversation = [];
      session.totalToolCalls = 0;
      applyConversationItems(session, snap.items ?? [], (s, model, usage, key) =>
        this.usageAccumulator.applyUsage(s, model, usage, key));
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
    session.subagents = session.subagents.filter(s => !workflowIds.has(stripPrefix(s.id)));
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

  // ── Queries ──

  getSnapshot(sessionId: string): ClaudeSessionSnapshot | null {
    const session = this.sessions.get(sessionId);
    if (!session) return null;
    return { ...session };
  }

  getAllSnapshots(): ClaudeSessionSnapshot[] {
    return Array.from(this.sessions.values()).map(s => ({ ...s }));
  }

  // ── Internals ──

  private createSession(sessionId: string, cwd: string): ClaudeSessionState {
    const session: ClaudeSessionState = {
      sessionId,
      cwd,
      ptyId: sessionId, // legacy field — renderer keys by this; we make it == sessionId
      transcriptPath: '',
      status: 'active',
      conversation: [],
      activeToolCalls: [],
      completedToolCalls: [],
      fileChanges: [],
      pendingApproval: null,
      pendingQuestions: null,
      subagents: [],
      workflows: [],
      ambientState: 'idle',
      startedAt: Date.now(),
      lastActivity: Date.now(),
      totalToolCalls: 0,
      peakContext: 0,
      usage: null,
    };
    // Apply any pre-registered spawn metadata (label, parentSessionId) so the
    // snapshot is enriched before the first push to the renderer.
    const meta = this.spawnMeta.get(sessionId);
    if (meta) {
      session.label = meta.label;
      session.parentSessionId = meta.parentSessionId;
      session.isSupervisor = meta.isSupervisor;
      session.provider = meta.provider;
      session.settings = meta.settings;
      this.spawnMeta.delete(sessionId);
    }
    this.sessions.set(sessionId, session);
    return session;
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

  private pushUpdate(session: ClaudeSessionState): void {
    if (!COALESCE_SNAPSHOT_UPDATES) {
      // Original immediate-send path (byte-for-byte identical behaviour).
      publishSnapshot({ ...session });
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
    // Mirror onto the hub bus for the web build (no-op when remote sharing is
    // off). Guard the object spread so the allocation is skipped when the hub
    // won't use it (publishSnapshot is a no-op when sharing is disabled).
    publishSnapshot({ ...session });
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
