import * as path from 'path';
import { BrowserWindow } from 'electron';

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
    }

    // Refresh conversation from JSONL transcript on every hook event
    if (session.transcriptPath) {
      this.refreshFromTranscript(session);
    }

    switch (hookName) {
      case 'SessionStart':
        session.status = 'active';
        session.ambientState = 'idle';
        break;

      case 'UserPromptSubmit':
        session.ambientState = 'streaming';
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

        // A new tool call invalidates any stale approval card from a prior
        // tool — the daemon gateway only parks one decision at a time.
        session.pendingApproval = null;

        // AskUserQuestion: surface the question payload as a pending picker.
        // Also defensively clear any stale approval card — these are mutually
        // exclusive: a picker means claude is asking the user, not asking for
        // tool permission.
        if (tc.name === 'AskUserQuestion' && Array.isArray(tc.input?.questions)) {
          session.pendingQuestions = tc.input.questions;
          session.pendingApproval = null;
          session.ambientState = 'waiting_input';
        }

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
        // Any completed tool clears any leftover approval card — the daemon
        // gateway is single-shot, so by the time PostToolUse fires, whatever
        // decision was pending is either resolved or no longer relevant.
        session.pendingApproval = null;
        const completed = session.activeToolCalls.find(t => t.id === event.tool_use_id);
        if (completed) {
          completed.status = 'complete';
          completed.completedAt = Date.now();
          session.activeToolCalls = session.activeToolCalls.filter(t => t.id !== event.tool_use_id);
          session.completedToolCalls.push(completed);
          if (completed.name === 'AskUserQuestion') {
            session.pendingQuestions = null;
          }
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
        session.pendingQuestions = null;
        // Clear tool calls — they're already shown inline in conversation via transcript
        session.activeToolCalls = [];
        session.completedToolCalls = [];
        session.subagents = session.subagents.filter(s => s.status === 'running');
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
    this.mainWindow.webContents.send('claude-session:update', session.sessionId, { ...session });
  }
}

export const claudeSessionStore = new ClaudeSessionStore();
