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
        break;

      case 'UserPromptSubmit':
        session.ambientState = 'thinking';
        // User message comes from JSONL transcript
        break;

      case 'PreToolUse': {
        // Track active tool calls for live UI display
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
          completed.completedAt = Date.now();
          session.activeToolCalls = session.activeToolCalls.filter(t => t.id !== event.tool_use_id);
        }
        // Conversation + tool details come from JSONL transcript
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
        this.hookStateTimestamp.set(sessionId, Date.now());
        // Clear active tool calls
        session.activeToolCalls = [];
        session.completedToolCalls = [];
        // Conversation is now driven by JSONL transcript (refreshed above)
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
      // Remove Claude Code status spinners and hook messages
      .replace(/^.*[✱⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏].*$/gm, '')
      .replace(/^.*running \w+ hook.*$/gmi, '')
      .replace(/^.*Cultivating.*$/gm, '')
      .replace(/^.*Thinking.*$/gm, '')
      // Remove Claude Code tool call display formatting from terminal
      .replace(/^\s*(Read|Edit|Write|Bash|Grep|Glob|Search|MultiEdit|Agent|TodoRead|TodoWrite)\(.*\).*$/gm, '')
      .replace(/^\s*[⎿┃│]\s+.*$/gm, '')  // indented result lines with box-drawing chars
      .replace(/^\s*⎿\s.*$/gm, '')
      // Remove horizontal rules (lines of ─, ━, —, - only)
      .replace(/^[\s─━—\-]{3,}$/gm, '')
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

    // Filter out noise: prompt markers, cost lines, token info, status bars, spinners, tool calls
    return response
      .replace(/^[●◆❯>]\s*/gm, '')
      .replace(/^.*\d+\.\d+[kK]? tokens.*$/gm, '')
      .replace(/^.*\$\d+\.\d+.*$/gm, '')
      .replace(/^.*\? for shortcuts.*$/gm, '')
      .replace(/^.*[✱⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏].*$/gm, '')
      .replace(/^.*running \w+ hook.*$/gmi, '')
      // Remove tool call display lines
      .replace(/^\s*(Read|Edit|Write|Bash|Grep|Glob|Search|MultiEdit|Agent|TodoRead|TodoWrite)\(.*\).*$/gm, '')
      .replace(/^\s*[⎿┃│]\s+.*$/gm, '')
      .replace(/^.*Cultivating.*$/gm, '')
      .replace(/^[\s─━—\-]{3,}$/gm, '')
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
      session.lastTranscriptLine = lines.length;

      for (const line of newLines) {
        try {
          const entry = JSON.parse(line);
          this.processTranscriptEntry(session, entry);
        } catch {}
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
      const blocks = Array.isArray(msg.content) ? msg.content : [];
      const textParts: string[] = [];
      const toolCalls: ToolCall[] = [];

      for (const block of blocks) {
        if (block.type === 'text' && block.text) {
          textParts.push(block.text);
        } else if (block.type === 'tool_use') {
          toolCalls.push({
            id: block.id ?? `tc-${Date.now()}`,
            name: block.name ?? 'unknown',
            input: block.input ?? {},
            status: 'complete',
            startedAt: Date.now(),
            completedAt: Date.now(),
          });
          session.totalToolCalls++;
        }
      }

      const text = textParts.join('\n').trim();
      if (text && !this.isDuplicateMessage(session, 'assistant', text)) {
        session.conversation.push({
          role: 'assistant',
          content: text,
          timestamp: Date.now(),
          toolCalls: toolCalls.length > 0 ? toolCalls : undefined,
        });
      } else if (toolCalls.length > 0) {
        const lastAssistant = [...session.conversation].reverse().find(t => t.role === 'assistant');
        if (lastAssistant) {
          lastAssistant.toolCalls = [...(lastAssistant.toolCalls ?? []), ...toolCalls];
        } else {
          session.conversation.push({
            role: 'assistant',
            content: '',
            timestamp: Date.now(),
            toolCalls,
          });
        }
      }
    }
    // 'result' entries are handled via tool_result blocks in user entries above
  }

  private pushUpdate(session: ClaudeSessionState): void {
    if (!this.mainWindow || this.mainWindow.isDestroyed()) return;
    if (!session.ptyId) return;
    this.mainWindow.webContents.send('claude-session:update', session.ptyId, { ...session });
  }
}

export const claudeSessionStore = new ClaudeSessionStore();
