/**
 * Behavioural tests for claudeSessionStore's analytics-snapshot lifecycle and
 * hook-event cwd backfill:
 *
 *   - every turn's Stop re-arms the delayed history write (historyWritten used
 *     to be set once and never cleared, so only the FIRST Stop of a session
 *     ever snapshotted analytics — long sessions kept turn-1 usage forever);
 *   - SessionEnd still suppresses an in-flight Stop timer (no 'active' write
 *     reverting the final 'ended' row), and a stray post-end prompt does not
 *     re-arm it;
 *   - sessions first created by a conversation delta (cwd '') get their cwd
 *     backfilled from the first hook event that carries one.
 *
 * Strategy: mock every side-effect collaborator (electron, notifier, watcher,
 * telemetry, analytics writer) so only the store's own logic runs, and drive
 * the 1500 ms Stop timer with fake timers.
 */

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import * as path from 'path';

vi.mock('electron', () => ({ BrowserWindow: class {} }));
vi.mock('./agentNotifier', () => ({ agentNotifier: { notifyOnTransition: vi.fn() } }));
vi.mock('./supervisorNudge', () => ({ supervisorNudge: { onBlock: vi.fn() } }));
vi.mock('./workflowWatcher', () => ({
  workflowWatcher: { attach: vi.fn(), detach: vi.fn(), poke: vi.fn() },
}));
vi.mock('./hubTelemetry', () => ({
  publishWorkflowRuns: vi.fn(),
  publishSnapshot: vi.fn(),
  forgetSession: vi.fn(),
}));
vi.mock('./claudemonDaemon', () => ({ CLAUDEMON_API_URL: 'http://127.0.0.1:0' }));
vi.mock('./sessionStore/usageAccumulator', () => ({
  SessionUsageAccumulator: class {
    applyUsage(): void {}
    forget(): void {}
  },
}));
vi.mock('./sessionStore/analyticsWriter', () => ({ writeHistory: vi.fn() }));

import { claudeSessionStore } from './claudeSessionStore';
import { writeHistory } from './sessionStore/analyticsWriter';

const writeHistoryMock = vi.mocked(writeHistory);

let seq = 0;
const uniqueId = (): string => `sess-test-${++seq}`;

function hook(sessionId: string, hookName: string, cwd = '/proj'): void {
  claudeSessionStore.handleHookEvent({
    hook_event_name: hookName,
    session_id: sessionId,
    cwd,
  });
}

beforeEach(() => {
  vi.useFakeTimers();
  writeHistoryMock.mockClear();
});

afterEach(() => {
  vi.useRealTimers();
});

describe('Stop → analytics snapshot re-arms each turn', () => {
  it('writes an active-history snapshot for EVERY turn boundary, not just the first', () => {
    const sid = uniqueId();

    hook(sid, 'UserPromptSubmit');
    hook(sid, 'Stop');
    vi.advanceTimersByTime(1600);
    expect(writeHistoryMock).toHaveBeenCalledTimes(1);
    expect(writeHistoryMock.mock.calls[0][1]).toBe('active');

    // Second turn: the new prompt must re-arm the snapshot so this Stop also
    // writes (the historyWritten guard used to bail forever after turn 1).
    hook(sid, 'UserPromptSubmit');
    hook(sid, 'Stop');
    vi.advanceTimersByTime(1600);
    expect(writeHistoryMock).toHaveBeenCalledTimes(2);
    expect(writeHistoryMock.mock.calls[1][1]).toBe('active');
  });

  it('SessionEnd suppresses an in-flight Stop timer and writes the terminal row once', () => {
    const sid = uniqueId();

    hook(sid, 'UserPromptSubmit');
    hook(sid, 'Stop');
    // SessionEnd lands before the 1500 ms Stop timer fires.
    hook(sid, 'SessionEnd');
    expect(writeHistoryMock).toHaveBeenCalledTimes(1);
    expect(writeHistoryMock.mock.calls[0][1]).toBe('ended');

    // The pending Stop timer must bail — no 'active' write reverting 'ended'.
    vi.advanceTimersByTime(1600);
    expect(writeHistoryMock).toHaveBeenCalledTimes(1);
  });

  it('a stray prompt after SessionEnd does not re-arm the snapshot', () => {
    const sid = uniqueId();

    hook(sid, 'Stop');
    hook(sid, 'SessionEnd');
    writeHistoryMock.mockClear();

    // Session is 'ended' (still resident until the eviction grace period):
    // late hooks must not let a Stop overwrite the terminal row.
    hook(sid, 'UserPromptSubmit');
    hook(sid, 'Stop');
    vi.advanceTimersByTime(1600);
    expect(writeHistoryMock).not.toHaveBeenCalled();
  });
});

describe('cwd backfill for delta-created sessions', () => {
  it("backfills cwd from the first hook when the session was created by a conversation delta with cwd ''", () => {
    const sid = uniqueId();

    // Deltas can outrun the first hook — the store creates the entry with ''.
    claudeSessionStore.applyConversationDelta({
      session_id: sid,
      seq: 0,
      items: [],
      reset: false,
    } as never);
    expect(claudeSessionStore.getSnapshot(sid)?.cwd).toBe('');

    hook(sid, 'UserPromptSubmit', '/real/project');
    expect(claudeSessionStore.getSnapshot(sid)?.cwd).toBe(path.resolve('/real/project'));
  });

  it('never overwrites an already-known cwd', () => {
    const sid = uniqueId();

    hook(sid, 'UserPromptSubmit', '/first');
    expect(claudeSessionStore.getSnapshot(sid)?.cwd).toBe(path.resolve('/first'));

    hook(sid, 'PostToolUse', '/second');
    expect(claudeSessionStore.getSnapshot(sid)?.cwd).toBe(path.resolve('/first'));
  });
});

