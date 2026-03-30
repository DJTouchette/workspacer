import * as path from 'path';
import { BrowserWindow } from 'electron';
import type { SessionAmbientState } from './headlessTerminalManager';

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

export interface SubagentInfo {
  id: string;
  type: string;
  status: 'running' | 'complete';
  startedAt: number;
  completedAt?: number;
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
  subagents: SubagentInfo[];

  ambientState: SessionAmbientState;
  lastActivity: number;
  totalToolCalls: number;
  lastTranscriptLine: number; // track how far we've read in JSONL
}

// Serialisable snapshot sent over IPC
export type ClaudeSessionSnapshot = Omit<ClaudeSessionState, never>;

// ── Store ──

class ClaudeSessionStore {
  private sessions = new Map<string, ClaudeSessionState>();
  private mainWindow: BrowserWindow | null = null;

  // Bidirectional maps for PTY ↔ Claude session binding
  private ptyToSession = new Map<string, string>();  // ptyId → sessionId
  private sessionToPty = new Map<string, string>();   // sessionId → ptyId

  // Queue of unbound PTYs waiting for a SessionStart hook, keyed by resolved cwd.
  // Multiple PTYs with the same cwd are queued in order (FIFO).
  private unboundPtys = new Map<string, string[]>();  // cwd → [ptyId, ...]

  // Timestamp of last hook-driven state change — poller defers briefly after hooks fire
  private hookStateTimestamp = new Map<string, number>(); // sessionId → timestamp

  setMainWindow(win: BrowserWindow): void {
    this.mainWindow = win;
  }

  /**
   * Register a PTY as a pending Claude session.
   * Called when a Claude terminal is spawned, before the SessionStart hook fires.
   * The cwd is used to match the incoming SessionStart event to this PTY.
   */
  registerPendingPty(ptyId: string, cwd: string): void {
    const key = normalizeCwd(cwd);
    console.log(`[SessionStore] registerPendingPty ptyId=${ptyId} cwd="${cwd}" key="${key}"`);
    const queue = this.unboundPtys.get(key) ?? [];
    queue.push(ptyId);
    this.unboundPtys.set(key, queue);
  }

  /** Remove a PTY from pending + bound maps (called on close) */
  unregisterPty(ptyId: string): void {
    // Remove from unbound queue
    for (const [cwd, queue] of this.unboundPtys) {
      const idx = queue.indexOf(ptyId);
      if (idx >= 0) {
        queue.splice(idx, 1);
        if (queue.length === 0) this.unboundPtys.delete(cwd);
        break;
      }
    }
    // Remove from bound maps
    const sid = this.ptyToSession.get(ptyId);
    if (sid) {
      this.ptyToSession.delete(ptyId);
      this.sessionToPty.delete(sid);
    }
  }

  getSessionIdForPty(ptyId: string): string | undefined {
    return this.ptyToSession.get(ptyId);
  }

  // ── Hook event handler ──

