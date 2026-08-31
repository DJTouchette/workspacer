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

import { claudeSessionStore, contextTokensFromStatusLine } from './claudeSessionStore';
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

// The only token figure a managed (non-Claude) session has is DERIVED — pct ×
// window — so it inherits every error in either input. A percentage is bounded
// by definition, but nothing upstream enforces that: claudemon reads
// `used_percentage` straight off the provider payload (session/state.rs) and
// does not clamp it. An unclamped multiply is one way an absurd token figure
// reaches a client; the other is an inflated WINDOW, which is why the
// stream transport's sub-agent bug (providers/claude_stream.rs) is a
// token-side bug as well as a limit-side one.
describe('contextTokensFromStatusLine — a derived count cannot exceed its window', () => {
  it('derives the ordinary case', () => {
    expect(contextTokensFromStatusLine({ contextUsedPct: 10, contextWindowSize: 272_000 })).toBe(
      27_200,
    );
  });

  it('clamps a percentage above 100 instead of multiplying the window by it', () => {
    // A provider reporting a running total where a percentage was expected
    // used to yield 4300% of 200k = 8.6 MILLION tokens on a 200k session.
    expect(contextTokensFromStatusLine({ contextUsedPct: 4_300, contextWindowSize: 200_000 })).toBe(
      200_000,
    );
  });

  it('clamps a negative percentage to zero rather than reporting negative tokens', () => {
    expect(contextTokensFromStatusLine({ contextUsedPct: -5, contextWindowSize: 200_000 })).toBe(0);
  });

  it('says nothing when either input is missing', () => {
    expect(contextTokensFromStatusLine({ contextWindowSize: 200_000 })).toBeUndefined();
    expect(contextTokensFromStatusLine({ contextUsedPct: 10 })).toBeUndefined();
    expect(contextTokensFromStatusLine(undefined)).toBeUndefined();
  });
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
    claudeSessionStore.setSpawnMeta(sid, { label: 'my agent', isWakeTarget: true });
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

  it('folds daemon-owned Codex subagent rows and derives background ambient state', () => {
    const sid = managedSession();
    claudeSessionStore.applyManagedMode(sid, 'input', {
      provider: 'codex',
      pending: null,
      backgroundTasks: 1,
      subagents: [
        {
          id: 'child-1',
          type: 'codex',
          status: 'running',
          startedAt: 1000,
          description: 'inspect',
          toolUseId: 'call-1',
        },
      ],
    });
    const snap = claudeSessionStore.getSnapshot(sid);
    expect(snap?.ambientState).toBe('background');
    expect(snap?.subagents).toEqual([
      {
        id: 'child-1',
        type: 'codex',
        status: 'running',
        startedAt: 1000,
        description: 'inspect',
        toolUseId: 'call-1',
      },
    ]);
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
    // The ownership check no longer lives at applyManagedMode's call site — it
    // is the `PendingSlot(session, 'daemon')` inside applyManagedPending, so
    // this passes because a daemon write to a hook-owned slot is refused, not
    // because a condition upstream remembered to ask.
    const sid = uniqueId();
    hook(sid, 'UserPromptSubmit');
    claudeSessionStore.applyManagedMode(sid, 'approval', {
      provider: 'claude',
      pending: { kind: 'approval', tool: 'Bash', raw: { tool_input: { command: 'ls' } } },
    });
    expect(claudeSessionStore.getSnapshot(sid)?.pendingApproval).toBeNull();
  });
});