describe('managed pending → approval/question cards', () => {
  // Managed providers (codex/opencode/pi) fire no hooks — the daemon's
  // `pending` slot on Managed frames is the only source for the needs-you
  // dock / inbox / fleet-card approval UI.
  function managedSession(provider = 'codex'): string {
    const sid = uniqueId();
    claudeSessionStore.applyConversationDelta({
      session_id: sid,
      seq: 0,
      items: [],
      reset: false,
    } as never);
    claudeSessionStore.applyManagedMode(sid, 'responding', { provider, pending: null });
    return sid;
  }

  it('maps a codex approval into pendingApproval and clears it when resolved', () => {
    const sid = managedSession();
    claudeSessionStore.applyManagedMode(sid, 'approval', {
      provider: 'codex',
      pending: {
        kind: 'approval',
        tool: 'exec_command',
        summary: 'npm test',
        raw: { command: ['npm', 'test'], cwd: '/proj' },
      },
    });
    const snap = claudeSessionStore.getSnapshot(sid);
    expect(snap?.pendingApproval?.toolName).toBe('exec_command');
    expect(snap?.pendingApproval?.toolInput).toEqual({ command: ['npm', 'test'], cwd: '/proj' });
    expect(snap?.ambientState).toBe('waiting_approval');

    claudeSessionStore.applyManagedMode(sid, 'responding', { provider: 'codex', pending: null });
    expect(claudeSessionStore.getSnapshot(sid)?.pendingApproval).toBeNull();
  });

  it('keeps the timestamp of an unchanged approval across re-broadcast frames', () => {
    const sid = managedSession();
    const pending = { kind: 'approval', tool: 'exec_command', raw: { command: 'ls' } } as const;
    claudeSessionStore.applyManagedMode(sid, 'approval', { provider: 'codex', pending });
    const first = claudeSessionStore.getSnapshot(sid)?.pendingApproval?.timestamp;
    vi.advanceTimersByTime(500);
    // The daemon re-sends Approval frames on unrelated updates; a bumped
    // timestamp would resurrect a card the user already dismissed.
    claudeSessionStore.applyManagedMode(sid, 'approval', { provider: 'codex', pending });
    expect(claudeSessionStore.getSnapshot(sid)?.pendingApproval?.timestamp).toBe(first);
  });

  it('maps a question payload into pendingQuestions', () => {
    const sid = managedSession('opencode');
    claudeSessionStore.applyManagedMode(sid, 'question', {
      provider: 'opencode',
      pending: {
        kind: 'question',
        questions: [
          {
            question: 'Which db?',
            header: 'DB',
            multi_select: false,
            options: [{ label: 'sqlite' }],
          },
        ],
        raw: {},
      },
    });
    const snap = claudeSessionStore.getSnapshot(sid);
    expect(snap?.pendingQuestions).toHaveLength(1);
    expect(snap?.pendingQuestions?.[0].question).toBe('Which db?');
    expect(snap?.pendingApproval).toBeNull();
  });

  it('never drives the cards for claude sessions (hook-owned)', () => {
    const sid = uniqueId();
    hook(sid, 'UserPromptSubmit');
    claudeSessionStore.applyManagedMode(sid, 'approval', {
      provider: 'claude',
      pending: { kind: 'approval', tool: 'Bash', raw: { tool_input: { command: 'ls' } } },
    });
    expect(claudeSessionStore.getSnapshot(sid)?.pendingApproval).toBeNull();
  });
});

describe('liveCwd follows the agent into and out of a worktree', () => {
  it('tracks a mid-session cwd change without touching the spawn cwd', () => {
    const sid = uniqueId();

    hook(sid, 'UserPromptSubmit', '/proj');
    expect(claudeSessionStore.getSnapshot(sid)?.liveCwd).toBeUndefined();

    // Agent enters a worktree — subsequent hooks carry the worktree cwd.
    hook(sid, 'PostToolUse', '/proj-worktrees/feature-x');
    const snap = claudeSessionStore.getSnapshot(sid);
    expect(snap?.cwd).toBe(path.resolve('/proj'));
    expect(snap?.liveCwd).toBe(path.resolve('/proj-worktrees/feature-x'));
  });

  it('clears liveCwd when the agent returns home', () => {
    const sid = uniqueId();

    hook(sid, 'UserPromptSubmit', '/proj');
    hook(sid, 'PostToolUse', '/proj-worktrees/feature-x');
    expect(claudeSessionStore.getSnapshot(sid)?.liveCwd).toBe(
      path.resolve('/proj-worktrees/feature-x'),
    );

    hook(sid, 'PostToolUse', '/proj');
    expect(claudeSessionStore.getSnapshot(sid)?.liveCwd).toBeUndefined();
  });

  it('an event with no cwd leaves liveCwd untouched', () => {
    const sid = uniqueId();

    hook(sid, 'UserPromptSubmit', '/proj');
    hook(sid, 'PostToolUse', '/proj-worktrees/feature-x');

    claudeSessionStore.handleHookEvent({ hook_event_name: 'Stop', session_id: sid });
    expect(claudeSessionStore.getSnapshot(sid)?.liveCwd).toBe(
      path.resolve('/proj-worktrees/feature-x'),
    );
  });
});
