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

/**
 * A working agent that stops producing anything is the case nobody notices: the
 * card says "thinking", the spinner spins, and nothing is wrong-looking. These
 * pin that it surfaces, and — just as important — that a genuinely busy agent
 * never does.
 */
describe('useAttentionFeed — a working agent that stops making progress', () => {
  const working = (over: any = {}) => ({
    s1: {
      ambientState: 'thinking',
      lastActivity: T0,
      conversation: [],
      activeToolCalls: [],
      completedToolCalls: [],
      totalToolCalls: 0,
      ...over,
    } as any,
  });

  it('surfaces a stalled agent once nothing has moved for long enough', async () => {
    const { result } = renderHook(() => useAttentionFeed(working(), [agent('a1', 's1')]));
    expect(result.current.items.some((it) => it.kind === 'stuck')).toBe(false);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(6 * 60_000);
    });

    const stalled = result.current.items.find((it) => it.kind === 'stuck');
    expect(stalled, 'a stalled card should have surfaced').toBeTruthy();
    expect(stalled!.title).toBe('Not moving');
    expect(stalled!.detail).toMatch(/nothing has changed for \d+m/);
  });

  it('keeps quiet while the agent is actually working', async () => {
    let snapshots = working({ totalToolCalls: 1 });
    const { result, rerender } = renderHook(({ s }) => useAttentionFeed(s, [agent('a1', 's1')]), {
      initialProps: { s: snapshots },
    });

    // Work advances every couple of minutes — well inside any threshold.
    for (let i = 2; i <= 12; i += 2) {
      await act(async () => {
        await vi.advanceTimersByTimeAsync(2 * 60_000);
      });
      snapshots = working({ totalToolCalls: i });
      rerender({ s: snapshots });
    }

    expect(
      result.current.items.some((it) => it.kind === 'stuck'),
      'progress must reset the clock, or every long task cries wolf',
    ).toBe(false);
  });

  it('an idle agent is finished, not stalled', async () => {
    const { result } = renderHook(() =>
      useAttentionFeed(working({ ambientState: 'idle' }), [agent('a1', 's1')]),
    );
    await act(async () => {
      await vi.advanceTimersByTimeAsync(20 * 60_000);
    });
    expect(result.current.items.some((it) => it.title === 'Not moving')).toBe(false);
  });

  it('says so plainly when the agent has stopped reporting at all', async () => {
    // Claude in a PTY — the one session shape whose status line is a real
    // heartbeat, so a stale `receivedAt` genuinely means the process went quiet.
    // (`working()` sets no provider/transport: absent means claude on PTY.)
    const { result } = renderHook(() =>
      useAttentionFeed(working({ statusLine: { receivedAt: new Date(T0).toISOString() } }), [
        agent('a1', 's1'),
      ]),
    );
    await act(async () => {
      await vi.advanceTimersByTimeAsync(6 * 60_000);
    });
    const item = result.current.items.find((it) => it.kind === 'stuck');
    expect(item?.title).toBe('No signal');
  });

  /**
   * The same stale status line on a managed provider means nothing at all: the
   * daemon publishes that line only when a usage frame moves the token totals,
   * which is exactly what the progress fingerprint counts. So it is guaranteed
   * stale the moment the stall fires, and reading it as death made every
   * managed-provider stall render as "No signal".
   */
  it('will not call a managed provider dead off a status line that always freezes', async () => {
    const { result } = renderHook(() =>
      useAttentionFeed(
        working({ provider: 'codex', statusLine: { receivedAt: new Date(T0).toISOString() } }),
        [agent('a1', 's1')],
      ),
    );
    await act(async () => {
      await vi.advanceTimersByTimeAsync(6 * 60_000);
    });
    const item = result.current.items.find((it) => it.kind === 'stuck');
    expect(item?.title).toBe('Not moving');
    // …and it says why, rather than quietly borrowing the Claude reading.
    expect(item?.detail).toMatch(/no heartbeat/);
    expect(item?.payload).toMatchObject({
      summary: expect.stringMatching(/reports only when it acts/),
    });
  });

  it('flags a workflow whose agents have all gone quiet', async () => {
    const { result } = renderHook(() =>
      useAttentionFeed(
        working({
          ambientState: 'background',
          workflows: [
            {
              runId: 'wf_1',
              name: 'bug-hunt',
              status: 'running',
              agents: [
                { id: 'a', status: 'running', toolCalls: 3, tokens: 10 },
                { id: 'b', status: 'done', toolCalls: 5, tokens: 20 },
              ],
            },
          ],
        }),
        [agent('a1', 's1')],
      ),
    );

    await act(async () => {
      await vi.advanceTimersByTimeAsync(9 * 60_000);
    });

    const wf = result.current.items.find((it) => it.title?.startsWith('Workflow not moving'));
    expect(wf, 'a stalled run should surface').toBeTruthy();
    expect(wf!.title).toContain('bug-hunt');
    expect(wf!.detail).toMatch(/1 still marked running/);
  });

  it('one card per stall, updated in place as it lengthens', async () => {
    const { result } = renderHook(() => useAttentionFeed(working(), [agent('a1', 's1')]));
    await act(async () => {
      await vi.advanceTimersByTimeAsync(6 * 60_000);
    });
    const first = result.current.items.filter((it) => it.kind === 'stuck').length;
    await act(async () => {
      await vi.advanceTimersByTimeAsync(6 * 60_000);
    });
    const later = result.current.items.filter((it) => it.kind === 'stuck');
    expect(first).toBe(1);
    expect(later).toHaveLength(1);
    expect(later[0].detail).toMatch(/1[12]m/, 'the same card, with a longer elapsed time');
  });
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