describe('the pending slot of a REMOTE row belongs to the peer, not to any local feed', () => {
  // A row that arrived from a peer hub is a mirror: the request is parked on
  // the OTHER machine, and nothing here can answer it. Ownership used to be
  // read off provider/transport alone, so a local daemon frame that happened to
  // carry a remote session's id walked straight onto the peer's card — the same
  // shape as every freeze this invariant exists to stop, one feed further out.
  const remoteApproval = (over: Record<string, unknown> = {}) => ({
    sessionId: 'sess-remote-1',
    cwd: '/peer/proj',
    status: 'active',
    ambientState: 'waiting_approval',
    pendingApproval: { toolName: 'Read', toolInput: { file_path: '/peer/x' }, timestamp: 7 },
    ...over,
  });

  it('a local daemon frame cannot RESOLVE the card a peer is holding open', () => {
    claudeSessionStore.upsertRemoteSession(
      'laptop',
      remoteApproval({ provider: 'codex' }) as never,
    );
    expect(claudeSessionStore.getSnapshot('sess-remote-1')?.pendingApproval?.toolName).toBe('Read');

    // claudemon on THIS machine says nothing is pending. It is not talking
    // about the peer's session, and must not empty its slot.
    claudeSessionStore.applyManagedMode('sess-remote-1', 'responding', {
      provider: 'codex',
      pending: null,
    });
    expect(claudeSessionStore.getSnapshot('sess-remote-1')?.pendingApproval?.toolName).toBe('Read');
  });

  it('a local daemon frame cannot PARK a card onto a peer-owned row either', () => {
    claudeSessionStore.upsertRemoteSession(
      'laptop',
      remoteApproval({ provider: 'codex', pendingApproval: null, ambientState: 'idle' }) as never,
    );
    claudeSessionStore.applyManagedMode('sess-remote-1', 'approval', {
      provider: 'codex',
      pending: { kind: 'approval', tool: 'Bash', raw: { tool_input: { command: 'ls' } } },
    });
    // An Approve button here would post the decision to the local daemon, which
    // has never heard of the request.
    expect(claudeSessionStore.getSnapshot('sess-remote-1')?.pendingApproval).toBeNull();
  });

  it('passes the remote owner model status and provenance through unchanged', () => {
    const sessionId = uniqueId();
    const remote = (costUSD: number) =>
      remoteApproval({
        sessionId,
        pendingApproval: null,
        ambientState: 'idle',
        requestedSelection: { model: 'opus', contextWindow: 1_000_000 },
        resolvedContextWindow: 1_000_000,
        statusLine: {
          modelDisplay: 'Claude 3.5 Haiku',
          contextWindowSize: 200_000,
          contextUsedPct: 25,
          costUSD,
        },
      });

    claudeSessionStore.upsertRemoteSession('laptop', remote(4) as never);
    expect(claudeSessionStore.getSnapshot(sessionId)?.statusLine).toEqual(remote(4).statusLine);

    // An update carrying the same owner provenance must not mint (or extend) a
    // local desktop fence either.
    claudeSessionStore.upsertRemoteSession('laptop', remote(5) as never);
    expect(claudeSessionStore.getSnapshot(sessionId)?.statusLine).toEqual(remote(5).statusLine);
  });

  it('passes sparse federation model telemetry and provenance without a local fence', () => {
    const sessionId = uniqueId();
    const sparse = (costUSD: number) => ({
      sessionId,
      sparse: true,
      status: 'active',
      ambientState: 'idle',
      requestedSelection: { model: 'opus', contextWindow: 1_000_000 },
      resolvedContextWindow: 1_000_000,
      statusLine: {
        modelDisplay: 'Claude 3.5 Haiku',
        contextWindowSize: 200_000,
        contextUsedPct: 25,
        costUSD,
      },
    });

    claudeSessionStore.upsertRemoteSession('headless-peer', sparse(4) as never);
    expect(claudeSessionStore.getSnapshot(sessionId)).toMatchObject({
      requestedSelection: { model: 'opus', contextWindow: 1_000_000 },
      resolvedContextWindow: 1_000_000,
      statusLine: sparse(4).statusLine,
    });

    claudeSessionStore.upsertRemoteSession('headless-peer', sparse(5) as never);
    expect(claudeSessionStore.getSnapshot(sessionId)?.statusLine).toEqual(sparse(5).statusLine);
  });

  it('a re-sent identical card keeps its first timestamp (the dock hides on dismissal)', () => {
    // Peers re-publish the same parked request on unrelated state changes. The
    // full-snapshot path used to take the wire card verbatim, so a re-stamped
    // resend resurrected a card the user had already dismissed.
    claudeSessionStore.upsertRemoteSession('laptop', remoteApproval() as never);
    expect(claudeSessionStore.getSnapshot('sess-remote-1')?.pendingApproval?.timestamp).toBe(7);
    claudeSessionStore.upsertRemoteSession(
      'laptop',
      remoteApproval({
        pendingApproval: { toolName: 'Read', toolInput: { file_path: '/peer/x' }, timestamp: 999 },
      }) as never,
    );
    expect(claudeSessionStore.getSnapshot('sess-remote-1')?.pendingApproval?.timestamp).toBe(7);
    // A genuinely different request is a new card and does get stamped.
    claudeSessionStore.upsertRemoteSession(
      'laptop',
      remoteApproval({
        pendingApproval: { toolName: 'Bash', toolInput: { command: 'rm -rf /' }, timestamp: 999 },
      }) as never,
    );
    expect(claudeSessionStore.getSnapshot('sess-remote-1')?.pendingApproval?.timestamp).toBe(999);
  });

  // applyRemoteStateChange is the LIGHT path: a peer hub's Go claudemon bridge
  // publishes `agent.state_changed` even where no desktop runs to send full
  // snapshots. Its `input` arm mapped straight to 'idle' with no background
  // check, while applyManagedMode two hundred lines above asked the question —
  // so a mirrored row went "done" while the workflow, subagent or background
  // shell it started carried on.
  it('a peer going ready-for-input does not read idle while its work runs', () => {
    claudeSessionStore.upsertRemoteSession(
      'laptop',
      remoteApproval({
        sessionId: 'sess-remote-bg',
        pendingApproval: null,
        ambientState: 'streaming',
        backgroundTasks: 3,
      }) as never,
    );
    claudeSessionStore.applyRemoteStateChange('laptop', 'sess-remote-bg', 'input');
    expect(claudeSessionStore.getSnapshot('sess-remote-bg')?.ambientState).toBe('background');
  });

  it('a peer with nothing left running does read idle', () => {
    claudeSessionStore.upsertRemoteSession(
      'laptop',
      remoteApproval({
        sessionId: 'sess-remote-quiet',
        pendingApproval: null,
        ambientState: 'streaming',
      }) as never,
    );
    claudeSessionStore.applyRemoteStateChange('laptop', 'sess-remote-quiet', 'input');
    expect(claudeSessionStore.getSnapshot('sess-remote-quiet')?.ambientState).toBe('idle');
  });

  it('an answer the peer ACCEPTED still clears the picker — that write is not a guess', () => {
    // clearPendingQuestions is the one ungated door (acknowledgeAnswer): the
    // user answered and `hub:<peer>/claude.answer` returned, so this resolves
    // the exact request the mirror shows, ahead of the peer's next snapshot.
    claudeSessionStore.upsertRemoteSession(
      'laptop',
      remoteApproval({
        pendingApproval: null,
        ambientState: 'waiting_input',
        pendingQuestions: [{ question: 'Which db?', options: [{ label: 'pg' }] }],
      }) as never,
    );
    expect(claudeSessionStore.getSnapshot('sess-remote-1')?.pendingQuestions).toHaveLength(1);
    claudeSessionStore.clearPendingQuestions('sess-remote-1');
    expect(claudeSessionStore.getSnapshot('sess-remote-1')?.pendingQuestions).toBeNull();
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

// ── The routing block (agents.spawn role / capability / decisionId) ─────────
//
// The desktop used to take these three off the spawn wire, echo them in the
// spawn result and forget them. That is not a tidiness problem: `respawn_with`
// inherits role + capability FROM THE SNAPSHOT (cmd/mcp/respawn.go), so a
// desktop-hosted reviewer lost its `fresh` marking on redispatch, silently. The
// shape here is the brain's — one `routing` block, only the keys that were
// given, and NO block at all for an unrouted spawn (cmd/brain/enrich.go).
describe('setSpawnMeta records the routing labels a dispatch arrived with', () => {
  it('returns them on the snapshot as a routing block, for a session born after the spawn', () => {
    const sid = uniqueId();
    claudeSessionStore.setSpawnMeta(sid, {
      routing: { role: 'reviewer', capability: 'frontier', decisionId: 'dec-123' },
    });
    hook(sid, 'SessionStart');

    expect(claudeSessionStore.getSnapshot(sid)?.routing).toEqual({
      role: 'reviewer',
      capability: 'frontier',
      decisionId: 'dec-123',
    });
  });

  it('invents no routing block for a spawn that named none of them', () => {
    const sid = uniqueId();
    claudeSessionStore.setSpawnMeta(sid, { label: 'plain worker' });
    hook(sid, 'SessionStart');

    const snap = claudeSessionStore.getSnapshot(sid);
    expect(snap?.label).toBe('plain worker');
    // Absent, not `{}` — an unrouted row keeps its exact shape, and a caller
    // reading `routing.role` must get "nothing was routed", not an empty object
    // that looks like a routed session missing a field.
    expect(snap?.routing).toBeUndefined();
    // And nothing survives serialization either — what a bus caller reads over
    // sessions.snapshot has no `routing` key at all, exactly as the brain's
    // unenriched row does.
    expect(JSON.parse(JSON.stringify(snap))).not.toHaveProperty('routing');
  });

  it('keeps only the labels that were given', () => {
    const sid = uniqueId();
    claudeSessionStore.setSpawnMeta(sid, { routing: { role: 'scout' } });
    hook(sid, 'SessionStart');

    expect(claudeSessionStore.getSnapshot(sid)?.routing).toEqual({ role: 'scout' });
  });

  // A restart re-spawns onto the SAME id (resumeSessionId pins it), so the row
  // is already live and createSession never runs again. Without an in-place
  // refresh the snapshot would keep answering with the previous life's role
  // while the new life runs under a different one.
  it('refreshes the block in place when a respawn lands on a live id', () => {
    const sid = uniqueId();
    claudeSessionStore.setSpawnMeta(sid, { routing: { role: 'implementer' } });
    hook(sid, 'SessionStart');
    expect(claudeSessionStore.getSnapshot(sid)?.routing).toEqual({ role: 'implementer' });

    claudeSessionStore.setSpawnMeta(sid, { routing: { role: 'reviewer', capability: 'frontier' } });

    expect(claudeSessionStore.getSnapshot(sid)?.routing).toEqual({
      role: 'reviewer',
      capability: 'frontier',
    });
  });
});

// ── The daemon-owned canonical selection slice ───────────────────────────────
//
// claudemon owns `requested_selection` / `resolved_context_window`. This store
// receives them (already mapped to camelCase by claudemonEventBridge), holds
// them verbatim and forwards them; it never re-derives them from
// `settings.model` or from the window resolver, which is the second,
// disagreeing answer the slice exists to retire.
describe('applyManagedMode carries the daemon selection slice', () => {
  function managedSession(): string {
    const sid = uniqueId();
    claudeSessionStore.applyConversationDelta({
      session_id: sid,
      seq: 0,
      items: [],
      reset: false,
    } as never);
    return sid;
  }

  const SLICE = {
    requestedSelection: { model: 'claude-opus-5', contextWindow: 1_000_000 },
    resolvedContextWindow: 1_000_000,
  };

  it('stores both fields exactly as the daemon sent them', () => {
    const sid = managedSession();
    claudeSessionStore.applyManagedMode(sid, 'responding', {
      provider: 'claude',
      selection: SLICE,
    });
    const snap = claudeSessionStore.getSnapshot(sid)!;
    expect(snap.requestedSelection).toEqual(SLICE.requestedSelection);
    expect(snap.resolvedContextWindow).toBe(1_000_000);
  });

  // The daemon broadcasts a Managed frame on every mode transition, and most of
  // them restate nothing about the model. A plain assignment would erase the
  // owner's fact on the very next tick.
  it('keeps them when a later frame says nothing about either', () => {
    const sid = managedSession();
    claudeSessionStore.applyManagedMode(sid, 'responding', { selection: SLICE });
    claudeSessionStore.applyManagedMode(sid, 'input', { selection: {} });
    const snap = claudeSessionStore.getSnapshot(sid)!;
    expect(snap.requestedSelection).toEqual(SLICE.requestedSelection);
    expect(snap.resolvedContextWindow).toBe(1_000_000);
  });

  it('lets a live model switch move the window the daemon reports', () => {
    const sid = managedSession();
    claudeSessionStore.applyManagedMode(sid, 'responding', { selection: SLICE });
    claudeSessionStore.applyManagedMode(sid, 'input', {
      selection: {
        requestedSelection: { model: 'claude-sonnet-5', contextWindow: 200_000 },
        resolvedContextWindow: 200_000,
      },
    });
    const snap = claudeSessionStore.getSnapshot(sid)!;
    expect(snap.requestedSelection).toEqual({
      model: 'claude-sonnet-5',
      contextWindow: 200_000,
    });
    expect(snap.resolvedContextWindow).toBe(200_000);
  });

  it('fences repeated stale 200K frames until the accepted 1M epoch is confirmed', () => {
    const sid = managedSession();
    claudeSessionStore.applyManagedMode(sid, 'responding', {
      provider: 'claude',
      selection: {
        requestedSelection: { model: 'sonnet', contextWindow: 200_000 },
        resolvedContextWindow: 200_000,
      },
    });
    claudeSessionStore.applyStatusLine(sid, {
      modelDisplay: 'Sonnet',
      contextWindowSize: 200_000,
      contextUsedPct: 75,
    });

    claudeSessionStore.noteRequestedModelSelection(
      sid,
      { model: 'opus', contextWindow: 1_000_000 },
      'opus[1m]',
    );
    for (const costUSD of [2, 3]) {
      claudeSessionStore.applyStatusLine(sid, {
        modelDisplay: 'Sonnet',
        contextWindowSize: 200_000,
        contextUsedPct: 80,
        costUSD,
      });
      const stale = claudeSessionStore.getSnapshot(sid)!;
      expect(stale.requestedSelection).toEqual({ model: 'opus', contextWindow: 1_000_000 });
      expect(stale.statusLine).toMatchObject({ costUSD });
      expect(stale.statusLine?.modelDisplay).toBeUndefined();
      expect(stale.statusLine?.contextWindowSize).toBeUndefined();
      expect(stale.statusLine?.contextUsedPct).toBeUndefined();
    }

    claudeSessionStore.applyStatusLine(sid, {
      modelDisplay: 'Opus',
      contextWindowSize: 1_000_000,
      contextUsedPct: 10,
    });
    expect(claudeSessionStore.getSnapshot(sid)?.statusLine?.contextWindowSize).toBe(1_000_000);

    claudeSessionStore.applyStatusLine(sid, {
      modelDisplay: 'Sonnet',
      contextWindowSize: 200_000,
      contextUsedPct: 5,
    });
    expect(claudeSessionStore.getSnapshot(sid)?.statusLine?.contextWindowSize).toBe(
      200_000,
      'truthful later provider changes remain visible after confirmation',
    );
  });

  it('releases a hydrated fence after three unmatchable Haiku-style frames', () => {
    const sid = managedSession();
    // First owner slice after this process starts is the desktop hydration
    // shape: conservatively fenced, but with the same finite bound as a local
    // accepted switch.
    claudeSessionStore.applyManagedMode(sid, 'responding', {
      provider: 'claude',
      selection: {
        requestedSelection: { model: 'opus', contextWindow: 1_000_000 },
        resolvedContextWindow: 1_000_000,
      },
    });

    for (const costUSD of [1, 2, 3]) {
      claudeSessionStore.applyStatusLine(sid, {
        modelDisplay: 'Claude 3.5 Haiku',
        contextWindowSize: 200_000,
        contextUsedPct: 20,
        costUSD,
      });
      expect(claudeSessionStore.getSnapshot(sid)?.statusLine).toMatchObject({
        modelDisplay: undefined,
        contextWindowSize: undefined,
        contextUsedPct: undefined,
        costUSD,
      });
    }

    claudeSessionStore.applyStatusLine(sid, {
      modelDisplay: 'Claude 3.5 Haiku',
      contextWindowSize: 200_000,
      contextUsedPct: 25,
    });
    expect(claudeSessionStore.getSnapshot(sid)?.statusLine).toMatchObject({
      modelDisplay: 'Claude 3.5 Haiku',
      contextWindowSize: 200_000,
      contextUsedPct: 25,
    });
  });

  // A session the daemon has said nothing about carries no key at all — absent
  // is a different fact from any number, and every readout already hides its
  // meter on it rather than drawing one against a guess.
  it('leaves a row that was never told about the slice without the keys', () => {
    const sid = managedSession();
    claudeSessionStore.applyManagedMode(sid, 'responding', { provider: 'codex' });
    const snap = claudeSessionStore.getSnapshot(sid)!;
    expect('requestedSelection' in snap).toBe(false);
    expect('resolvedContextWindow' in snap).toBe(false);
  });

  // Occupancy is a DISPLAY question (busContextLimit), never a storage one: the
  // owner's number is held as given even while the raw status pair disagrees.
  it('never rewrites the stored window on occupancy', () => {
    const sid = managedSession();
    claudeSessionStore.applyManagedMode(sid, 'responding', {
      selection: { resolvedContextWindow: 200_000 },
    });
    claudeSessionStore.applyStatusLine(sid, { contextWindowSize: 200_000, contextUsedPct: 100 });
    claudeSessionStore.applyManagedMode(sid, 'input', { selection: {} });
    expect(claudeSessionStore.getSnapshot(sid)!.resolvedContextWindow).toBe(200_000);
  });
});
