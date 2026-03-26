import { BrowserWindow } from 'electron';
import type { SessionAmbientState } from './headlessTerminalManager';
import { getNewBufferContent, markBufferPosition, getFullBuffer } from './headlessTerminalManager';

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

  // Bidirectional maps for PTY ↔ Claude session binding
  private ptyToSession = new Map<string, string>();  // ptyId → sessionId
  private sessionToPty = new Map<string, string>();   // sessionId → ptyId

  // Queue of unbound PTYs waiting for a SessionStart hook, keyed by resolved cwd.
  // Multiple PTYs with the same cwd are queued in order (FIFO).
  private unboundPtys = new Map<string, string[]>();  // cwd → [ptyId, ...]

  setMainWindow(win: BrowserWindow): void {
    this.mainWindow = win;
  }

  /**
   * Register a PTY as a pending Claude session.
   * Called when a Claude terminal is spawned, before the SessionStart hook fires.
   * The cwd is used to match the incoming SessionStart event to this PTY.
   */
  registerPendingPty(ptyId: string, cwd: string): void {
    console.log(`[SessionStore] registerPendingPty ptyId=${ptyId} cwd="${cwd}"`);
    const queue = this.unboundPtys.get(cwd) ?? [];
    queue.push(ptyId);
    this.unboundPtys.set(cwd, queue);
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
    const cwd: string = event.cwd ?? '';

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
        // Mark terminal buffer position so we can extract the response later
        if (session.ptyId) markBufferPosition(session.ptyId);
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
        // Extract assistant response from terminal buffer
        if (session.ptyId) {
          const lines = getFullBuffer(session.ptyId);
          const response = this.extractLastResponse(lines);
          if (response && !this.isDuplicateMessage(session, 'assistant', response)) {
            console.log(`[SessionStore] extracted response: "${response.slice(0, 120)}"`);
            session.conversation.push({
              role: 'assistant',
              content: response,
              timestamp: Date.now(),
            });
          }
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
      // Claude just finished responding — grab terminal output
      const rawText = getNewBufferContent(ptyId);
      console.log(`[SessionStore] extraction: raw=${rawText.length} chars`);
      if (!rawText) return;

      // Clean up terminal artifacts
      const cleaned = this.cleanTerminalText(rawText);
      if (!cleaned) return;

      // Try to split into user prompt + assistant response
      const { userText, assistantText } = this.parseExchange(cleaned);

      if (userText && !this.isDuplicateMessage(session, 'user', userText)) {
        console.log(`[SessionStore] extracted user message: "${userText.slice(0, 80)}"`);
        session.conversation.push({
          role: 'user',
          content: userText,
          timestamp: Date.now() - 1000, // slightly before response
        });
      }

      if (assistantText && !this.isDuplicateMessage(session, 'assistant', assistantText)) {
        console.log(`[SessionStore] extracted assistant message: "${assistantText.slice(0, 80)}..."`);
        session.conversation.push({
          role: 'assistant',
          content: assistantText,
          timestamp: Date.now(),
        });
      }
    } else if (!wasActive && (newState === 'thinking' || newState === 'streaming')) {
      // Claude starting to think — mark buffer position so we capture from here
      markBufferPosition(ptyId);
    }
  }

  /** Remove terminal control artifacts from text */
  private cleanTerminalText(text: string): string {
    return text
      // Remove common Claude Code UI elements
      .replace(/^.*Claude Code v[\d.]+.*$/gm, '')
      .replace(/^.*Tips for getting started.*$/gm, '')
      .replace(/^.*Welcome back.*$/gm, '')
      .replace(/^.*Recent activity.*$/gm, '')
      .replace(/^.*No recent activity.*$/gm, '')
      .replace(/^.*Run \/init.*$/gm, '')
      .replace(/^.*\? for shortcuts.*$/gm, '')
      .replace(/^.*Opus.*context.*$/gm, '')
      .replace(/^.*Organization.*$/gm, '')
      .replace(/^.*Claude Max.*$/gm, '')
      .replace(/^.*@.*\.com.*$/gm, '')
      // Remove prompt markers (◆, >, ❯ at start of line)
      .replace(/^[◆❯>]\s*/gm, '')
      // Remove cost/token info lines
      .replace(/^.*\d+\.\d+[kK]? tokens.*$/gm, '')
      .replace(/^.*\$\d+\.\d+.*cost.*$/gm, '')
      // Collapse multiple blank lines
      .replace(/\n{3,}/g, '\n\n')
      .trim();
  }

  /** Try to split terminal output into user input + assistant response */
  private parseExchange(text: string): { userText: string; assistantText: string } {
    // Claude Code typically shows: [prompt marker] user text \n\n assistant response \n\n [prompt marker]
    // After cleaning, we have: user text \n\n assistant response
    // The first non-empty block is likely the user input, the rest is the response

    const blocks = text.split(/\n\n+/).filter(b => b.trim());

    if (blocks.length === 0) return { userText: '', assistantText: '' };
    if (blocks.length === 1) {
      // Single block — treat as assistant response (user input was likely on same line as prompt)
      return { userText: '', assistantText: blocks[0].trim() };
    }

    // First block = user input, rest = assistant response
    const userText = blocks[0].trim();
    const assistantText = blocks.slice(1).join('\n\n').trim();

    // If user text is very long, it's probably part of the response
    if (userText.length > 200) {
      return { userText: '', assistantText: text.trim() };
    }

    return { userText, assistantText };
  }

  /**
   * Parse the full terminal buffer and extract the last assistant response.
   * Claude Code terminal format:
   *   ◆ user prompt text
   *
   *   assistant response text
   *
   *   ◆ (next prompt)
   *
   * We find the last response block between prompt markers.
   */
  private extractLastResponse(lines: string[]): string {
    // Find prompt markers: lines starting with ◆, ❯, or > followed by space
    // Also detect the input prompt (line with just ◆ or > and maybe whitespace)
    const promptPattern = /^[◆❯>]\s*/;
    const promptIndices: number[] = [];

    for (let i = 0; i < lines.length; i++) {
      const trimmed = lines[i].trim();
      if (promptPattern.test(trimmed)) {
        promptIndices.push(i);
      }
    }

    if (promptIndices.length < 2) return '';

    // The last prompt marker is the current input prompt (empty)
    // The second-to-last is where the user typed their message
    // The response is between the user's prompt line and the current prompt
    const userPromptIdx = promptIndices[promptIndices.length - 2];
    const currentPromptIdx = promptIndices[promptIndices.length - 1];

    // User prompt is on the line with the marker
    // Response starts on the next line
    const responseLines: string[] = [];
    for (let i = userPromptIdx + 1; i < currentPromptIdx; i++) {
      responseLines.push(lines[i]);
    }

    // Trim empty lines from edges
    while (responseLines.length > 0 && responseLines[0].trim() === '') responseLines.shift();
    while (responseLines.length > 0 && responseLines[responseLines.length - 1].trim() === '') responseLines.pop();

    const response = responseLines.join('\n').trim();

    // Filter out noise: prompt markers, cost lines, token info, status bars
    return response
      .replace(/^[●◆❯>]\s*/gm, '')
      .replace(/^.*\d+\.\d+[kK]? tokens.*$/gm, '')
      .replace(/^.*\$\d+\.\d+.*$/gm, '')
      .replace(/^.*\? for shortcuts.*$/gm, '')
      .replace(/\n{3,}/g, '\n\n')
      .trim();
  }

  /** Check if a message was already added to avoid duplicates */
  private isDuplicateMessage(session: ClaudeSessionState, role: string, content: string): boolean {
    const recent = session.conversation.slice(-5);
    return recent.some(
      (t) => t.role === role && (t.content === content || content.startsWith(t.content) || t.content.startsWith(content)),
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
    if (!session.ptyId) return;
    this.mainWindow.webContents.send('claude-session:update', session.ptyId, { ...session });
  }
}

export const claudeSessionStore = new ClaudeSessionStore();
