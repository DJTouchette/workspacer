/**
 * The worker-finished wake (FLEET_MANAGER_SPIKE.md gap #2): a finish routes
 * ONLY to the worker's parent (unlike blocks, which broadcast to every
 * supervisor), coalesces a burst into one message, and carries the worker's
 * COMPLETE final message so the manager can act without a fetch. Gated so it
 * only fires on a GENUINE finish: no wake before the worker has received its
 * task turn, none for a worker that resumed working during the coalesce
 * window, and a killed worker's wake says stopped/killed, not a bare finish.
 */
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';

const message = vi.fn().mockResolvedValue(undefined);
vi.mock('./claudemonSessionClient', () => ({
  claudemonSessionClient: { message: (...args: unknown[]) => message(...args) },
}));

import { supervisorNudge } from './supervisorNudge';
import type { ClaudeSessionState, ConversationTurn } from './claudeSessionStore';
import { parseFleetMessage } from '../shared/fleetMessages';

const turns = (...t: Array<[ConversationTurn['role'], string]>): ConversationTurn[] =>
  t.map(([role, content]) => ({ role, content }));

const worker = (over: Partial<ClaudeSessionState> = {}): ClaudeSessionState =>
  ({
    sessionId: 'w1',
    cwd: '/home/u/Work/alpha',
    label: 'alpha: fix tests',
    status: 'active',
    ambientState: 'idle',
    conversation: turns(['user', 'fix the tests'], ['assistant', 'All 42 tests pass.\nDone.']),
    ...over,
  }) as ClaudeSessionState;

beforeEach(() => {
  vi.useFakeTimers();
  message.mockClear();
});
afterEach(() => vi.useRealTimers());

describe('supervisorNudge.onFinished', () => {
  it('wakes the parent with label, session ref, excerpt AND the complete final message', async () => {
    supervisorNudge.onFinished(worker(), 'mgr', 'All 42 tests pass.\nDone.');
    await vi.advanceTimersByTimeAsync(2000);
    expect(message).toHaveBeenCalledTimes(1);
    const [target, text] = message.mock.calls[0] as [string, string];
    expect(target).toBe('mgr');
    expect(text).toContain('[fleet] Worker finished');
    expect(text).toContain('alpha: fix tests');
    expect(text).toContain('session:w1');
    expect(text).toContain('All 42 tests pass. Done.'); // the single-line bullet excerpt
    // The COMPLETE message rides in the wake, newlines intact — the manager
    // never needs get_conversation just to read the report.
    expect(text).toContain('Full final message — alpha: fix tests (session:w1):');
    expect(text).toContain('All 42 tests pass.\nDone.');
    expect(text).toContain('brief.md');
    // The wake must stay recognizable to the GUI's card renderer.
    expect(parseFleetMessage(text)).toEqual({
      kind: 'worker-finished',
      entries: [
        {
          label: 'alpha: fix tests',
          sessionId: 'w1',
          cwd: '/home/u/Work/alpha',
          lastReply: 'All 42 tests pass. Done.',
        },
      ],
    });
  });

  it('re-reads the reply at delivery, so a final message landing late still rides the wake', async () => {
    const w = worker({ conversation: turns(['user', 'fix the tests']) });
    supervisorNudge.onFinished(w, 'mgr', '');
    // The transcript tailer lands the final assistant message during the
    // coalesce window (claudemon keeps tailing briefly after Stop).
    w.conversation.push({ role: 'assistant', content: 'Late-landing report.\nAll green.' });
    await vi.advanceTimersByTimeAsync(2000);
    const [, text] = message.mock.calls[0] as [string, string];
    expect(text).toContain('Late-landing report. All green.');
    expect(text).toContain('Late-landing report.\nAll green.');
  });

  it('does NOT fire before the worker has received its task turn (boot idle)', async () => {
    supervisorNudge.onFinished(worker({ conversation: [] }), 'mgr', '');
    await vi.advanceTimersByTimeAsync(2000);
    expect(message).not.toHaveBeenCalled();
  });

  it('does NOT fire for a worker that resumed working during the coalesce window (mid-stream blip)', async () => {
    const w = worker();
    supervisorNudge.onFinished(w, 'mgr', 'half-done partial output');
    w.ambientState = 'streaming'; // the idle was a blip; the turn is still going
    await vi.advanceTimersByTimeAsync(2000);
    expect(message).not.toHaveBeenCalled();
    // The real finish later fires a fresh edge and wakes normally.
    w.ambientState = 'idle';
    supervisorNudge.onFinished(w, 'mgr', 'now actually done');
    await vi.advanceTimersByTimeAsync(2000);
    expect(message).toHaveBeenCalledTimes(1);
  });

  it('says stopped/killed for a worker whose session ENDED instead of idling', async () => {
    const w = worker({
      status: 'ended',
      conversation: turns(['user', 'fix the tests'], ['assistant', 'was still working on it']),
    });
    supervisorNudge.onFinished(w, 'mgr', 'was still working on it');
    await vi.advanceTimersByTimeAsync(2000);
    expect(message).toHaveBeenCalledTimes(1);
    const [, text] = message.mock.calls[0] as [string, string];
    expect(text).toContain('stopped/killed');
    expect(parseFleetMessage(text)?.entries[0]).toMatchObject({ sessionId: 'w1', stopped: true });
  });

  it('a killed worker that never got its task stays silent too', async () => {
    supervisorNudge.onFinished(worker({ status: 'ended', conversation: [] }), 'mgr', '');
    await vi.advanceTimersByTimeAsync(2000);
    expect(message).not.toHaveBeenCalled();
  });

  it('coalesces a burst of finishes into ONE wake per parent, dropping only the unfinished', async () => {
    const still = worker({ sessionId: 'w3', label: 'gamma: wip' });
    supervisorNudge.onFinished(worker(), 'mgr', 'a');
    supervisorNudge.onFinished(worker({ sessionId: 'w2', label: 'beta: docs' }), 'mgr', 'b');
    supervisorNudge.onFinished(still, 'mgr', 'c');
    still.ambientState = 'thinking';
    await vi.advanceTimersByTimeAsync(2000);
    expect(message).toHaveBeenCalledTimes(1);
    const [, text] = message.mock.calls[0] as [string, string];
    expect(text).toContain('session:w1');
    expect(text).toContain('session:w2');
    expect(text).not.toContain('session:w3');
  });

  it('never wakes a session about its own finish, and truncates a huge reply with an explicit note', async () => {
    supervisorNudge.onFinished(worker({ sessionId: 'mgr' }), 'mgr', 'x');
    await vi.advanceTimersByTimeAsync(2000);
    expect(message).not.toHaveBeenCalled();

    const huge = 'line\n'.repeat(20_000); // 100k chars, multi-line so the excerpt is lossy
    supervisorNudge.onFinished(worker({ conversation: turns(['user', 't'], ['assistant', huge]) }), 'mgr', huge);
    await vi.advanceTimersByTimeAsync(2000);
    const [, text] = message.mock.calls[0] as [string, string];
    expect(text).toContain('…'); // the bullet excerpt is capped
    expect(text).toContain('[truncated: showing the first');
    expect(text.length).toBeLessThan(40_000); // generous, but bounded
  });
});