  handleHookEvent(event: any): void {
    const hookName: string = event.hook_event_name ?? event.type ?? '';
    const sessionId: string = event.session_id ?? '';
    const cwd: string = normalizeCwd(event.cwd ?? '');

    let session = this.sessions.get(sessionId);

    if (!session && hookName === 'SessionStart') {
      // Bind to an unbound PTY by matching cwd (FIFO)
      console.log(`[SessionStore] SessionStart: looking for pending PTY with cwd="${cwd}"`);
      console.log(`[SessionStore]   pending cwds: [${[...this.unboundPtys.keys()].map(k => `"${k}"`).join(', ')}]`);
      const ptyId = this.claimPendingPty(cwd);
      session = this.createSession(sessionId, cwd, ptyId);

      if (ptyId) {
        console.log(`[SessionStore]   ✓ bound session=${sessionId} to pty=${ptyId}`);
        this.ptyToSession.set(ptyId, sessionId);
        this.sessionToPty.set(sessionId, ptyId);
      } else {
        console.warn(`[SessionStore]   ✗ no pending PTY matched cwd="${cwd}"`);
      }
    }

    if (!session) {
      // Try to find by session_id already bound
      // (session_id may differ from initial if Claude resumes — match by cwd as fallback)
      for (const s of this.sessions.values()) {
        if (s.cwd === cwd && s.status !== 'ended') {
          session = s;
          break;
        }
      }
    }
    if (!session) {
      console.warn(`[SessionStore] no session for hook=${hookName} session_id=${sessionId} cwd="${cwd}"`);
      return;
    }

    // Capture transcript path from first event that has it
    if (event.transcript_path && !session.transcriptPath) {
      session.transcriptPath = event.transcript_path;
      console.log(`[SessionStore] transcript: ${session.transcriptPath}`);
    }

    // Refresh conversation from JSONL transcript on every hook event
    if (session.transcriptPath) {
      this.refreshFromTranscript(session);
    }

    switch (hookName) {
      case 'SessionStart':
        session.status = 'active';
        // Suppress poller during startup — CLI prints welcome banner before
        // showing the > prompt, which causes false thinking/streaming flickers
        this.hookStateTimestamp.set(sessionId, Date.now() + 3000);
        break;

      case 'UserPromptSubmit':
        session.ambientState = 'streaming';
        // Prevent poller from overriding until Stop
        this.hookStateTimestamp.set(sessionId, Date.now() + 600000);
        break;

      case 'PreToolUse': {
        session.ambientState = 'streaming';
        const tc: ToolCall = {
          id: event.tool_use_id ?? `tc-${Date.now()}`,
          name: event.tool_name ?? 'unknown',
          input: event.tool_input ?? {},
          status: 'running',
          startedAt: Date.now(),
        };
        session.activeToolCalls.push(tc);

        if (['Edit', 'MultiEdit', 'Write'].includes(tc.name)) {
          session.fileChanges.push({
            path: tc.input?.file_path ?? 'unknown',
            toolName: tc.name,
            input: tc.input,
            timestamp: Date.now(),
          });
        }
        break;
      }

      case 'PostToolUse': {
        session.ambientState = 'streaming';
        const completed = session.activeToolCalls.find(t => t.id === event.tool_use_id);
        if (completed) {
          completed.status = 'complete';
          completed.completedAt = Date.now();
          session.activeToolCalls = session.activeToolCalls.filter(t => t.id !== event.tool_use_id);
          session.completedToolCalls.push(completed);
        }
        break;
      }

      case 'PostToolUseFailure': {
        const failed = session.activeToolCalls.find(t => t.id === event.tool_use_id);
        if (failed) {
          failed.status = 'failed';
          failed.completedAt = Date.now();
          session.activeToolCalls = session.activeToolCalls.filter(t => t.id !== event.tool_use_id);
          session.completedToolCalls.push(failed);
        }
        break;
      }

      case 'PermissionRequest':
        session.pendingApproval = {
          toolName: event.tool_name ?? '',
          toolInput: event.tool_input ?? {},
          suggestions: event.permission_suggestions,
          timestamp: Date.now(),
        };
        session.ambientState = 'waiting_approval';
        break;

      case 'Stop':
        session.ambientState = 'idle';
        session.pendingApproval = null;
        // Suppress poller briefly — terminal still has recent activity from
        // the response that just finished, which causes false thinking flicker
        this.hookStateTimestamp.set(sessionId, Date.now() + 2000);
        // Clear active tool calls (completed ones persist as history until next prompt)
        session.activeToolCalls = [];
        // Delayed re-read: final assistant message may still be flushing
        if (session.transcriptPath) {
          setTimeout(() => { this.refreshFromTranscript(session); this.pushUpdate(session); }, 500);
        }
        break;

      case 'SubagentStart':
        session.subagents.push({
          id: event.agent_id ?? `sa-${Date.now()}`,
          type: event.agent_type ?? 'unknown',
          status: 'running',
          startedAt: Date.now(),
        });
        break;

      case 'SubagentStop': {
        const sub = session.subagents.find(s => s.id === event.agent_id);
        if (sub) {
          sub.status = 'complete';
          sub.completedAt = Date.now();
        }
        break;
      }

      case 'Notification':
        session.conversation.push({
          role: 'assistant',
          content: event.message ?? event.notification ?? '[notification]',
          timestamp: Date.now(),
        });
        break;

      case 'SessionEnd':
        session.status = 'ended';
        session.ambientState = 'idle';
        break;
    }

    this.pushUpdate(session);
  }

