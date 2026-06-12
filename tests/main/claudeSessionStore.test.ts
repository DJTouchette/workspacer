/**
 * Characterization tests for claudeSessionStore.
 *
 * Covers the current public API:
 *   setMainWindow(win)
 *   handleHookEvent(event)
 *   getSnapshot(sessionId)
 *   getAllSnapshots()
 *
 * All sidecar imports that touch OS notifications, analytics DB, hub WebSocket,
 * or the workflowWatcher poll loop are mocked so the tests run deterministically
 * in the Node vitest environment. Real fs is used for transcript tests (temp files).
 */

import { describe, it, expect, vi } from 'vitest';
import * as path from 'path';
import * as os from 'os';
import * as fsSync from 'fs';

// ── Mock sidecar modules before importing the store ──────────────────────────

// Electron — BrowserWindow + Notification used by agentNotifier
vi.mock('electron', () => ({
  BrowserWindow: vi.fn(),
  Notification: {
    isSupported: vi.fn(() => false),
  },
}));

// configService — used by agentNotifier.cfg() and claudeSessionStore.rememberModel()
vi.mock('../../src/main/services/configService', () => ({
  configService: {
    getConfig: vi.fn(() => ({
      notifications: { enabled: false },
      claude: { seenModels: [] },
    })),
    saveConfig: vi.fn(),
  },
}));

// workflowWatcher — has a setInterval poll loop; we stub the three used methods
vi.mock('../../src/main/services/workflowWatcher', () => ({
  workflowWatcher: {
    attach: vi.fn(),
    poke: vi.fn(),
    detach: vi.fn(),
  },
}));

// hubTelemetry — publishWorkflowRuns + publishSnapshot + forgetSession talk to
// a WebSocket hub
vi.mock('../../src/main/services/hubTelemetry', () => ({
  publishWorkflowRuns: vi.fn(),
  publishSnapshot: vi.fn(),
  forgetSession: vi.fn(),
}));

// sessionHistory — writes to SQLite via `database`
vi.mock('../../src/main/services/sessionHistory', () => ({
  sessionHistory: { record: vi.fn() },
}));

// hubClient — WebSocket; only publishToHub is called transitively
vi.mock('../../src/main/services/hubClient', () => ({
  publishToHub: vi.fn(),
  isHubConnected: vi.fn(() => false),
  startHubClient: vi.fn(),
  stopHubClient: vi.fn(),
  setHubMainWindow: vi.fn(),
  registerCapability: vi.fn(),
}));

// hubDaemon — pulls HUB_BUS_URL at module load
vi.mock('../../src/main/services/hubDaemon', () => ({
  HUB_BUS_URL: 'ws://localhost:3457/bus',
  getHubToken: vi.fn(() => null),
}));

// ── Import the module under test (after mocks are in place) ─────────────────

const { claudeSessionStore: store } = await import('../../src/main/services/claudeSessionStore');

// ── Event builder helpers ────────────────────────────────────────────────────

function sid(n: number | string) {
  return `session-${n}`;
}

function mkEvent(hookName: string, sessionId: string, extra: Record<string, unknown> = {}) {
  return { hook_event_name: hookName, session_id: sessionId, cwd: '/test/project', ...extra };
}

// ── Tests ────────────────────────────────────────────────────────────────────

