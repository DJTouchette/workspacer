/**
 * `agent.snapshot` stopped carrying whole transcripts and now carries a bounded
 * WINDOW anchored by `conversationOffset`. createSnapshotFold is what turns that
 * back into the full conversation a pane renders.
 *
 * The failure mode is a wrong transcript on the session the user is actively
 * watching — silently missing or duplicated turns, invisible until someone
 * scrolls up — so these pin the seams rather than the happy path.
 */
import { describe, expect, it, vi } from 'vitest';
import { createSnapshotFold } from './webBackend';
import type { HubBusClient } from './hubBusClient';

type Snap = Record<string, any>;

function fold(snapshotReply: Snap | null = null) {
  const calls: Array<{ method: string; params: any }> = [];
  const client = {
    call: vi.fn((method: string, params: any) => {
      calls.push({ method, params });
      return Promise.resolve(snapshotReply);
    }),
  };
  return {
    ...createSnapshotFold(client as unknown as Pick<HubBusClient, 'call'>),
    calls,
    countOf: (m: string) => calls.filter((c) => c.method === m).length,
  };
}

const snap = (conversation: string[], conversationOffset = 0, extra: Snap = {}): Snap => ({
  sessionId: 's1',
  status: 'running',
  conversation,
  conversationOffset,
  ...extra,
});

describe('createSnapshotFold — conversation windows', () => {
  it('splices a window onto the full snapshot that seeded it', () => {
    const f = fold();
    f.seedFull(snap(['a', 'b', 'c', 'd', 'e']) as never);

    const out = f.foldConversation(snap(['d', 'e', 'f'], 3) as never) as Snap;
    expect(out.conversation).toEqual(['a', 'b', 'c', 'd', 'e', 'f']);
    expect(out.conversationOffset).toBe(0);
  });

  it('accumulates across successive windows', () => {
    const f = fold();
    f.seedFull(snap(['a', 'b']) as never);
    f.foldConversation(snap(['b', 'c'], 1) as never);
    const out = f.foldConversation(snap(['c', 'd'], 2) as never) as Snap;
    expect(out.conversation).toEqual(['a', 'b', 'c', 'd']);
  });

  // Without the seed a pane would only ever see the window, which is exactly
  // the regression this whole change risks.
  it('adopts the window when no history was seeded', () => {
    const f = fold();
    const out = f.foldConversation(snap(['y', 'z'], 8) as never) as Snap;
    expect(out.conversation).toEqual(['y', 'z']);
    expect(out.conversationOffset).toBe(8);
  });

  it('drops a stale push instead of truncating what the user can see', () => {
    const f = fold();
    f.seedFull(snap(['a', 'b', 'c', 'd']) as never);
    expect(f.foldConversation(snap(['b', 'c'], 1) as never)).toBeNull();
  });

  // A gap must never be concatenated across: it would render turns adjacent
  // that never were.
  it('refetches the full snapshot on a gap and suppresses the window', () => {
    const f = fold(snap(['a', 'b', 'c', 'd', 'e', 'f', 'g']) as never);
    f.seedFull(snap(['a', 'b']) as never);

    expect(f.foldConversation(snap(['f', 'g'], 5) as never)).toBeNull();
    expect(f.countOf('sessions.snapshot')).toBe(1);
    expect(f.calls[0].params).toEqual({ sessionId: 's1' });
  });

  it('keeps only one refetch in flight per session', () => {
    const f = fold();
    f.seedFull(snap(['a', 'b']) as never);
    f.foldConversation(snap(['f', 'g'], 5) as never);
    f.foldConversation(snap(['h', 'i'], 7) as never);
    expect(f.countOf('sessions.snapshot')).toBe(1);
  });

  it('adopts a rebuilt transcript whose offset resets to zero', () => {
    const f = fold();
    f.seedFull(snap(['p', 'q', 'r'], 4) as never);
    const out = f.foldConversation(snap(['x', 'y'], 0) as never) as Snap;
    expect(out.conversation).toEqual(['x', 'y']);
    expect(out.conversationOffset).toBe(0);
  });
});

