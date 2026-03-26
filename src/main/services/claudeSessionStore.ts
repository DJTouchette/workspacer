import { BrowserWindow } from 'electron';
import type { SessionAmbientState } from './headlessTerminalManager';

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
}

// Serialisable snapshot sent over IPC
export type ClaudeSessionSnapshot = Omit<ClaudeSessionState, never>;

// ── Store ──

class ClaudeSessionStore {
  private sessions = new Map<string, ClaudeSessionState>();
  private mainWindow: BrowserWindow | null = null;
  // Map PTY id → claude session id for routing terminal data
  private ptyToSession = new Map<string, string>();

  setMainWindow(win: BrowserWindow): void {
    this.mainWindow = win;
  }

  /** Register a PTY as belonging to a Claude session */
  bindPty(ptyId: string, claudeSessionId: string): void {
    this.ptyToSession.set(ptyId, claudeSessionId);
  }

  getSessionIdForPty(ptyId: string): string | undefined {
    return this.ptyToSession.get(ptyId);
  }

  // ── Hook event handler ──

  handleHookEvent(event: any): void {
    const hookName: string = event.hook_event_name ?? event.type ?? '';
    const sessionId: string = event.session_id ?? '';
    const cwd: string = event.cwd ?? '';

    let session = this.sessions.get(sessionId);

    if (!session && hookName === 'SessionStart') {
      session = this.createSession(sessionId, cwd);
    }
    if (!session) {
      // Try to find session by cwd (fallback routing)
      for (const s of this.sessions.values()) {
        if (s.cwd === cwd && s.status !== 'ended') {
          session = s;
          break;
        }
      }
    }
    if (!session) return;

    switch (hookName) {
      case 'SessionStart':
        session.status = 'active';
        break;

      case 'UserPromptSubmit':
        session.conversation.push({
          role: 'user',
          content: event.prompt ?? event.user_prompt ?? '',
          timestamp: Date.now(),
        });
        session.ambientState = 'thinking';
        break;

      case 'PreToolUse': {
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
        const completed = session.activeToolCalls.find(t => t.id === event.tool_use_id);
        if (completed) {
          completed.status = 'complete';
          completed.response = event.tool_response;
          completed.completedAt = Date.now();
          session.activeToolCalls = session.activeToolCalls.filter(t => t.id !== event.tool_use_id);
          session.completedToolCalls.push(completed);
        }
        session.totalToolCalls++;
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
        // Push as an assistant message
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

  updateAmbientState(sessionId: string, state: SessionAmbientState): void {
    const session = this.sessions.get(sessionId);
    if (!session) return;
    // Don't override hook-driven states
    if (session.pendingApproval && state !== 'waiting_approval') return;
    session.ambientState = state;
    session.lastActivity = Date.now();
    this.pushUpdate(session);
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

  private createSession(sessionId: string, cwd: string): ClaudeSessionState {
    const session: ClaudeSessionState = {
      sessionId,
      cwd,
      ptyId: '',
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
    };
    this.sessions.set(sessionId, session);
    return session;
  }

  private pushUpdate(session: ClaudeSessionState): void {
    if (!this.mainWindow || this.mainWindow.isDestroyed()) return;
    this.mainWindow.webContents.send('claude-session:update', session.ptyId, { ...session });
  }
}

export const claudeSessionStore = new ClaudeSessionStore();
