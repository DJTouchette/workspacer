import * as path from 'path';
import { BrowserWindow } from 'electron';
import { agentNotifier } from './agentNotifier';
import {
  workflowWatcher,
  type WorkflowRunInfo,
  type WorkflowWatcherUpdate,
} from './workflowWatcher';
import { publishWorkflowRuns, forgetSession as forgetTelemetry } from './hubTelemetry';
import { applyHookEvent, applyStopEvent, applySessionEndEvent } from './sessionStore/hookEventRouter';
import { refreshFromTranscript } from './sessionStore/transcriptParser';
import { SessionUsageAccumulator } from './sessionStore/usageAccumulator';
import { writeHistory } from './sessionStore/analyticsWriter';

export type { WorkflowRunInfo, WorkflowAgentInfo, WorkflowPhaseInfo } from './workflowWatcher';

/**
 * Ambient session activity, mostly driven by hook events now that claudemon
 * owns the hook ingestion. Kept compatible with the renderer's view-side type
 * (`src/renderer/src/types/claudeSession.ts`).
 */
export type SessionAmbientState = 'idle' | 'thinking' | 'streaming' | 'waiting_input' | 'waiting_approval';

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
  model?: string;
  tokens?: number;
  toolCalls?: number;
  lastToolName?: string;
  lastToolSummary?: string;
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
  lastTranscriptLine: number; // track how far we've read in JSONL
  usage: import('./modelUsage').SessionUsage | null; // token / cost / context, parsed from transcript
  // Adoption metadata — set before first hook arrives so adopted cards can be
  // named and nested under the agent that spawned them.
  label?: string;
  parentSessionId?: string;
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
  private spawnMeta = new Map<string, { label?: string; parentSessionId?: string }>();

  /** Record name/parent for a session about to be spawned, keyed by its pinned
   *  id. Consumed when the session first registers (see createSession), so an
   *  adopted card can be named and nested under its spawner. */
  setSpawnMeta(sessionId: string, meta: { label?: string; parentSessionId?: string }): void {
    if (!sessionId) return;
    this.spawnMeta.set(sessionId, meta);
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

    // Refresh conversation from JSONL transcript on every hook event
    if (session.transcriptPath) {
      refreshFromTranscript(session, (s, model, usage, key) => this.usageAccumulator.applyUsage(s, model, usage, key));
    }

    // Snapshot the ambient state so the notifier can detect transitions
    // (needs-you / done) after the switch below applies the new state.
    const prevAmbient = session.ambientState;

    // Handle Stop and SessionEnd here because they need store-level side-effects.
    if (hookName === 'Stop') {
      applyStopEvent(session);
      // Delayed re-read: final assistant message may still be flushing
      if (session.transcriptPath) {
        setTimeout(() => {
          refreshFromTranscript(session!, (s, model, usage, key) => this.usageAccumulator.applyUsage(s, model, usage, key));
          this.pushUpdate(session!);
          writeHistory(session!, 'active');
        }, 500);
      } else {
        writeHistory(session, 'active');
      }
    } else if (hookName === 'SessionEnd') {
      applySessionEndEvent(session);
      workflowWatcher.detach(sessionId);
      forgetTelemetry(sessionId);
      writeHistory(session, 'ended');
    } else {
      applyHookEvent(session, event);
    }

    agentNotifier.notifyOnTransition(session, prevAmbient);
    this.mergeWatcherData(session);
    this.pushUpdate(session);
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
      if (activity.model) sub.model = activity.model;
      if (activity.tokens !== undefined) sub.tokens = activity.tokens;
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
      lastTranscriptLine: 0,
      usage: null,
    };
    // Apply any pre-registered spawn metadata (label, parentSessionId) so the
    // snapshot is enriched before the first push to the renderer.
    const meta = this.spawnMeta.get(sessionId);
    if (meta) {
      session.label = meta.label;
      session.parentSessionId = meta.parentSessionId;
      this.spawnMeta.delete(sessionId);
    }
    this.sessions.set(sessionId, session);
    return session;
  }

  private pushUpdate(session: ClaudeSessionState): void {
    if (!this.mainWindow || this.mainWindow.isDestroyed()) return;
    this.mainWindow.webContents.send('claude-session:update', session.sessionId, { ...session });
  }
}

export const claudeSessionStore = new ClaudeSessionStore();
