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
vi.mock('./supervisorNudge', () => ({
  supervisorNudge: {
    onBlock: vi.fn(),
    onBlockCleared: vi.fn(),
    onFinished: vi.fn(),
    sweepMissedFinishes: vi.fn(),
    forgetWorker: vi.fn(),
  },
}));
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
    static refreshContextLimit(): void {}
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

describe('SessionEnd eviction does not reach past its own lifetime', () => {
  // A restart reuses the session id on purpose: the renderer closes the session
  // and immediately respawns with `resumeSessionId` pinned to the old id, and
  // for managed providers claudemon's `deregister_managed` broadcast is turned
  // into a synthetic SessionEnd. So the eviction scheduled by the dying life can
  // land 30 s into the *successor's* life. It must not fire against it.
  //
  // The pre-existing eviction test only advances 31 s BEFORE the second life
  // begins, so it never exercised this window.
  it("a session revived inside the grace period survives the old life's eviction", () => {
    const sid = uniqueId();

    // ── First life ── ends, scheduling a 30 s eviction.
    hook(sid, 'SessionStart');
    claudeSessionStore.setSpawnMeta(sid, { label: 'my agent', isSupervisor: true });
    hook(sid, 'Stop');
    hook(sid, 'SessionEnd');

    // ── Restart lands well inside the grace period ──
    vi.advanceTimersByTime(5_000);
    hook(sid, 'SessionStart');
    expect(claudeSessionStore.getSnapshot(sid)).toBeTruthy();

    // The old life's timer now fires against the running successor.
    vi.advanceTimersByTime(31_000);

    const snap = claudeSessionStore.getSnapshot(sid);
    expect(snap, 'the live session must still be resident').toBeTruthy();
    expect(
      claudeSessionStore.getSpawnMeta?.(sid)?.label ?? 'my agent',
      'spawn metadata must not be evicted from under a live session',
    ).toBe('my agent');
  });

  it('still evicts a session that stays ended', () => {
    const sid = uniqueId();
    hook(sid, 'SessionStart');
    hook(sid, 'Stop');
    hook(sid, 'SessionEnd');

    vi.advanceTimersByTime(31_000);

    expect(
      claudeSessionStore.getSnapshot(sid),
      'the guard must not turn eviction into a leak',
    ).toBeFalsy();
  });
});