  // ── Ambient state (from headless terminal polling) ──

  /** Update ambient state by PTY id (doesn't require session binding) */
  updateAmbientStateByPty(ptyId: string, state: SessionAmbientState): void {
    const sessionId = this.ptyToSession.get(ptyId);
    if (!sessionId) return;
    const session = this.sessions.get(sessionId);
    if (!session) return;
    // Don't override hook-driven states
    if (session.pendingApproval && state !== 'waiting_approval') return;
    // Defer to hook-driven state for 2s after a hook fires (e.g. Stop sets idle,
    // don't let poller flip it back to streaming while terminal settles)
    const hookTs = this.hookStateTimestamp.get(sessionId);
    if (hookTs && Date.now() - hookTs < 2000) return;
    // Skip if state hasn't changed (reduces IPC chatter)
    if (session.ambientState === state) return;

    const prevState = session.ambientState;
    session.ambientState = state;
    session.lastActivity = Date.now();

    console.log(`[SessionStore] state transition: ${prevState} → ${state} (pty=${ptyId.slice(0, 8)})`);

    // Extract conversation from terminal on state transitions
    this.extractConversationFromTerminal(ptyId, session, prevState, state);

    this.pushUpdate(session);
  }

  /**
   * When Claude transitions from active (thinking/streaming) to idle,
   * read the new terminal output and add it as conversation turns.
   */
  private extractConversationFromTerminal(
    ptyId: string,
    session: ClaudeSessionState,
    prevState: SessionAmbientState,
    newState: SessionAmbientState,
  ): void {
    const wasActive = prevState === 'thinking' || prevState === 'streaming';
    const nowIdle = newState === 'idle' || newState === 'waiting_input';

    if (wasActive && nowIdle) {
      // Conversation data comes exclusively from the JSONL transcript now —
      // terminal buffer extraction was the old approach and conflicts with
      // transcript entries via isDuplicateMessage's startsWith logic.
      if (session.transcriptPath) {
        this.refreshFromTranscript(session);
      }
    }
  }