describe('supervisorNudge.onBlock', () => {
  it('broadcasts a parseable [supervisor] wake, never to the blocked agent itself', async () => {
    supervisorNudge.onBlock(worker(), 'approval', ['sup', 'w1']);
    await vi.advanceTimersByTimeAsync(2000);
    expect(message).toHaveBeenCalledTimes(1);
    const [target, text] = message.mock.calls[0] as [string, string];
    expect(target).toBe('sup');
    expect(text).toContain('[supervisor]');
    expect(text).toContain('/supervise');
    expect(parseFleetMessage(text)).toEqual({
      kind: 'blocked',
      entries: [{ label: 'alpha: fix tests', sessionId: 'w1', blockedOn: 'approval' }],
    });
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
    expect(parseFleetMessage(text)?.kind).toBe('catch-up');
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

  it('nudges for an ENDED (dead) child too, marking it stopped/killed', () => {
    const now = 10_000 + GRACE + 1;
    supervisorNudge.sweepMissedFinishes(
      [mgr(), child({ ambientState: 'streaming', status: 'ended' })],
      now,
    );
    expect(message).toHaveBeenCalledTimes(1);
    const [, text] = message.mock.calls[0] as [string, string];
    expect(text).toContain('stopped/killed');
    expect(parseFleetMessage(text)?.entries[0]).toMatchObject({ sessionId: 'w1', stopped: true });
  });

  it('ignores a child that never received its task turn (same boot-idle gate as the wake)', () => {
    const now = 10_000 + GRACE + 1;
    supervisorNudge.sweepMissedFinishes([mgr(), child({ conversation: [] })], now);
    expect(message).not.toHaveBeenCalled();
    // With the task turn present, the same child IS caught up.
    supervisorNudge.sweepMissedFinishes(
      [mgr(), child({ conversation: [{ role: 'user', content: 'go' }] })],
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
