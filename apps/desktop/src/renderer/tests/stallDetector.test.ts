/**
 * Stall detection: telling "this agent is thinking hard" apart from "this agent
 * is wedged" without a false positive every time either happens.
 *
 * The trap these pin down is that a working agent legitimately goes silent — a
 * long extended-thinking block emits no conversation items at all, because the
 * stream adapter drops thinking deltas. Anything keyed purely on "no items
 * lately" flags every deep think as broken, which is worse than not flagging at
 * all: a badge that cries wolf gets ignored when it's right.
 */
import { describe, it, expect } from 'vitest';
import type { ClaudeSessionSnapshot } from '../src/types/claudeSession';
import {
  STALL_MS,
  isWorkingState,
  progressFingerprint,
  runningWorkflows,
  stallOf,
  stalledFor,
  trackProgress,
  workflowFingerprint,
} from '../src/lib/stallDetector';

const T0 = 1_700_000_000_000;

function snap(over: Partial<ClaudeSessionSnapshot> = {}): ClaudeSessionSnapshot {
  return {
    sessionId: 's1',
    ambientState: 'thinking',
    conversation: [],
    activeToolCalls: [],
    completedToolCalls: [],
    fileChanges: [],
    subagents: [],
    workflows: [],
    totalToolCalls: 0,
    lastActivity: T0,
    ...over,
  } as ClaudeSessionSnapshot;
}

describe('progressFingerprint', () => {
  it('changes when any observable work advances', () => {
    const base = snap();
    const fp = progressFingerprint(base);

    expect(progressFingerprint(snap({ totalToolCalls: 1 }))).not.toBe(fp);
    expect(progressFingerprint(snap({ conversation: [{} as never] }))).not.toBe(fp);
    expect(progressFingerprint(snap({ fileChanges: [{} as never] }))).not.toBe(fp);
    expect(
      progressFingerprint(snap({ subagents: [{ id: 'x', status: 'running' } as never] })),
    ).not.toBe(fp);
  });

  /**
   * The case a naive check gets wrong: a turn deep in a thinking block produces
   * no items, but its token spend is climbing. That IS progress.
   */
  it('follows token spend, so a silent thinking turn still counts as moving', () => {
    const before = snap({ statusLine: { totalOutputTokens: 100 } as never });
    const after = snap({ statusLine: { totalOutputTokens: 240 } as never });
    expect(progressFingerprint(after)).not.toBe(progressFingerprint(before));
  });

  it('ignores things that are not progress', () => {
    const base = snap();
    // A status-line tick with the same figures is not work happening.
    expect(progressFingerprint(snap({ statusLine: { receivedAt: 'later' } as never }))).toBe(
      progressFingerprint(snap({ statusLine: {} as never })),
    );
    // Nor is the wall clock moving.
    expect(progressFingerprint(snap({ lastActivity: T0 + 60_000 }))).toBe(
      progressFingerprint(base),
    );
  });

  it('sees a workflow agent advance even when the parent looks unchanged', () => {
    const run = (toolCalls: number) => ({
      workflows: [
        {
          runId: 'wf_1',
          status: 'running',
          agents: [{ id: 'a', status: 'running', toolCalls, tokens: 0 }],
        },
      ] as never,
    });
    expect(workflowFingerprint(snap(run(3)))).not.toBe(workflowFingerprint(snap(run(2))));
    // …and the session fingerprint carries it too.
    expect(progressFingerprint(snap(run(3)))).not.toBe(progressFingerprint(snap(run(2))));
  });
});

