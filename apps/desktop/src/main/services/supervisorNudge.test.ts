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
