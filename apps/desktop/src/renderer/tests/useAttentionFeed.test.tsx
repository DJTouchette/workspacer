/**
 * Regressions in useAttentionFeed's time handling:
 *  #7  `stuck` items never surface because `now` only advances while something
 *      is snoozed — with an open-but-unanswered question (nothing snoozed) the
 *      clock is frozen and `now - since` can never cross STUCK_MS.
 *  #12 Expired snooze entries are never pruned, so the 5s ticker keeps running
 *      forever after any snooze even once it has expired.
 */
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { renderHook, act } from '@testing-library/react';
import { useAttentionFeed } from '../src/hooks/useAttentionFeed';

const T0 = 1_700_000_000_000;
const STUCK_MS = 5 * 60_000;

function agent(id: string, sessionId: string) {
  return { id, name: id, cwd: '/x', sessionId, global: false, activeTabId: '', tabs: [] } as any;
}

beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(T0);
});
afterEach(() => {
  vi.useRealTimers();
});

describe('useAttentionFeed — stuck detection (#7)', () => {
  it('surfaces a stuck item once an unanswered question ages past STUCK_MS', async () => {
    const snapshots = {
      s1: {
        ambientState: 'waiting_input',
        lastActivity: T0 - 1000, // asked 1s ago — not yet stuck
        pendingQuestions: [{ question: 'Pick a branch?', header: 'Question' }],
      } as any,
    };
    const { result } = renderHook(() => useAttentionFeed(snapshots, [agent('a1', 's1')]));

    // Initially the question is fresh — no stuck card yet.
    expect(result.current.items.some((it) => it.kind === 'stuck')).toBe(false);

    // Let real time pass well beyond the stuck threshold.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(STUCK_MS + 10_000);
    });

    expect(result.current.items.some((it) => it.kind === 'stuck')).toBe(true);
  });
});

describe('useAttentionFeed — question createdAt', () => {
  it('stamps a question item with the arrival time, not the current clock', () => {
    const asked = T0 - 600_000; // question arrived 10 minutes ago
    const snapshots = {
      s1: {
        ambientState: 'waiting_input',
        lastActivity: asked,
        pendingQuestions: [{ question: 'Pick a branch?', header: 'Question' }],
      } as any,
    };
    const { result } = renderHook(() => useAttentionFeed(snapshots, [agent('a1', 's1')]));

    const q = result.current.items.find((it) => it.kind === 'question');
    expect(q).toBeDefined();
    // Must reflect when the question arrived (like the co-present stuck item),
    // not Date.now() — otherwise its age always renders as "now".
    expect(q!.createdAt).toBe(asked);
  });
});

describe('useAttentionFeed — snooze pruning (#12)', () => {
  it('stops the ticker once the only snooze has expired (entry pruned)', async () => {
    const snapshots = {
      s1: {
        ambientState: 'waiting_approval',
        lastActivity: T0,
        pendingApproval: { toolName: 'Bash', toolInput: { command: 'ls' }, timestamp: T0 },
      } as any,
    };
    const { result } = renderHook(() => useAttentionFeed(snapshots, [agent('a1', 's1')]));

    const sig = result.current.items[0].signature;
    act(() => result.current.snooze(sig, 1)); // snooze 1 minute

    // Ticker is armed while a snooze is pending.
    expect(vi.getTimerCount()).toBeGreaterThan(0);

    // Advance past the snooze expiry plus several ticks.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(90_000);
    });

    // The expired entry must be pruned and, with no other time-based work, the
    // ticker must stop instead of spinning forever.
    expect(vi.getTimerCount()).toBe(0);
    // And the item re-surfaces now that the snooze is over.
    expect(result.current.items.some((it) => it.signature === sig)).toBe(true);
  });
});
