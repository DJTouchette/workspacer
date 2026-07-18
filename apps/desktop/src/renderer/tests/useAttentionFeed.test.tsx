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
import { compactClaudeSnapshotForBackground } from '../src/lib/compactClaudeSnapshot';

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

describe('useAttentionFeed — bigdiff after background compaction', () => {
  it('surfaces a bigdiff card for one large rewrite even once its input has been compacted', () => {
    // A single ~200-line Edit: old_string + new_string whose JSON far exceeds
    // the 1000-char file-change compaction budget.
    const bigOld = Array.from({ length: 200 }, (_, i) => `old line ${i} ${'x'.repeat(20)}`).join(
      '\n',
    );
    const bigNew = Array.from({ length: 200 }, (_, i) => `new line ${i} ${'y'.repeat(20)}`).join(
      '\n',
    );
    const raw = {
      ambientState: 'idle',
      lastActivity: T0 - 1000,
      fileChanges: [
        {
          path: '/repo/src/big.ts',
          toolName: 'Edit',
          timestamp: T0 - 1000,
          input: { file_path: '/repo/src/big.ts', old_string: bigOld, new_string: bigNew },
        },
      ],
    } as any;

    // App.tsx stores the *background-compacted* snapshot into snapshotBySession,
    // which is exactly what reaches useAttentionFeed.
    const compacted = compactClaudeSnapshotForBackground(raw);
    const snapshots = { s1: compacted };

    const { result } = renderHook(() => useAttentionFeed(snapshots, [agent('a1', 's1')]));

    expect(result.current.items.some((it) => it.kind === 'bigdiff')).toBe(true);
  });
});