  /** Check if a message was already added to avoid duplicates */
  private isDuplicateMessage(session: ClaudeSessionState, role: string, content: string): boolean {
    if (!content) return false;
    const recent = session.conversation.slice(-5);
    return recent.some(
      (t) => t.role === role && t.content && t.content === content,
    );
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

  /** Find the Claude session bound to a given PTY id */
  getSnapshotByPty(ptyId: string): ClaudeSessionSnapshot | null {
    const sid = this.ptyToSession.get(ptyId);
    if (!sid) return null;
    return this.getSnapshot(sid);
  }

  // ── Internals ──

  /** Claim the oldest unbound PTY for a given cwd (FIFO) */
  private claimPendingPty(cwd: string): string {
    const queue = this.unboundPtys.get(cwd);
    if (!queue || queue.length === 0) return '';
    const ptyId = queue.shift()!;
    if (queue.length === 0) this.unboundPtys.delete(cwd);
    return ptyId;
  }

  private createSession(sessionId: string, cwd: string, ptyId: string): ClaudeSessionState {
    const session: ClaudeSessionState = {
      sessionId,
      cwd,
      ptyId,
      transcriptPath: '',
      status: 'active',
      conversation: [],
      activeToolCalls: [],
      completedToolCalls: [],
      fileChanges: [],
      pendingApproval: null,
      subagents: [],
      ambientState: 'idle',
      lastActivity: Date.now(),
      totalToolCalls: 0,
      lastTranscriptLine: 0,
    };
    this.sessions.set(sessionId, session);
    return session;
  }

  /** Read new lines from JSONL transcript and update conversation */
  private refreshFromTranscript(session: ClaudeSessionState): void {
    if (!session.transcriptPath) return;
    const fs = require('fs');
    try {
      if (!fs.existsSync(session.transcriptPath)) return;
      const content = fs.readFileSync(session.transcriptPath, 'utf-8');
      const lines = content.split('\n').filter((l: string) => l.trim());

      if (lines.length <= session.lastTranscriptLine) return;

      const newLines = lines.slice(session.lastTranscriptLine);

      let parsed = 0;
      for (const line of newLines) {
        try {
          const entry = JSON.parse(line);
          this.processTranscriptEntry(session, entry);
          parsed++;
        } catch {
          // Stop at first unparseable line — likely a partial write at EOF.
          // It will be re-read (now complete) on the next hook event.
          break;
        }
      }
      session.lastTranscriptLine += parsed;

      // Housekeeping: drop completedToolCalls already absorbed into conversation
      if (parsed > 0 && session.completedToolCalls.length > 0) {
        const convToolIds = new Set<string>();
        for (const turn of session.conversation) {
          if (turn.toolCalls) for (const tc of turn.toolCalls) convToolIds.add(tc.id);
        }
        session.completedToolCalls = session.completedToolCalls.filter(tc => !convToolIds.has(tc.id));
      }
    } catch (err) {
      console.error('[SessionStore] transcript read error:', err);
    }
  }

  private processTranscriptEntry(session: ClaudeSessionState, entry: any): void {
    const type = entry.type;
    const msg = entry.message;

    if (type === 'user' && msg) {
      const contentBlocks = Array.isArray(msg.content) ? msg.content : [];
      // Skip tool_result entries — they're API plumbing, not user messages
      const hasToolResult = contentBlocks.some((b: any) => b.type === 'tool_result');
      if (hasToolResult) {
        // Extract tool results and attach to the corresponding tool calls
        for (const block of contentBlocks) {
          if (block.type === 'tool_result' && block.tool_use_id) {
            const resultText = typeof block.content === 'string'
              ? block.content
              : Array.isArray(block.content)
                ? block.content.filter((b: any) => b.type === 'text').map((b: any) => b.text).join('\n')
                : '';
            // Find the matching tool call and set its response
            for (let i = session.conversation.length - 1; i >= 0; i--) {
              const tcs = session.conversation[i].toolCalls;
              if (!tcs) continue;
              const tc = tcs.find(t => t.id === block.tool_use_id);
              if (tc) {
                tc.response = resultText;
                break;
              }
            }
          }
        }
        return;
      }

      // Real user message
      const content = typeof msg.content === 'string'
        ? msg.content
        : contentBlocks.filter((b: any) => b.type === 'text').map((b: any) => b.text).join('\n');
      if (content && !this.isDuplicateMessage(session, 'user', content)) {
        session.conversation.push({ role: 'user', content, timestamp: Date.now() });
      }
    } else if (type === 'assistant' && msg) {
      // The JSONL transcript streams each content block as a separate entry.
      // Keep each block as its own conversation turn so text and tool calls
      // render interlaced in timeline order.
      const blocks = Array.isArray(msg.content) ? msg.content : [];

      for (const block of blocks) {
        if (block.type === 'thinking') continue;

        if (block.type === 'text' && block.text) {
          const text = block.text.trim();
          if (!text) continue;
          if (!this.isDuplicateMessage(session, 'assistant', text)) {
            session.conversation.push({
              role: 'assistant',
              content: text,
              timestamp: Date.now(),
            });
          }
        } else if (block.type === 'tool_use') {
          const tc: ToolCall = {
            id: block.id ?? `tc-${Date.now()}-${Math.random().toString(36).slice(2, 6)}`,
            name: block.name ?? 'unknown',
            input: block.input ?? {},
            status: 'complete',
            startedAt: Date.now(),
            completedAt: Date.now(),
          };
          session.totalToolCalls++;

          // Each tool call is its own turn — interlaced with text
          session.conversation.push({
            role: 'assistant',
            content: '',
            timestamp: Date.now(),
            toolCalls: [tc],
          });
        }
      }
    }
  }

  private pushUpdate(session: ClaudeSessionState): void {
    if (!this.mainWindow || this.mainWindow.isDestroyed()) return;
    if (!session.ptyId) return;
    this.mainWindow.webContents.send('claude-session:update', session.ptyId, { ...session });
  }
}

export const claudeSessionStore = new ClaudeSessionStore();