// sessions.snapshots is compacted now, and getAllClaudeSessions runs every row
// through foldSparse — which writes the history cache. OverviewPane refreshes
// that list up to 1/s while an agent streams, so a list row that replaced the
// cache would collapse the watched pane's transcript to twelve turns, once a
// second, forever.
describe('createSnapshotFold — a background list row must not shorten history', () => {
  const long = Array.from({ length: 40 }, (_, i) => `t${i}`);

  it('keeps the seeded full history when a compacted list row arrives', () => {
    const f = fold();
    f.seedFull(snap(long) as never);
    f.foldSparse(snap(long.slice(28), 28) as never); // the compacted list row

    // The next push must still splice onto all 40 turns, not onto 12.
    const out = f.foldConversation(snap(['t39', 't40'], 39) as never) as Snap;
    expect(out.conversation).toHaveLength(41);
    expect(out.conversation[0]).toBe('t0');
  });

  it('still adopts a list row when there is no history to protect', () => {
    const f = fold();
    f.foldSparse(snap(long.slice(28), 28) as never);
    const out = f.foldConversation(snap(['t39', 't40'], 39) as never) as Snap;
    expect(out.conversation).toEqual([...long.slice(28), 't40']);
  });
});

describe('createSnapshotFold — sparse rows still overlay', () => {
  it('overlays a sparse row onto the retained rich one without losing conversation', () => {
    const f = fold();
    f.seedFull(snap(['a', 'b'], 0, { cwd: '/proj' }) as never);
    const out = f.foldSparse({
      sessionId: 's1',
      status: 'ended',
      sparse: true,
    } as never) as Snap;
    expect(out.conversation).toEqual(['a', 'b']);
    expect(out.cwd).toBe('/proj');
    expect(out.status).toBe('ended');
  });

  // ── The daemon-owned canonical selection slice ──
  //
  // claudemon owns `requested_selection` / `resolved_context_window`; the web
  // backend maps and forwards them. The combined brain now emits the camelCase
  // pair alongside the snake originals, but an older one sends only the snake
  // form — so both are read, and a sparse tick that mentions neither must not
  // erase what the rich row already carried.
  it('maps the snake_case slice an older headless node still sends', () => {
    const f = fold();
    const out = f.foldSparse({
      sessionId: 's1',
      status: 'active',
      sparse: true,
      requested_selection: { model: 'claude-opus-5', context_window: 1_000_000 },
      resolved_context_window: 1_000_000,
    } as never) as Snap;
    expect(out.requestedSelection).toEqual({ model: 'claude-opus-5', contextWindow: 1_000_000 });
    expect(out.resolvedContextWindow).toBe(1_000_000);
  });

  it('keeps the slice when a later sparse row omits it', () => {
    const f = fold();
    f.seedFull(
      snap(['a'], 0, {
        requestedSelection: { model: 'claude-opus-5', contextWindow: 1_000_000 },
        resolvedContextWindow: 1_000_000,
      }) as never,
    );
    const out = f.foldSparse({
      sessionId: 's1',
      status: 'active',
      sparse: true,
      ambientState: 'streaming',
    } as never) as Snap;
    expect(out.ambientState).toBe('streaming');
    expect(out.resolvedContextWindow).toBe(1_000_000);
    expect(out.requestedSelection).toEqual({ model: 'claude-opus-5', contextWindow: 1_000_000 });
  });

  it('leaves both keys off a row whose owner said nothing', () => {
    const f = fold();
    const out = f.foldSparse(snap(['a']) as never) as Snap;
    expect('requestedSelection' in out).toBe(false);
    expect('resolvedContextWindow' in out).toBe(false);
  });

  // Ended sessions must leave the cache or it grows for the app's lifetime.
  it('evicts an ended session from the cache', () => {
    const f = fold();
    f.seedFull(snap(['a'], 0) as never);
    f.foldConversation(snap(['a', 'b'], 0, { status: 'ended' }) as never);
    // Re-entering after eviction adopts rather than splicing onto stale history.
    const out = f.foldConversation(snap(['z'], 9) as never) as Snap;
    expect(out.conversation).toEqual(['z']);
    expect(out.conversationOffset).toBe(9);
  });
});