describe('ClaudeSessionStore — handleHookEvent / getSnapshot / getAllSnapshots', () => {

  // Each describe block uses a unique session-id namespace so the singleton
  // store's accumulated state doesn't bleed between test groups.

  // ── Session lifecycle ──────────────────────────────────────────────────────

  describe('session creation on first event', () => {
    it('creates a session on the very first event even without SessionStart', () => {
      const id = sid('create-1');
      store.handleHookEvent(mkEvent('UserPromptSubmit', id));
      const snap = store.getSnapshot(id);
      expect(snap).not.toBeNull();
      expect(snap!.sessionId).toBe(id);
    });

    it('returns null for an unknown session', () => {
      expect(store.getSnapshot('does-not-exist')).toBeNull();
    });

    it('initialises a fresh session with sensible defaults', () => {
      const id = sid('defaults-1');
      store.handleHookEvent(mkEvent('SessionStart', id, { cwd: '/my/project' }));
      const snap = store.getSnapshot(id)!;
      expect(snap.status).toBe('active');
      expect(snap.conversation).toEqual([]);
      expect(snap.activeToolCalls).toEqual([]);
      expect(snap.completedToolCalls).toEqual([]);
      expect(snap.fileChanges).toEqual([]);
      expect(snap.pendingApproval).toBeNull();
      expect(snap.pendingQuestions).toBeNull();
      expect(snap.subagents).toEqual([]);
      expect(snap.workflows).toEqual([]);
      expect(snap.totalToolCalls).toBe(0);
      expect(snap.peakContext).toBe(0);
      expect(snap.usage).toBeNull();
      // ptyId is set to sessionId (legacy renderer key)
      expect(snap.ptyId).toBe(id);
    });

    it('normalises cwd using path.resolve', () => {
      const id = sid('cwd-1');
      store.handleHookEvent(mkEvent('SessionStart', id, { cwd: '/my/../my/project' }));
      const snap = store.getSnapshot(id)!;
      expect(snap.cwd).toBe(path.resolve('/my/../my/project'));
    });

    it('ignores events without a session_id', () => {
      const before = store.getAllSnapshots().length;
      store.handleHookEvent({ hook_event_name: 'SessionStart' }); // no session_id
      expect(store.getAllSnapshots().length).toBe(before);
    });
  });

  // ── SessionStart ──────────────────────────────────────────────────────────

  describe('SessionStart', () => {
    it('sets status active and ambientState idle', () => {
      const id = sid('start-1');
      store.handleHookEvent(mkEvent('SessionStart', id));
      const snap = store.getSnapshot(id)!;
      expect(snap.status).toBe('active');
      expect(snap.ambientState).toBe('idle');
    });

    it('captures transcript_path from the first event that has it', () => {
      const id = sid('transcript-1');
      store.handleHookEvent(
        mkEvent('SessionStart', id, { transcript_path: '/tmp/session.jsonl' }),
      );
      const snap = store.getSnapshot(id)!;
      expect(snap.transcriptPath).toBe('/tmp/session.jsonl');
    });

    it('does not overwrite an already-set transcript_path', () => {
      const id = sid('transcript-2');
      store.handleHookEvent(
        mkEvent('SessionStart', id, { transcript_path: '/tmp/first.jsonl' }),
      );
      store.handleHookEvent(
        mkEvent('UserPromptSubmit', id, { transcript_path: '/tmp/second.jsonl' }),
      );
      // Still the first one
      expect(store.getSnapshot(id)!.transcriptPath).toBe('/tmp/first.jsonl');
    });
  });

  // ── UserPromptSubmit ──────────────────────────────────────────────────────

  describe('UserPromptSubmit', () => {
    it('sets ambientState to streaming', () => {
      const id = sid('prompt-1');
      store.handleHookEvent(mkEvent('SessionStart', id));
      store.handleHookEvent(mkEvent('UserPromptSubmit', id, { prompt: 'hello' }));
      expect(store.getSnapshot(id)!.ambientState).toBe('streaming');
    });
  });

  // ── PreToolUse ────────────────────────────────────────────────────────────

  describe('PreToolUse', () => {
    it('adds a running tool call to activeToolCalls', () => {
      const id = sid('pretool-1');
      store.handleHookEvent(mkEvent('SessionStart', id));
      store.handleHookEvent(
        mkEvent('PreToolUse', id, {
          tool_name: 'Read',
          tool_input: { file_path: '/src/foo.ts' },
          tool_use_id: 'tc-1',
        }),
      );
      const snap = store.getSnapshot(id)!;
      expect(snap.activeToolCalls).toHaveLength(1);
      expect(snap.activeToolCalls[0].name).toBe('Read');
      expect(snap.activeToolCalls[0].status).toBe('running');
      expect(snap.activeToolCalls[0].id).toBe('tc-1');
      expect(snap.ambientState).toBe('streaming');
    });

    it('clears any stale pendingApproval when a new tool call starts', () => {
      const id = sid('pretool-clear-approval-1');
      store.handleHookEvent(mkEvent('SessionStart', id));
      store.handleHookEvent(
        mkEvent('PermissionRequest', id, {
          tool_name: 'Bash',
          tool_input: { command: 'ls' },
        }),
      );
      expect(store.getSnapshot(id)!.pendingApproval).not.toBeNull();

      store.handleHookEvent(
        mkEvent('PreToolUse', id, {
          tool_name: 'Read',
          tool_input: {},
          tool_use_id: 'tc-clear-1',
        }),
      );
      expect(store.getSnapshot(id)!.pendingApproval).toBeNull();
    });

    it('tracks file changes for Edit, Write, MultiEdit tools only', () => {
      const id = sid('filechange-1');
      store.handleHookEvent(mkEvent('SessionStart', id));
      store.handleHookEvent(
        mkEvent('PreToolUse', id, {
          tool_name: 'Edit',
          tool_input: { file_path: '/src/a.ts' },
          tool_use_id: 'tc-edit',
        }),
      );
      store.handleHookEvent(
        mkEvent('PreToolUse', id, {
          tool_name: 'Write',
          tool_input: { file_path: '/src/b.ts' },
          tool_use_id: 'tc-write',
        }),
      );
      store.handleHookEvent(
        mkEvent('PreToolUse', id, {
          tool_name: 'MultiEdit',
          tool_input: { file_path: '/src/c.ts' },
          tool_use_id: 'tc-multiedit',
        }),
      );
      store.handleHookEvent(
        mkEvent('PreToolUse', id, {
          tool_name: 'Bash',
          tool_input: { command: 'ls' },
          tool_use_id: 'tc-bash',
        }),
      );
      const snap = store.getSnapshot(id)!;
      expect(snap.fileChanges).toHaveLength(3);
      expect(snap.fileChanges.map(f => f.path)).toEqual(['/src/a.ts', '/src/b.ts', '/src/c.ts']);
      expect(snap.fileChanges.map(f => f.toolName)).toEqual(['Edit', 'Write', 'MultiEdit']);
    });

    describe('AskUserQuestion', () => {
      it('sets pendingQuestions and ambientState to waiting_input', () => {
        const id = sid('askq-1');
        store.handleHookEvent(mkEvent('SessionStart', id));
        store.handleHookEvent(
          mkEvent('PreToolUse', id, {
            tool_name: 'AskUserQuestion',
            tool_input: {
              questions: [
                { question: 'Which branch?', options: [{ label: 'main' }, { label: 'dev' }] },
              ],
            },
            tool_use_id: 'tc-askq-1',
          }),
        );
        const snap = store.getSnapshot(id)!;
        expect(snap.pendingQuestions).toHaveLength(1);
        expect(snap.pendingQuestions![0].question).toBe('Which branch?');
        expect(snap.ambientState).toBe('waiting_input');
        // Also clears any stale pendingApproval
        expect(snap.pendingApproval).toBeNull();
      });

      it('does not set pendingQuestions when questions is absent or not an array', () => {
        const id = sid('askq-2');
        store.handleHookEvent(mkEvent('SessionStart', id));
        store.handleHookEvent(
          mkEvent('PreToolUse', id, {
            tool_name: 'AskUserQuestion',
            tool_input: { questions: 'not-an-array' },
            tool_use_id: 'tc-askq-2',
          }),
        );
        expect(store.getSnapshot(id)!.pendingQuestions).toBeNull();
      });
    });
  });

  // ── PostToolUse ───────────────────────────────────────────────────────────

  describe('PostToolUse', () => {
    it('moves the tool call from active to completed with status complete', () => {
      const id = sid('post-1');
      store.handleHookEvent(mkEvent('SessionStart', id));
      store.handleHookEvent(
        mkEvent('PreToolUse', id, {
          tool_name: 'Bash',
          tool_input: { command: 'pwd' },
          tool_use_id: 'tc-post-1',
        }),
      );
      store.handleHookEvent(
        mkEvent('PostToolUse', id, { tool_use_id: 'tc-post-1' }),
      );
      const snap = store.getSnapshot(id)!;
      expect(snap.activeToolCalls).toHaveLength(0);
      expect(snap.completedToolCalls).toHaveLength(1);
      expect(snap.completedToolCalls[0].status).toBe('complete');
      expect(snap.completedToolCalls[0].id).toBe('tc-post-1');
    });

    it('clears pendingApproval on completion', () => {
      const id = sid('post-clear-approval-1');
      store.handleHookEvent(mkEvent('SessionStart', id));
      store.handleHookEvent(
        mkEvent('PermissionRequest', id, { tool_name: 'Bash', tool_input: {} }),
      );
      store.handleHookEvent(
        mkEvent('PreToolUse', id, {
          tool_name: 'Bash',
          tool_input: {},
          tool_use_id: 'tc-pca-1',
        }),
      );
      store.handleHookEvent(
        mkEvent('PostToolUse', id, { tool_use_id: 'tc-pca-1' }),
      );
      expect(store.getSnapshot(id)!.pendingApproval).toBeNull();
    });

    it('clears pendingQuestions when AskUserQuestion completes', () => {
      const id = sid('post-askq-1');
      store.handleHookEvent(mkEvent('SessionStart', id));
      store.handleHookEvent(
        mkEvent('PreToolUse', id, {
          tool_name: 'AskUserQuestion',
          tool_input: {
            questions: [{ question: 'Pick one', options: [{ label: 'A' }] }],
          },
          tool_use_id: 'tc-paq-1',
        }),
      );
      expect(store.getSnapshot(id)!.pendingQuestions).not.toBeNull();

      store.handleHookEvent(
        mkEvent('PostToolUse', id, { tool_use_id: 'tc-paq-1' }),
      );
      expect(store.getSnapshot(id)!.pendingQuestions).toBeNull();
    });

    it('sets ambientState to streaming', () => {
      const id = sid('post-ambient-1');
      store.handleHookEvent(mkEvent('SessionStart', id));
      store.handleHookEvent(
        mkEvent('PreToolUse', id, { tool_name: 'Read', tool_input: {}, tool_use_id: 'tc-pa-1' }),
      );
      store.handleHookEvent(mkEvent('PostToolUse', id, { tool_use_id: 'tc-pa-1' }));
      expect(store.getSnapshot(id)!.ambientState).toBe('streaming');
    });

    it('is a no-op for unknown tool_use_id', () => {
      const id = sid('post-noop-1');
      store.handleHookEvent(mkEvent('SessionStart', id));
      store.handleHookEvent(
        mkEvent('PostToolUse', id, { tool_use_id: 'unknown-id' }),
      );
      const snap = store.getSnapshot(id)!;
      expect(snap.activeToolCalls).toHaveLength(0);
      expect(snap.completedToolCalls).toHaveLength(0);
    });
  });

  // ── PostToolUseFailure ────────────────────────────────────────────────────

  describe('PostToolUseFailure', () => {
    it('marks the tool call as failed and moves it to completedToolCalls', () => {
      const id = sid('fail-1');
      store.handleHookEvent(mkEvent('SessionStart', id));
      store.handleHookEvent(
        mkEvent('PreToolUse', id, {
          tool_name: 'Bash',
          tool_input: { command: 'exit 1' },
          tool_use_id: 'tc-fail-1',
        }),
      );
      store.handleHookEvent(
        mkEvent('PostToolUseFailure', id, { tool_use_id: 'tc-fail-1' }),
      );
      const snap = store.getSnapshot(id)!;
      expect(snap.activeToolCalls).toHaveLength(0);
      expect(snap.completedToolCalls).toHaveLength(1);
      expect(snap.completedToolCalls[0].status).toBe('failed');
    });

    it('is a no-op for unknown tool_use_id', () => {
      const id = sid('fail-noop-1');
      store.handleHookEvent(mkEvent('SessionStart', id));
      store.handleHookEvent(
        mkEvent('PostToolUseFailure', id, { tool_use_id: 'unknown-id' }),
      );
      const snap = store.getSnapshot(id)!;
      expect(snap.completedToolCalls).toHaveLength(0);
    });
  });

  // ── PermissionRequest ─────────────────────────────────────────────────────

  describe('PermissionRequest', () => {
    it('populates pendingApproval and sets ambientState to waiting_approval', () => {
      const id = sid('perm-1');
      store.handleHookEvent(mkEvent('SessionStart', id));
      store.handleHookEvent(
        mkEvent('PermissionRequest', id, {
          tool_name: 'Bash',
          tool_input: { command: 'rm -rf /' },
          permission_suggestions: ['allow_once', 'deny'],
        }),
      );
      const snap = store.getSnapshot(id)!;
      expect(snap.pendingApproval).not.toBeNull();
      expect(snap.pendingApproval!.toolName).toBe('Bash');
      expect(snap.pendingApproval!.toolInput).toEqual({ command: 'rm -rf /' });
      expect(snap.pendingApproval!.suggestions).toEqual(['allow_once', 'deny']);
      expect(snap.ambientState).toBe('waiting_approval');
    });

    it('works when permission_suggestions is absent', () => {
      const id = sid('perm-no-sug-1');
      store.handleHookEvent(mkEvent('SessionStart', id));
      store.handleHookEvent(
        mkEvent('PermissionRequest', id, { tool_name: 'Bash', tool_input: {} }),
      );
      const snap = store.getSnapshot(id)!;
      expect(snap.pendingApproval!.suggestions).toBeUndefined();
    });
  });

  // ── Stop ──────────────────────────────────────────────────────────────────

  describe('Stop', () => {
    it('sets ambientState to idle and clears pendingApproval and pendingQuestions', () => {
      const id = sid('stop-1');
      store.handleHookEvent(mkEvent('SessionStart', id));
      store.handleHookEvent(
        mkEvent('PermissionRequest', id, { tool_name: 'Bash', tool_input: {} }),
      );
      store.handleHookEvent(mkEvent('Stop', id));
      const snap = store.getSnapshot(id)!;
      expect(snap.ambientState).toBe('idle');
      expect(snap.pendingApproval).toBeNull();
      expect(snap.pendingQuestions).toBeNull();
    });

    it('clears activeToolCalls and completedToolCalls on Stop', () => {
      const id = sid('stop-toolcalls-1');
      store.handleHookEvent(mkEvent('SessionStart', id));
      store.handleHookEvent(
        mkEvent('PreToolUse', id, { tool_name: 'Read', tool_input: {}, tool_use_id: 'tc-s-1' }),
      );
      store.handleHookEvent(mkEvent('PostToolUse', id, { tool_use_id: 'tc-s-1' }));
      store.handleHookEvent(mkEvent('Stop', id));
      const snap = store.getSnapshot(id)!;
      expect(snap.activeToolCalls).toHaveLength(0);
      expect(snap.completedToolCalls).toHaveLength(0);
    });

    it('keeps running subagents after Stop (only running ones survive)', () => {
      const id = sid('stop-subagent-1');
      store.handleHookEvent(mkEvent('SessionStart', id));
      store.handleHookEvent(
        mkEvent('SubagentStart', id, { agent_id: 'sa-run-1', agent_type: 'Explore' }),
      );
      store.handleHookEvent(
        mkEvent('SubagentStart', id, { agent_id: 'sa-run-2', agent_type: 'Explore' }),
      );
      store.handleHookEvent(mkEvent('SubagentStop', id, { agent_id: 'sa-run-2' }));
      store.handleHookEvent(mkEvent('Stop', id));
      // Stop keeps only running subagents
      const snap = store.getSnapshot(id)!;
      expect(snap.subagents.every(s => s.status === 'running')).toBe(true);
    });
  });

  // ── SubagentStart / SubagentStop ──────────────────────────────────────────

  describe('SubagentStart / SubagentStop', () => {
    it('SubagentStart adds a running subagent', () => {
      const id = sid('subagent-1');
      store.handleHookEvent(mkEvent('SessionStart', id));
      store.handleHookEvent(
        mkEvent('SubagentStart', id, { agent_id: 'agent-1', agent_type: 'Analyze' }),
      );
      const snap = store.getSnapshot(id)!;
      expect(snap.subagents).toHaveLength(1);
      expect(snap.subagents[0].id).toBe('agent-1');
      expect(snap.subagents[0].type).toBe('Analyze');
      expect(snap.subagents[0].status).toBe('running');
    });

    it('SubagentStart uses a fallback id when agent_id is absent', () => {
      const id = sid('subagent-fallback-1');
      store.handleHookEvent(mkEvent('SessionStart', id));
      store.handleHookEvent(mkEvent('SubagentStart', id, { agent_type: 'X' }));
      const snap = store.getSnapshot(id)!;
      expect(snap.subagents[0].id).toMatch(/^sa-\d+$/);
    });

    it('SubagentStop marks the subagent complete', () => {
      const id = sid('subagent-stop-1');
      store.handleHookEvent(mkEvent('SessionStart', id));
      store.handleHookEvent(
        mkEvent('SubagentStart', id, { agent_id: 'agent-A', agent_type: 'Z' }),
      );
      store.handleHookEvent(mkEvent('SubagentStop', id, { agent_id: 'agent-A' }));
      const snap = store.getSnapshot(id)!;
      expect(snap.subagents[0].status).toBe('complete');
      expect(snap.subagents[0].completedAt).toBeDefined();
    });

    it('SubagentStop is a no-op for unknown agent_id', () => {
      const id = sid('subagent-noop-1');
      store.handleHookEvent(mkEvent('SessionStart', id));
      store.handleHookEvent(
        mkEvent('SubagentStart', id, { agent_id: 'agent-known', agent_type: 'T' }),
      );
      store.handleHookEvent(mkEvent('SubagentStop', id, { agent_id: 'agent-unknown' }));
      // known subagent still running
      expect(store.getSnapshot(id)!.subagents[0].status).toBe('running');
    });

    it('multiple subagents can run concurrently', () => {
      const id = sid('subagent-multi-1');
      store.handleHookEvent(mkEvent('SessionStart', id));
      store.handleHookEvent(
        mkEvent('SubagentStart', id, { agent_id: 'sa-m-1', agent_type: 'A' }),
      );
      store.handleHookEvent(
        mkEvent('SubagentStart', id, { agent_id: 'sa-m-2', agent_type: 'B' }),
      );
      expect(store.getSnapshot(id)!.subagents).toHaveLength(2);
    });
  });

  // ── Notification ──────────────────────────────────────────────────────────

  describe('Notification', () => {
    it('pushes an assistant message from event.message', () => {
      const id = sid('notif-1');
      store.handleHookEvent(mkEvent('SessionStart', id));
      store.handleHookEvent(
        mkEvent('Notification', id, { message: 'Task complete!' }),
      );
      const snap = store.getSnapshot(id)!;
      expect(snap.conversation).toHaveLength(1);
      expect(snap.conversation[0].role).toBe('assistant');
      expect(snap.conversation[0].content).toBe('Task complete!');
    });

    it('falls back to event.notification when message is absent', () => {
      const id = sid('notif-2');
      store.handleHookEvent(mkEvent('SessionStart', id));
      store.handleHookEvent(
        mkEvent('Notification', id, { notification: 'Fallback text' }),
      );
      expect(store.getSnapshot(id)!.conversation[0].content).toBe('Fallback text');
    });

    it('uses [notification] placeholder when neither field is present', () => {
      const id = sid('notif-3');
      store.handleHookEvent(mkEvent('SessionStart', id));
      store.handleHookEvent(mkEvent('Notification', id));
      expect(store.getSnapshot(id)!.conversation[0].content).toBe('[notification]');
    });
  });

  // ── SessionEnd ────────────────────────────────────────────────────────────

  describe('SessionEnd', () => {
    it('sets status to ended and ambientState to idle', () => {
      const id = sid('end-1');
      store.handleHookEvent(mkEvent('SessionStart', id));
      store.handleHookEvent(mkEvent('UserPromptSubmit', id));
      store.handleHookEvent(mkEvent('SessionEnd', id));
      const snap = store.getSnapshot(id)!;
      expect(snap.status).toBe('ended');
      expect(snap.ambientState).toBe('idle');
    });
  });

  // ── Unknown hook event ────────────────────────────────────────────────────

  describe('unknown hook event name', () => {
    it('still creates the session but does not crash or alter state beyond creation', () => {
      const id = sid('unknown-1');
      store.handleHookEvent(mkEvent('SomeUnrecognisedEvent', id));
      const snap = store.getSnapshot(id);
      expect(snap).not.toBeNull();
      expect(snap!.status).toBe('active'); // default from createSession
    });
  });

  // ── hook_event_name vs event.type alias ───────────────────────────────────

  describe('event.type fallback for hook name', () => {
    it('reads hook name from event.type when hook_event_name is absent', () => {
      const id = sid('type-alias-1');
      // Some event sources set `type` rather than `hook_event_name`
      store.handleHookEvent({ type: 'SessionStart', session_id: id, cwd: '/tmp' });
      const snap = store.getSnapshot(id)!;
      expect(snap.status).toBe('active');
      expect(snap.ambientState).toBe('idle');
    });
  });

  // ── Transcript-driven fields ───────────────────────────────────────────────
  // These tests write real JSONL files to a temp dir so that the store's
  // refreshFromTranscript (which uses require('fs')) hits actual disk I/O.

  /** Write lines to a temp JSONL file and return its path. */
  function writeTempJsonl(lines: object[]): string {
    const p = path.join(os.tmpdir(), `store-test-${Date.now()}-${Math.random().toString(36).slice(2)}.jsonl`);
    fsSync.writeFileSync(p, lines.map(l => JSON.stringify(l)).join('\n') + '\n', 'utf8');
    return p;
  }

  describe('transcript-driven conversation (real temp files)', () => {
    it('reads new JSONL lines from the transcript on each hook event', () => {
      const id = sid('transcript-conv-1');
      const transcriptPath = writeTempJsonl([
        { type: 'user', message: { content: 'Hello from transcript', role: 'user' } },
        { type: 'assistant', message: { role: 'assistant', content: [{ type: 'text', text: 'Hi there!' }] } },
      ]);

      store.handleHookEvent(mkEvent('SessionStart', id, { transcript_path: transcriptPath }));

      const snap = store.getSnapshot(id)!;
      const userTurns = snap.conversation.filter(t => t.role === 'user');
      const assistantTurns = snap.conversation.filter(t => t.role === 'assistant');
      expect(userTurns.length).toBeGreaterThanOrEqual(1);
      expect(userTurns[0].content).toBe('Hello from transcript');
      expect(assistantTurns.length).toBeGreaterThanOrEqual(1);
      expect(assistantTurns[0].content).toBe('Hi there!');

      fsSync.unlinkSync(transcriptPath);
    });

    it('skips tool_result user messages — they are API plumbing', () => {
      const id = sid('transcript-skip-1');
      const transcriptPath = writeTempJsonl([
        {
          type: 'user',
          message: {
            content: [{ type: 'tool_result', tool_use_id: 'tc-x', content: 'result' }],
            role: 'user',
          },
        },
      ]);

      store.handleHookEvent(mkEvent('SessionStart', id, { transcript_path: transcriptPath }));
      const snap = store.getSnapshot(id)!;
      // tool_result user messages must NOT appear as conversation turns
      const userTurns = snap.conversation.filter(t => t.role === 'user');
      expect(userTurns).toHaveLength(0);

      fsSync.unlinkSync(transcriptPath);
    });

    it('records tool_use blocks from assistant messages and increments totalToolCalls', () => {
      const id = sid('transcript-tool-1');
      const transcriptPath = writeTempJsonl([
        {
          type: 'assistant',
          message: {
            role: 'assistant',
            content: [{ type: 'tool_use', id: 'tc-tr-1', name: 'Bash', input: { command: 'ls' } }],
          },
        },
      ]);

      store.handleHookEvent(mkEvent('SessionStart', id, { transcript_path: transcriptPath }));
      const snap = store.getSnapshot(id)!;
      expect(snap.totalToolCalls).toBe(1);
      const toolTurns = snap.conversation.filter(t => t.toolCalls && t.toolCalls.length > 0);
      expect(toolTurns).toHaveLength(1);
      expect(toolTurns[0].toolCalls![0].name).toBe('Bash');

      fsSync.unlinkSync(transcriptPath);
    });

    it('applies usage from assistant messages and updates session.usage', () => {
      const id = sid('transcript-usage-1');
      const transcriptPath = writeTempJsonl([
        {
          type: 'assistant',
          message: {
            id: 'msg-u-1',
            role: 'assistant',
            model: 'claude-sonnet-4-5',
            content: [],
            usage: { input_tokens: 100, output_tokens: 50, cache_creation_input_tokens: 0, cache_read_input_tokens: 0 },
          },
        },
      ]);

      store.handleHookEvent(mkEvent('SessionStart', id, { transcript_path: transcriptPath }));
      const snap = store.getSnapshot(id)!;
      expect(snap.usage).not.toBeNull();
      expect(snap.usage!.model).toBe('claude-sonnet-4-5');
      expect(snap.usage!.contextTokens).toBe(100);
      expect(snap.usage!.totalInputTokens).toBe(100);

      fsSync.unlinkSync(transcriptPath);
    });

    it('does not double-count usage for the same message id across multiple events', () => {
      const id = sid('transcript-dedup-1');
      // The transcript has one assistant message with a stable id.
      // We'll trigger two hook events after attaching — the store's
      // lastTranscriptLine cursor prevents re-processing already-seen lines,
      // and applyUsage's key dedup prevents double-counting even if the same
      // line were processed twice.
      const transcriptPath = writeTempJsonl([
        {
          type: 'assistant',
          message: {
            id: 'msg-dedup-1',
            role: 'assistant',
            model: 'claude-sonnet-4-5',
            content: [],
            usage: { input_tokens: 200, output_tokens: 30 },
          },
        },
      ]);

      store.handleHookEvent(mkEvent('SessionStart', id, { transcript_path: transcriptPath }));
      // Second event — no new lines; lastTranscriptLine is already 1
      store.handleHookEvent(mkEvent('UserPromptSubmit', id));

      const snap = store.getSnapshot(id)!;
      // Should only count once: the first read consumed the line
      expect(snap.usage!.totalInputTokens).toBe(200);

      fsSync.unlinkSync(transcriptPath);
    });

    it('ignores thinking blocks in assistant messages', () => {
      const id = sid('transcript-thinking-1');
      const transcriptPath = writeTempJsonl([
        {
          type: 'assistant',
          message: {
            role: 'assistant',
            content: [
              { type: 'thinking', thinking: 'some internal reasoning' },
              { type: 'text', text: 'Visible text' },
            ],
          },
        },
      ]);

      store.handleHookEvent(mkEvent('SessionStart', id, { transcript_path: transcriptPath }));
      const snap = store.getSnapshot(id)!;
      const textTurns = snap.conversation.filter(t => t.role === 'assistant' && t.content === 'Visible text');
      expect(textTurns).toHaveLength(1);
      // Thinking block must not produce a conversation turn
      const thinkingTurns = snap.conversation.filter(t => t.content && t.content.includes('internal reasoning'));
      expect(thinkingTurns).toHaveLength(0);

      fsSync.unlinkSync(transcriptPath);
    });

    it('attaches tool_result to the matching tool call as response', () => {
      const id = sid('transcript-result-1');
      const transcriptPath = writeTempJsonl([
        {
          type: 'assistant',
          message: {
            role: 'assistant',
            content: [{ type: 'tool_use', id: 'tc-res-1', name: 'Bash', input: { command: 'pwd' } }],
          },
        },
        {
          type: 'user',
          message: {
            content: [{ type: 'tool_result', tool_use_id: 'tc-res-1', content: '/home/user' }],
            role: 'user',
          },
        },
      ]);

      store.handleHookEvent(mkEvent('SessionStart', id, { transcript_path: transcriptPath }));
      const snap = store.getSnapshot(id)!;
      const toolTurns = snap.conversation.filter(t => t.toolCalls?.some(tc => tc.id === 'tc-res-1'));
      expect(toolTurns).toHaveLength(1);
      expect(toolTurns[0].toolCalls![0].response).toBe('/home/user');

      fsSync.unlinkSync(transcriptPath);
    });
  });

  // ── getAllSnapshots ────────────────────────────────────────────────────────

  describe('getAllSnapshots', () => {
    it('returns snapshots for all sessions seen so far', () => {
      const all = store.getAllSnapshots();
      expect(Array.isArray(all)).toBe(true);
      // We created many sessions across all the tests above
      expect(all.length).toBeGreaterThan(0);
    });

    it('every snapshot has a sessionId string', () => {
      const all = store.getAllSnapshots();
      expect(all.every(s => typeof s.sessionId === 'string' && s.sessionId.length > 0)).toBe(true);
    });

    it('snapshot objects are independent copies (spread), not shared references', () => {
      const id = sid('copy-1');
      store.handleHookEvent(mkEvent('SessionStart', id));
      const snap1 = store.getSnapshot(id)!;
      const snap2 = store.getSnapshot(id)!;
      expect(snap1).not.toBe(snap2); // different object references
      expect(snap1).toEqual(snap2);  // same data
    });
  });

  // ── Multi-session isolation ───────────────────────────────────────────────

  describe('session isolation', () => {
    it('events for session A do not affect session B', () => {
      const idA = sid('iso-A');
      const idB = sid('iso-B');
      store.handleHookEvent(mkEvent('SessionStart', idA));
      store.handleHookEvent(mkEvent('SessionStart', idB));
      store.handleHookEvent(
        mkEvent('PermissionRequest', idA, { tool_name: 'Bash', tool_input: {} }),
      );
      expect(store.getSnapshot(idA)!.pendingApproval).not.toBeNull();
      expect(store.getSnapshot(idB)!.pendingApproval).toBeNull();
    });

    it('SubagentStart on session A does not appear in session B', () => {
      const idA = sid('iso-sub-A');
      const idB = sid('iso-sub-B');
      store.handleHookEvent(mkEvent('SessionStart', idA));
      store.handleHookEvent(mkEvent('SessionStart', idB));
      store.handleHookEvent(
        mkEvent('SubagentStart', idA, { agent_id: 'sa-iso', agent_type: 'X' }),
      );
      expect(store.getSnapshot(idA)!.subagents).toHaveLength(1);
      expect(store.getSnapshot(idB)!.subagents).toHaveLength(0);
    });
  });

  // ── Notification deduplication behaviour ─────────────────────────────────
  // The Notification case pushes directly to conversation without going through
  // isDuplicateMessage — so duplicate notifications ARE stored (current behavior).

  describe('Notification — deduplication behaviour', () => {
    it('allows duplicate notification messages (they are pushed without dedup check)', () => {
      const id = sid('dedup-notif-1');
      store.handleHookEvent(mkEvent('SessionStart', id));
      store.handleHookEvent(mkEvent('Notification', id, { message: 'Same message' }));
      store.handleHookEvent(mkEvent('Notification', id, { message: 'Same message' }));
      // Notification does NOT call isDuplicateMessage — both entries appear
      const conv = store.getSnapshot(id)!.conversation;
      const matches = conv.filter(t => t.content === 'Same message');
      expect(matches.length).toBe(2);
    });
  });
});