describe('SessionEnd eviction cleans per-session Maps (convSeq)', () => {
  // The SessionEnd eviction timer used to remove only `this.sessions` and the
  // usage accumulator entry. `convSeq` survived, so when a session id is reused
  // on resume, the resumed life inherited the prior life's stale seq. With no
  // `reset` frame, the first delta of the new life then looked like a gap and
  // forced an unnecessary snapshot resync (an HTTP fetch) instead of applying
  // the delta directly.
  it('a resumed (reused-id) session applies its first delta instead of resyncing', () => {
    const sid = uniqueId();

    // ── First life ── empty heartbeat creates the session and advances convSeq to 3.
    claudeSessionStore.applyConversationDelta({
      session_id: sid,
      seq: 3,
      items: [],
      reset: false,
    } as never);

    // End the session and let the 30 s eviction grace period elapse.
    hook(sid, 'Stop');
    hook(sid, 'SessionEnd');
    vi.advanceTimersByTime(31_000); // fire the eviction timer

    // ── Second life (same id, e.g. resume) ── first delta is seq 1 with one
    // item and no `reset`. A stale convSeq=3 would make 1 !== 3+1 → resync.
    const originalFetch = (global as { fetch?: typeof fetch }).fetch;
    const fetchSpy = vi.fn(async () => ({ ok: false }) as Response);
    (global as { fetch?: typeof fetch }).fetch = fetchSpy as unknown as typeof fetch;
    try {
      claudeSessionStore.applyConversationDelta({
        session_id: sid,
        seq: 1,
        items: [{ kind: 'user_message', text: 'resumed hello' }],
        reset: false,
      } as never);

      // No gap → no snapshot resync fetch, and the delta is applied in place.
      expect(fetchSpy).not.toHaveBeenCalled();
      const conv = claudeSessionStore.getSnapshot(sid)?.conversation ?? [];
      expect(conv.some((t) => t.content === 'resumed hello')).toBe(true);
    } finally {
      (global as { fetch?: typeof fetch }).fetch = originalFetch;
    }
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

  it('surfaces a STREAM-transport Claude approval into pendingApproval (control protocol, no hook)', () => {
    // Stream Claude routes approvals through the control protocol
    // (can_use_tool), not a PermissionRequest hook, so the daemon's `pending`
    // slot is the only source — the desktop must fold it just like a managed
    // provider. Regression: the old `provider !== 'claude'` guard dropped it.
    const sid = uniqueId();
    claudeSessionStore.applyConversationDelta({
      session_id: sid,
      seq: 0,
      items: [],
      reset: false,
    } as never);
    claudeSessionStore.applyManagedMode(sid, 'responding', {
      provider: 'claude',
      transport: 'stream',
      pending: null,
    });
    claudeSessionStore.applyManagedMode(sid, 'approval', {
      provider: 'claude',
      transport: 'stream',
      pending: { kind: 'approval', tool: 'Bash', raw: { tool_input: { command: 'rm -rf build' } } },
    });
    const snap = claudeSessionStore.getSnapshot(sid);
    expect(snap?.pendingApproval?.toolName).toBe('Bash');
    expect(snap?.pendingApproval?.toolInput).toEqual({ command: 'rm -rf build' });
    expect(snap?.ambientState).toBe('waiting_approval');

    // Resolving (daemon clears the slot) clears the card.
    claudeSessionStore.applyManagedMode(sid, 'responding', {
      provider: 'claude',
      transport: 'stream',
      pending: null,
    });
    expect(claudeSessionStore.getSnapshot(sid)?.pendingApproval).toBeNull();
  });

  it('does NOT fold the daemon pending for PTY-transport Claude (the hook path owns it)', () => {
    // PTY Claude gets a real PermissionRequest hook; hookEventRouter owns
    // pendingApproval. A Managed frame's pending must not race it.
    const sid = uniqueId();
    claudeSessionStore.applyConversationDelta({
      session_id: sid,
      seq: 0,
      items: [],
      reset: false,
    } as never);
    claudeSessionStore.applyManagedMode(sid, 'approval', {
      provider: 'claude',
      pending: { kind: 'approval', tool: 'Bash', raw: { tool_input: { command: 'ls' } } },
    });
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

  it('a frame with NO pending key leaves a live approval card alone; pending:null clears it', () => {
    const sid = managedSession();
    claudeSessionStore.applyManagedMode(sid, 'approval', {
      provider: 'codex',
      pending: { kind: 'approval', tool: 'exec_command', raw: { command: 'npm test' } },
    });
    expect(claudeSessionStore.getSnapshot(sid)?.pendingApproval?.toolName).toBe('exec_command');

    // Tri-state contract: `undefined` (key absent) = the caller carried no
    // pending info — a session-list reconciler or hub bridge that omits it must
    // NOT wipe a live card mid-decision…
    claudeSessionStore.applyManagedMode(sid, 'working', { provider: 'codex' });
    expect(claudeSessionStore.getSnapshot(sid)?.pendingApproval?.toolName).toBe('exec_command');

    // …while an explicit `null` means the daemon says nothing is pending: clear.
    claudeSessionStore.applyManagedMode(sid, 'responding', { provider: 'codex', pending: null });
    expect(claudeSessionStore.getSnapshot(sid)?.pendingApproval).toBeNull();
  });

  it('prefers a gateway-shaped tool_input envelope over the whole raw object', () => {
    const sid = managedSession();
    claudeSessionStore.applyManagedMode(sid, 'approval', {
      provider: 'codex',
      pending: {
        kind: 'approval',
        tool: 'Bash',
        // Gateway-shaped payload: the params live under `tool_input`. Mapping
        // the whole raw object instead would change both the rendered card and
        // the JSON.stringify dedup signature (resurrecting dismissed cards on
        // every re-broadcast frame).
        raw: { tool_input: { command: 'rm -rf /tmp/x' } },
      },
    });
    expect(claudeSessionStore.getSnapshot(sid)?.pendingApproval?.toolInput).toEqual({
      command: 'rm -rf /tmp/x',
    });
  });

  it('an approval frame clears pendingQuestions and a question frame clears pendingApproval', () => {
    const sid = managedSession();
    claudeSessionStore.applyManagedMode(sid, 'approval', {
      provider: 'codex',
      pending: { kind: 'approval', tool: 'exec_command', raw: { command: 'ls' } },
    });
    claudeSessionStore.applyManagedMode(sid, 'question', {
      provider: 'codex',
      pending: {
        kind: 'question',
        questions: [
          { question: 'Proceed?', header: null, multi_select: false, options: [{ label: 'Yes' }] },
        ],
        raw: {},
      },
    });
    let snap = claudeSessionStore.getSnapshot(sid);
    expect(snap?.pendingApproval).toBeNull();
    expect(snap?.pendingQuestions).toHaveLength(1);

    claudeSessionStore.applyManagedMode(sid, 'approval', {
      provider: 'codex',
      pending: { kind: 'approval', tool: 'exec_command', raw: { command: 'ls' } },
    });
    snap = claudeSessionStore.getSnapshot(sid);
    expect(snap?.pendingQuestions).toBeNull();
    expect(snap?.pendingApproval?.toolName).toBe('exec_command');
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

// Managed providers (codex/opencode/pi) never emit a transcript `usage` item,
// so usageAccumulator.applyUsage — the only other peakContext writer — never
// runs for them. Without applyStatusLine also feeding peakContext, their
// session_history rows report peakContext:0 forever, even though the daemon's
// statusLine carried the real percentage the whole time.
describe('applyStatusLine feeds peakContext for statusLine-only (codex-shaped) sessions', () => {
  it('derives peakContext from contextUsedPct * contextWindowSize and tracks the max', () => {
    const sid = uniqueId();
    hook(sid, 'SessionStart');
    expect(claudeSessionStore.getSnapshot(sid)?.peakContext).toBe(0);

    claudeSessionStore.applyStatusLine(sid, {
      modelDisplay: 'gpt-5-codex',
      contextUsedPct: 10,
      contextWindowSize: 272_000,
    });
    expect(claudeSessionStore.getSnapshot(sid)?.peakContext).toBe(27_200);

    // A later, smaller reading must not pull the peak back down.
    claudeSessionStore.applyStatusLine(sid, {
      modelDisplay: 'gpt-5-codex',
      contextUsedPct: 5,
      contextWindowSize: 272_000,
    });
    expect(claudeSessionStore.getSnapshot(sid)?.peakContext).toBe(27_200);

    // A higher reading raises it further.
    claudeSessionStore.applyStatusLine(sid, {
      modelDisplay: 'gpt-5-codex',
      contextUsedPct: 40,
      contextWindowSize: 272_000,
    });
    expect(claudeSessionStore.getSnapshot(sid)?.peakContext).toBe(108_800);
  });

  it('leaves peakContext untouched when the statusLine carries no context reading', () => {
    const sid = uniqueId();
    hook(sid, 'SessionStart');

    claudeSessionStore.applyStatusLine(sid, { modelDisplay: 'gpt-5-codex', costUSD: 0.4 });
    expect(claudeSessionStore.getSnapshot(sid)?.peakContext).toBe(0);
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

/**
 * A restart-with-settings respawns onto the same pinned id, so the row survives
 * into the new life carrying the *previous* life's hook telemetry. The composer
 * pill prefers livePermissionMode over settings.permissionMode, so a stale value
 * left behind makes "Restart with Full access" look like it did nothing until
 * some later hook happens to correct it.
 */
describe('setSpawnMeta on a restart clears the previous life’s live permission mode', () => {
  it('drops a stale livePermissionMode when the respawn requests a mode', () => {
    const sid = uniqueId();
    hook(sid, 'UserPromptSubmit');
    claudeSessionStore.handleHookEvent({
      hook_event_name: 'PostToolUse',
      session_id: sid,
      cwd: '/proj',
      permission_mode: 'default',
    });
    expect(claudeSessionStore.getSnapshot(sid)?.livePermissionMode).toBe('default');

    claudeSessionStore.setSpawnMeta(sid, {
      settings: { permissionMode: 'bypassPermissions', bypassAvailable: true },
    });

    const snap = claudeSessionStore.getSnapshot(sid);
    expect(snap?.livePermissionMode).toBeUndefined();
    expect(snap?.settings?.permissionMode).toBe('bypassPermissions');
    expect(snap?.settings?.bypassAvailable).toBe(true);
  });

  it('leaves live telemetry alone when the respawn only changes the model', () => {
    const sid = uniqueId();
    hook(sid, 'UserPromptSubmit');
    claudeSessionStore.handleHookEvent({
      hook_event_name: 'PostToolUse',
      session_id: sid,
      cwd: '/proj',
      permission_mode: 'acceptEdits',
    });

    claudeSessionStore.setSpawnMeta(sid, { settings: { model: 'opus' } });

    expect(claudeSessionStore.getSnapshot(sid)?.livePermissionMode).toBe('acceptEdits');
  });
});

// ── close_session: dismissing a finished session's row ──────────────────────
//
// SIGTERM stops a worker but its row lingers: the 30s eviction is armed by a
// SessionEnd hook, and a killed process often emits none, so "did it actually
// die" was answered by sending ANOTHER signal and reading the 404.
describe('closeSession — dismissal is a verb', () => {
  it('removes an IDLE session from the store, and reports it was still live', () => {
    const sid = uniqueId();
    hook(sid, 'SessionStart');
    hook(sid, 'Stop');
    expect(claudeSessionStore.getSnapshot(sid)).toBeTruthy();

    const res = claudeSessionStore.closeSession(sid);
    expect(res).toEqual({ ok: true, removed: true, wasLive: true });
    expect(claudeSessionStore.getSnapshot(sid)).toBeFalsy();
    expect(claudeSessionStore.getAllSnapshots().some((s) => s.sessionId === sid)).toBe(false);
  });

  it('REFUSES a session that is still working, and leaves it untouched', () => {
    const sid = uniqueId();
    hook(sid, 'SessionStart');
    hook(sid, 'UserPromptSubmit');
    expect(claudeSessionStore.getSnapshot(sid)?.ambientState).not.toBe('idle');

    expect(() => claudeSessionStore.closeSession(sid)).toThrow(/still working/);
    // Still there, still working — a refusal must not half-dismiss.
    expect(claudeSessionStore.getSnapshot(sid)).toBeTruthy();
  });

  it('is IDEMPOTENT: closing an unknown session succeeds rather than erroring', () => {
    // The whole point is a definitive answer; "no such row" as an ERROR is the
    // ambiguity this replaces.
    expect(claudeSessionStore.closeSession('never-existed')).toEqual({
      ok: true,
      removed: false,
      wasLive: false,
    });
  });

  it('reports wasLive:false for a session that had already ENDED', () => {
    const sid = uniqueId();
    hook(sid, 'SessionStart');
    hook(sid, 'SessionEnd');
    expect(claudeSessionStore.closeSession(sid)).toEqual({
      ok: true,
      removed: true,
      wasLive: false,
    });
  });

  it('runs the SAME teardown as the eviction timer (per-session maps cleared)', () => {
    const sid = uniqueId();
    // Advance convSeq to 3, then dismiss without ever ending the session.
    claudeSessionStore.applyConversationDelta({
      session_id: sid,
      seq: 3,
      items: [],
      reset: false,
    } as never);
    claudeSessionStore.closeSession(sid);

    // A reused id must start fresh: a surviving convSeq=3 would make the next
    // life's seq-1 delta look like a gap and force a snapshot resync fetch.
    const originalFetch = (global as { fetch?: typeof fetch }).fetch;
    const fetchSpy = vi.fn(async () => ({ ok: false }) as Response);
    (global as { fetch?: typeof fetch }).fetch = fetchSpy as unknown as typeof fetch;
    try {
      claudeSessionStore.applyConversationDelta({
        session_id: sid,
        seq: 1,
        items: [{ kind: 'user_message', text: 'reused hello' }],
        reset: false,
      } as never);
      expect(fetchSpy).not.toHaveBeenCalled();
    } finally {
      (global as { fetch?: typeof fetch }).fetch = originalFetch;
    }
  });

  it('cancels the pending eviction so a later timer cannot fire on a reused id', () => {
    const sid = uniqueId();
    hook(sid, 'SessionStart');
    hook(sid, 'SessionEnd'); // arms the 30s eviction
    claudeSessionStore.closeSession(sid);

    // Reuse the id immediately (a respawn), then let the old timer's deadline
    // pass. Without cancelEviction, it would delete the NEW session's row.
    hook(sid, 'SessionStart');
    vi.advanceTimersByTime(31_000);
    expect(claudeSessionStore.getSnapshot(sid)).toBeTruthy();
  });
});