describe('trackProgress', () => {
  it('stamps the time a fingerprint changed and holds it while it does not', () => {
    let marks = trackProgress(new Map(), { s1: snap() }, T0);
    expect(marks.get('s1')?.since).toBe(T0);

    // Same work → the mark is kept, so elapsed time keeps accumulating.
    marks = trackProgress(marks, { s1: snap() }, T0 + 60_000);
    expect(marks.get('s1')?.since).toBe(T0);

    // Real progress → the clock resets.
    marks = trackProgress(marks, { s1: snap({ totalToolCalls: 1 }) }, T0 + 90_000);
    expect(marks.get('s1')?.since).toBe(T0 + 90_000);
  });

  it('returns the same map when nothing moved, so it is safe as a dependency', () => {
    const first = trackProgress(new Map(), { s1: snap() }, T0);
    const second = trackProgress(first, { s1: snap() }, T0 + 1000);
    expect(second).toBe(first);
  });

  it('forgets sessions that have gone away', () => {
    const marks = trackProgress(new Map(), { s1: snap(), s2: snap() }, T0);
    const after = trackProgress(marks, { s1: snap() }, T0 + 1000);
    expect(after.has('s2')).toBe(false);
    expect(after.get('s1')?.since).toBe(T0);
  });
});

describe('stallOf', () => {
  const mark = { fingerprint: 'x', since: T0 };

  it('reports a stall only after the threshold, and only while working', () => {
    expect(stallOf(snap(), mark, T0 + STALL_MS - 1)).toBeNull();
    expect(stallOf(snap(), mark, T0 + STALL_MS + 1)).not.toBeNull();

    for (const state of ['idle', 'waiting_approval', 'waiting_input'] as const) {
      expect(
        stallOf(snap({ ambientState: state }), mark, T0 + STALL_MS * 10),
        `${state} is not a stall — the feed says that better with another kind`,
      ).toBeNull();
    }
  });

  it('counts a session with spawned work still running', () => {
    // 'background' means the turn ended but subagents/workflows are live — the
    // exact state where a wedged child goes unnoticed.
    expect(stallOf(snap({ ambientState: 'background' }), mark, T0 + STALL_MS + 1)).not.toBeNull();
  });

  it('never reports a stall for a session it has not tracked yet', () => {
    expect(stallOf(snap(), undefined, T0 + STALL_MS * 10)).toBeNull();
  });

  /**
   * The two readings the card has to distinguish: a live process producing
   * nothing (a long think, or a hung tool) versus one that has stopped
   * reporting altogether.
   */
  it('separates "alive but silent" from "no signal at all"', () => {
    const ticking = snap({
      statusLine: { receivedAt: new Date(T0 + STALL_MS).toISOString() } as never,
    });
    expect(stallOf(ticking, mark, T0 + STALL_MS + 1000)?.alive).toBe(true);

    const gone = snap({ statusLine: { receivedAt: new Date(T0).toISOString() } as never });
    expect(stallOf(gone, mark, T0 + STALL_MS + 1000)?.alive).toBe(false);
  });

  it('does not claim silence it cannot observe', () => {
    // Providers other than claude publish no status line; absent is unknown, and
    // unknown must not read as dead.
    expect(stallOf(snap(), mark, T0 + STALL_MS + 1)?.alive).toBe(true);
    expect(
      stallOf(snap({ statusLine: { receivedAt: 'not a date' } as never }), mark, T0 + STALL_MS + 1)
        ?.alive,
    ).toBe(true);
  });

  it('reports how long it has been stalled', () => {
    expect(stallOf(snap(), mark, T0 + 7 * 60_000)?.stalledForMs).toBe(7 * 60_000);
  });
});

describe('helpers', () => {
  it('only counts running workflow runs', () => {
    const s = snap({
      workflows: [
        { runId: 'a', status: 'running', agents: [] },
        { runId: 'b', status: 'completed', agents: [] },
        { runId: 'c', status: 'failed', agents: [] },
      ] as never,
    });
    expect(runningWorkflows(s).map((w) => w.runId)).toEqual(['a']);
  });

  it('phrases elapsed time plainly', () => {
    expect(stalledFor(6 * 60_000)).toBe('6m');
    expect(stalledFor(64 * 60_000)).toBe('1h 4m');
    // Never "0m" — a card that says nothing has happened for no time is absurd.
    expect(stalledFor(20_000)).toBe('1m');
  });

  it('knows which states are working states', () => {
    expect(isWorkingState('thinking')).toBe(true);
    expect(isWorkingState('streaming')).toBe(true);
    expect(isWorkingState('background')).toBe(true);
    expect(isWorkingState('idle')).toBe(false);
    expect(isWorkingState(undefined)).toBe(false);
  });
});
