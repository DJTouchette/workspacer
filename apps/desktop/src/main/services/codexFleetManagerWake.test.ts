/**
 * THE question this whole change turns on: does a Fleet Manager running on
 * CODEX actually receive a wake when one of its workers finishes? A manager
 * that dispatches and then never hears back is decorative, and — worse — looks
 * identical to a working one.
 *
 * So this drives the REAL claudeSessionStore and the REAL supervisorNudge over
 * the signal path a codex session actually uses. Codex fires NO Claude hooks:
 * its only working/idle signal is `applyManagedMode`, fed by claudemonEventBridge
 * from the daemon's `session.update` frames (codex's `turn/started` →
 * 'responding', `turn/completed` → 'input'). The one mocked boundary is
 * claudemonSessionClient.message — the wire the wake goes out on, which is
 * exactly what we are counting.
 *
 * Sibling of perTurnWake.repro.test.ts, which pins the same machinery over the
 * HOOK-driven (Claude) path.
 */
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';

const message = vi.fn().mockResolvedValue(undefined);
vi.mock('./claudemonSessionClient', () => ({
  claudemonSessionClient: { message: (...a: unknown[]) => message(...a) },
}));
vi.mock('electron', () => ({ BrowserWindow: class {} }));
vi.mock('./agentNotifier', () => ({ agentNotifier: { notifyOnTransition: vi.fn() } }));
vi.mock('./workflowWatcher', () => ({
  workflowWatcher: { attach: vi.fn(), detach: vi.fn(), poke: vi.fn() },
}));
vi.mock('./hubTelemetry', () => ({
  publishWorkflowRuns: vi.fn(),
  publishSnapshot: vi.fn(),
  forgetSession: vi.fn(),
}));
vi.mock('./claudemonDaemon', () => ({ CLAUDEMON_API_URL: 'http://127.0.0.1:0' }));
vi.mock('./sessionStore/analyticsWriter', () => ({ writeHistory: vi.fn() }));

import { claudeSessionStore } from './claudeSessionStore';
import { parseFleetMessage } from '../shared/fleetMessages';

let seq = 0;
const uid = (p: string): string => `${p}-codex-${++seq}`;

const convCursor = new Map<string, number>();
/** One conversation frame in the daemon's delta shape — codex echoes the user's
 *  prompt as a UserMessage item, which is what satisfies the wake's
 *  hasReceivedTask gate (a worker idling with no task never "finished"). */
function say(sessionId: string, role: 'user' | 'assistant', text: string): void {
  const n = (convCursor.get(sessionId) ?? 0) + 1;
  convCursor.set(sessionId, n);
  claudeSessionStore.applyConversationDelta({
    session_id: sessionId,
    seq: n,
    reset: false,
    items: [{ type: role === 'user' ? 'user_message' : 'assistant_text', text }] as never,
  });
}

/** The daemon's managed-mode frames for a codex session. */
function mode(sessionId: string, m: 'responding' | 'input' | 'approval'): void {
  claudeSessionStore.applyManagedMode(sessionId, m, { provider: 'codex', transport: 'stream' });
}

/** Register a session the way spawnManagedAgent does: spawn meta first (so the
 *  role/provider are on the card from birth), then the managed session row. */
function register(
  sessionId: string,
  meta: { label: string; isWakeTarget?: boolean; parentSessionId?: string },
): void {
  claudeSessionStore.setSpawnMeta(sessionId, { ...meta, provider: 'codex', transport: 'stream' });
  claudeSessionStore.ensureManagedSession(sessionId, '/proj');
}

beforeEach(() => {
  vi.useFakeTimers();
  message.mockClear();
});
afterEach(() => vi.useRealTimers());

describe('a codex Fleet Manager receives its workers’ finish wakes', () => {
  it('wakes the codex manager when a codex worker completes a turn', async () => {
    const mgr = uid('mgr');
    const child = uid('child');
    // `manager: true` on the spawn is what sets isWakeTarget — the flag the IPC
    // managed branch used to drop. Without it nudgeParentOnFinish bails and no
    // wake is ever routed here, which is the whole bug.
    register(mgr, { label: 'Fleet Manager', isWakeTarget: true });
    register(child, { label: 'alpha: fix tests', parentSessionId: mgr });

    // The dispatch: the manager sends the task, codex reports the turn started,
    // answers, and reports the turn complete.
    say(child, 'user', 'fix the failing tests');
    mode(child, 'responding');
    say(child, 'assistant', 'Fixed. All 42 pass.');
    mode(child, 'input');
    await vi.advanceTimersByTimeAsync(3000);

    expect(message).toHaveBeenCalledTimes(1);
    const [target, text] = message.mock.calls[0] as [string, string];
    expect(target).toBe(mgr); // …and it went to the MANAGER, not the worker
    const wake = parseFleetMessage(text);
    expect(wake?.kind).toBe('worker-finished');
    // The wake carries the worker's actual final message, so the manager can
    // report without fetching the conversation.
    expect(text).toContain('Fixed. All 42 pass.');
    expect(text).toContain(child);
  });

  it('routes a BLOCKED codex worker to the codex manager too', async () => {
    const mgr = uid('mgr');
    const child = uid('child');
    register(mgr, { label: 'Fleet Manager', isWakeTarget: true });
    register(child, { label: 'beta: build', parentSessionId: mgr });

    say(child, 'user', 'run the release build');
    mode(child, 'responding');
    mode(child, 'approval');
    // A block is debounced 20s (a prompt the user clears themselves must never
    // reach the manager), then coalesced — so wait past both.
    await vi.advanceTimersByTimeAsync(25_000);

    // A block broadcasts to EVERY live supervisor (unlike a finish, which goes
    // only to the worker's parent), so find this manager's copy rather than
    // assuming it is the first call.
    const mine = (message.mock.calls as Array<[string, string]>).find(([t]) => t === mgr);
    expect(mine).toBeDefined();
    expect(parseFleetMessage(mine![1])?.kind).toBe('blocked');
    expect(mine![1]).toContain(child);
  });

  it('does NOT wake a manager that was spawned without the manager flag', async () => {
    // The pre-fix shape, pinned so it cannot come back silently: the session is
    // there, the parent link is there, the worker finishes — and the manager
    // hears nothing, because isWakeTarget was never set.
    const mgr = uid('mgr');
    const child = uid('child');
    register(mgr, { label: 'Fleet Manager' }); // no isWakeTarget
    register(child, { label: 'gamma: docs', parentSessionId: mgr });

    say(child, 'user', 'update the docs');
    mode(child, 'responding');
    say(child, 'assistant', 'Done.');
    mode(child, 'input');
    await vi.advanceTimersByTimeAsync(3000);

    expect(message).not.toHaveBeenCalled();
  });
});
