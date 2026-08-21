/**
 * The worker-finished wake (FLEET_MANAGER_SPIKE.md gap #2): a finish routes
 * ONLY to the worker's parent (unlike blocks, which broadcast to every
 * supervisor), coalesces a burst into one message, and carries a capped
 * excerpt of the worker's last reply so the manager can act without a fetch.
 */
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';

const message = vi.fn().mockResolvedValue(undefined);
vi.mock('./claudemonSessionClient', () => ({
  claudemonSessionClient: { message: (...args: unknown[]) => message(...args) },
}));

import { supervisorNudge } from './supervisorNudge';
import type { ClaudeSessionState } from './claudeSessionStore';

const worker = (over: Partial<ClaudeSessionState> = {}): ClaudeSessionState =>
  ({
    sessionId: 'w1',
    cwd: '/home/u/Work/alpha',
    label: 'alpha: fix tests',
    ...over,
  }) as ClaudeSessionState;

beforeEach(() => {
  vi.useFakeTimers();
  message.mockClear();
});
afterEach(() => vi.useRealTimers());

describe('supervisorNudge.onFinished', () => {
  it('wakes the parent with label, session ref, and a capped last-reply excerpt', async () => {
    supervisorNudge.onFinished(worker(), 'mgr', 'All 42 tests pass.\nDone.');
    await vi.advanceTimersByTimeAsync(2000);
    expect(message).toHaveBeenCalledTimes(1);
    const [target, text] = message.mock.calls[0] as [string, string];
    expect(target).toBe('mgr');
    expect(text).toContain('[fleet] Worker finished');
    expect(text).toContain('alpha: fix tests');
    expect(text).toContain('session:w1');
    expect(text).toContain('All 42 tests pass. Done.'); // flattened, not truncated
    expect(text).toContain('brief.md');
  });

  it('coalesces a burst of finishes into ONE wake per parent', async () => {
    supervisorNudge.onFinished(worker(), 'mgr', 'a');
    supervisorNudge.onFinished(worker({ sessionId: 'w2', label: 'beta: docs' }), 'mgr', 'b');
    await vi.advanceTimersByTimeAsync(2000);
    expect(message).toHaveBeenCalledTimes(1);
    const [, text] = message.mock.calls[0] as [string, string];
    expect(text).toContain('session:w1');
    expect(text).toContain('session:w2');
  });

  it('never wakes a session about its own finish, and caps a runaway reply', async () => {
    supervisorNudge.onFinished(worker({ sessionId: 'mgr' }), 'mgr', 'x');
    await vi.advanceTimersByTimeAsync(2000);
    expect(message).not.toHaveBeenCalled();

    supervisorNudge.onFinished(worker(), 'mgr', 'y'.repeat(2000));
    await vi.advanceTimersByTimeAsync(2000);
    const [, text] = message.mock.calls[0] as [string, string];
    expect(text.length).toBeLessThan(1200);
    expect(text).toContain('…');
  });
});

describe('supervisorNudge.sweepMissedFinishes (dropped-wake backstop)', () => {
  const GRACE = 3 * 60_000;
  const mgr = (over = {}) =>
    ({
      sessionId: 'mgr',
      isSupervisor: true,
      status: 'active',
      ambientState: 'idle',
      lastActivity: 1_000,
      ...over,
    }) as any;
  const child = (over = {}) =>
    ({
      sessionId: 'w1',
      parentSessionId: 'mgr',
      cwd: '/home/u/Work/alpha',
      label: 'alpha: fix tests',
      ambientState: 'idle',
      lastActivity: 10_000,
      ...over,
    }) as any;

  it('re-nudges an idle manager whose child finished after it, past the grace window', () => {
    const now = 10_000 + GRACE + 1;
    supervisorNudge.sweepMissedFinishes([mgr(), child()], now);
    expect(message).toHaveBeenCalledTimes(1);
    const [target, text] = message.mock.calls[0] as [string, string];
    expect(target).toBe('mgr');
    expect(text).toContain('Catch-up');
    expect(text).toContain('session:w1');
  });

  it('does NOT nudge inside the grace window (a normal wake may still be in flight)', () => {
    supervisorNudge.sweepMissedFinishes([mgr(), child()], 10_000 + GRACE - 1);
    expect(message).not.toHaveBeenCalled();
  });

  it('does NOT nudge when the manager already acted after the finish (implicit dedup)', () => {
    // Manager's lastActivity is newer than the child's finish → it handled it.
    const now = 10_000 + GRACE + 1;
    supervisorNudge.sweepMissedFinishes([mgr({ lastActivity: 20_000 }), child()], now);
    expect(message).not.toHaveBeenCalled();
  });

  it('ignores a still-working child (only finished/idle-or-ended children count)', () => {
    const now = 10_000 + GRACE + 1;
    supervisorNudge.sweepMissedFinishes([mgr(), child({ ambientState: 'thinking' })], now);
    expect(message).not.toHaveBeenCalled();
  });

  it('nudges for an ENDED (dead) child too', () => {
    const now = 10_000 + GRACE + 1;
    supervisorNudge.sweepMissedFinishes(
      [mgr(), child({ ambientState: 'streaming', status: 'ended' })],
      now,
    );
    expect(message).toHaveBeenCalledTimes(1);
  });

  it('never fires when the manager itself is busy', () => {
    const now = 10_000 + GRACE + 1;
    supervisorNudge.sweepMissedFinishes([mgr({ ambientState: 'thinking' }), child()], now);
    expect(message).not.toHaveBeenCalled();
  });
});
