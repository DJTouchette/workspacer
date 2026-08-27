import { describe, it, expect, vi, beforeEach } from 'vitest';
import {
  applyConversationItems,
  createBusConversations,
  newConversationState,
  type ConversationItemWire,
} from '../../src/backend/busConversation';
import { createWebBackend } from '../../src/backend/webBackend';
import type { ClaudeSessionSnapshot } from '../../../main/shared/ipcTypes';

// ─── The bug this file exists for ────────────────────────────────────────────
// `/app` against a hub whose sessions live on a HEADLESS node (`workspacer
// serve` — a Fly worker, or any brain-provided fleet) showed an empty chat and
// an optimistic "Sending…" bubble that never cleared. Reproduced live: the
// message was delivered (agents.sendMessage acked in ~2ms, the turn ran, usage
// and approvals streamed back), but the brain's snapshot rows are `sparse` and
// carry NO conversation by design — the transcript lives behind
// `sessions.conversation`, which the web backend never called for its own
// sessions. ClaudePane retires optimistic bubbles by watching the conversation
// grow a user turn, so with no conversation the bubble was immortal.
//
// The e2e `app` suite could not see this: its fake provider answers RICH
// desktop-shaped rows (FIXTURE_SESSIONS, conversation included), which is the
// one shape that never had the problem.

describe('applyConversationItems (claudemon items → renderer turns)', () => {
  it('folds a user message and the assistant reply into turns', () => {
    const st = newConversationState();
    applyConversationItems(st, [
      { kind: 'user_message', text: 'Reply with exactly: PONG' },
      { kind: 'assistant_text', text: 'PONG', timestamp: '2026-08-26T14:40:11.268Z' },
    ]);
    expect(st.turns).toEqual([
      { role: 'user', content: 'Reply with exactly: PONG', timestamp: undefined },
      { role: 'assistant', content: 'PONG', timestamp: Date.parse('2026-08-26T14:40:11.268Z') },
    ]);
  });

  it('grows the newest assistant bubble in place when the daemon re-sends it', () => {
    // claudemon coalesces a streamed reply into ONE item that grows; our poll
    // deliberately re-requests that item, so the fold must extend rather than
    // push a second bubble.
    const st = newConversationState();
    applyConversationItems(st, [{ kind: 'assistant_text', text: '1. sabbath' }]);
    applyConversationItems(st, [{ kind: 'assistant_text', text: '1. sabbath\n2. sabotage' }]);
    expect(st.turns).toHaveLength(1);
    expect(st.turns[0].content).toBe('1. sabbath\n2. sabotage');
  });

  it('gives every tool call its own turn and folds its result back in', () => {
    const st = newConversationState();
    applyConversationItems(st, [
      { kind: 'tool_use', id: 'toolu_1', name: 'Bash', input: { command: 'ls' } },
      { kind: 'tool_result', tool_use_id: 'toolu_1', content: 'a\nb', is_error: false },
      // A re-delivered call (replay / our own re-request) must not duplicate.
      { kind: 'tool_use', id: 'toolu_1', name: 'Bash', input: { command: 'ls' } },
    ]);
    expect(st.turns).toHaveLength(1);
    expect(st.turns[0].toolCalls?.[0]).toMatchObject({ id: 'toolu_1', name: 'Bash' });
    expect(st.turns[0].toolCalls?.[0].response).toBe('a\nb');
  });

  it('renders a slash command as a command card and attaches its output', () => {
    const st = newConversationState();
    applyConversationItems(st, [
      { kind: 'slash_command', name: 'model', args: 'opus' },
      { kind: 'command_output', output: 'switched' },
    ]);
    expect(st.turns).toHaveLength(1);
    expect(st.turns[0].command).toEqual({ name: 'model', args: 'opus', output: 'switched' });
  });

  it('drops a repeated user message that carries no fresh timestamp', () => {
    const st = newConversationState();
    applyConversationItems(st, [{ kind: 'user_message', text: 'go' }]);
    applyConversationItems(st, [{ kind: 'user_message', text: 'go' }]);
    expect(st.turns).toHaveLength(1);
  });
});

describe('createBusConversations', () => {
  it('re-requests the newest item so a growing reply keeps arriving', async () => {
    // A faithful stand-in for claudemon's window: `?since=` drops leading
    // items by index (items_skip in daemon/api.rs), and a streaming reply
    // COALESCES into the newest item rather than appending a new one — which
    // is why asking for "strictly newer than what I hold" would go blind.
    const store: ConversationItemWire[] = [{ kind: 'user_message', text: 'hi' }];
    let seq = 1;
    const calls: Array<Record<string, unknown>> = [];
    const folded: string[] = [];
    const convo = createBusConversations(
      (_sessionId, params) => {
        calls.push(params);
        const since = params.sinceSeq as number | undefined;
        const skip = since === undefined ? 0 : Math.min(Math.max(0, since + 1 - 1), store.length);
        return Promise.resolve({ seq, first_seq: 1, items: store.slice(skip) });
      },
      (sessionId) => folded.push(sessionId),
    );

    await convo.poke('s1');
    expect(calls[0]).toEqual({ sessionId: 's1' }); // nothing held yet → whole window

    store.push({ kind: 'assistant_text', text: 'hel' });
    seq = 6;
    await convo.poke('s1');
    expect(calls[1]).toEqual({ sessionId: 's1', sinceSeq: 0 });

    // The reply grows IN PLACE (same item, higher seq) — the case that made
    // "since = the seq you were told" lose the whole answer.
    store[1] = { kind: 'assistant_text', text: 'hello' };
    seq = 9;
    await convo.poke('s1');
    expect(calls[2]).toEqual({ sessionId: 's1', sinceSeq: 1 });

    const merged = convo.merge({ sessionId: 's1', status: 'active' } as ClaudeSessionSnapshot);
    expect(merged.conversation?.map((t) => [t.role, t.content])).toEqual([
      ['user', 'hi'],
      ['assistant', 'hello'],
    ]);
    expect(folded, 'each fetch that changed something re-emits the session').toEqual([
      's1',
      's1',
      's1',
    ]);
  });

  it('rebuilds from scratch when the daemon rewinds (a restarted thread)', async () => {
    let n = 0;
    const convo = createBusConversations(
      (_sessionId, params) => {
        n++;
        if (n === 1)
          return Promise.resolve({
            seq: 9,
            first_seq: 1,
            items: [{ kind: 'user_message', text: 'old' }],
          });
        if (n === 2) return Promise.resolve({ seq: 2, first_seq: 1, items: [] }); // rewound
        return Promise.resolve({
          seq: 2,
          first_seq: 1,
          items: [{ kind: 'user_message', text: 'new thread' }],
        });
      },
      () => {},
    );
    await convo.poke('s1');
    await convo.poke('s1');
    const merged = convo.merge({ sessionId: 's1', status: 'active' } as ClaudeSessionSnapshot);
    expect(merged.conversation?.map((t) => t.content)).toEqual(['new thread']);
  });

  it('hands each snapshot its OWN array, so a memo keyed on the transcript re-runs', async () => {
    // The fold buffer is mutated in place; handing it out directly gave every
    // snapshot the same array identity forever. ClaudePane memoizes the
    // "Working for …" anchor on exactly that identity (`[conversation]`), so a
    // stable reference froze the anchor a whole turn behind and the label
    // counted the gap between turns as work. Electron IPC gives the desktop a
    // fresh array per push (structured clone); the bus seam must match.
    let turn = 0;
    const convo = createBusConversations(
      () =>
        Promise.resolve({
          seq: ++turn,
          first_seq: 1,
          items: [
            {
              kind: 'user_message',
              text: `ask ${turn}`,
              timestamp: `2026-08-26T14:4${turn}:00.000Z`,
            },
          ],
        }),
      () => {},
    );
    await convo.poke('s1');
    const first = convo.merge({ sessionId: 's1', status: 'active' } as ClaudeSessionSnapshot);
    // Two merges of the SAME fold still hand out distinct arrays — cheap, and
    // it means no caller can ever come to depend on the buffer's identity.
    const again = convo.merge({ sessionId: 's1', status: 'active' } as ClaudeSessionSnapshot);
    expect(again.conversation).not.toBe(first.conversation);
    expect(again.conversation).toEqual(first.conversation);

    await convo.poke('s1');
    const second = convo.merge({ sessionId: 's1', status: 'active' } as ClaudeSessionSnapshot);
    expect(second.conversation).not.toBe(first.conversation);
    // …and the older array is a real snapshot of the older transcript, not a
    // view onto the buffer that grew under it.
    expect(first.conversation?.map((t) => t.content)).toEqual(['ask 1']);
    expect(second.conversation?.map((t) => t.content)).toEqual(['ask 1', 'ask 2']);
  });

  it('leaves a snapshot that brought its own conversation untouched', () => {
    const convo = createBusConversations(
      () => Promise.resolve(null),
      () => {},
    );
    const rich = {
      sessionId: 's1',
      status: 'active',
      conversation: [{ role: 'user', content: 'from the desktop' }],
    } as ClaudeSessionSnapshot;
    expect(convo.merge(rich)).toBe(rich);
  });
});

// ─── Delta mode: fragments, not re-sends ─────────────────────────────────────
// An `agent.conversation.<id>` bus event carries claudemon's raw broadcast
// fragments ("world" after "hello"). The snapshot rule's startsWith test is
// false for a fragment, and folding one through it pushed a second bubble —
// one paragraph per token.

describe('applyConversationItems in delta mode', () => {
  it('appends a streaming fragment to the open assistant bubble', () => {
    const st = newConversationState();
    applyConversationItems(st, [{ kind: 'assistant_text', text: 'hello' }], {
      kind: 'delta',
      streaming: true,
    });
    applyConversationItems(st, [{ kind: 'assistant_text', text: ' world' }], {
      kind: 'delta',
      streaming: true,
    });
    expect(st.turns).toHaveLength(1);
    expect(st.turns[0].content).toBe('hello world');
  });

  it('replaces when the fragment is accumulated-text growth (OpenCode shape)', () => {
    const st = newConversationState();
    applyConversationItems(st, [{ kind: 'assistant_text', text: 'hello' }], {
      kind: 'delta',
      streaming: true,
    });
    applyConversationItems(st, [{ kind: 'assistant_text', text: 'hello world' }], {
      kind: 'delta',
      streaming: true,
    });
    expect(st.turns).toHaveLength(1);
    expect(st.turns[0].content).toBe('hello world');
  });

  it('a tool call closes the bubble — later fragments start a new one', () => {
    const st = newConversationState();
    applyConversationItems(
      st,
      [
        { kind: 'assistant_text', text: 'first' },
        { kind: 'tool_use', id: 't1', name: 'Bash', input: {} },
        { kind: 'assistant_text', text: 'second' },
      ],
      { kind: 'delta', streaming: true },
    );
    expect(st.turns.map((t) => t.content)).toEqual(['first', '', 'second']);
  });

  it('keeps dedup-and-push for a Claude PTY transcript (whole blocks, replayed)', () => {
    // A PTY transcript's items are complete blocks re-emitted around
    // compaction — the same rule conversationApplier.ts applies locally.
    const st = newConversationState();
    applyConversationItems(st, [{ kind: 'assistant_text', text: 'block one' }], {
      kind: 'delta',
      streaming: false,
    });
    applyConversationItems(st, [{ kind: 'assistant_text', text: 'block one' }], {
      kind: 'delta',
      streaming: false,
    });
    applyConversationItems(st, [{ kind: 'assistant_text', text: 'block two' }], {
      kind: 'delta',
      streaming: false,
    });
    expect(st.turns.map((t) => t.content)).toEqual(['block one', 'block two']);
  });
});

describe('createBusConversations.applyDelta', () => {
  function rig() {
    const calls: Array<Record<string, unknown>> = [];
    const folded: string[] = [];
    const convo = createBusConversations(
      (_sessionId, params) => {
        calls.push(params);
        return Promise.resolve({
          seq: 1,
          first_seq: 1,
          items: [{ kind: 'user_message', text: 'ask' } as ConversationItemWire],
        });
      },
      (sessionId) => folded.push(sessionId),
    );
    return { convo, calls, folded };
  }

  it('folds contiguous fragments with no fetch at all — the whole point', async () => {
    const { convo, calls, folded } = rig();
    await convo.poke('s1'); // seed: lastSeq = 1
    const fetches = calls.length;

    convo.applyDelta(
      's1',
      { session_id: 's1', seq: 2, items: [{ kind: 'assistant_text', text: 'PO' }] },
      true,
    );
    convo.applyDelta(
      's1',
      { session_id: 's1', seq: 3, items: [{ kind: 'assistant_text', text: 'NG' }] },
      true,
    );

    expect(calls.length, 'a contiguous delta costs zero RPCs').toBe(fetches);
    const merged = convo.merge({ sessionId: 's1', status: 'active' } as ClaudeSessionSnapshot);
    expect(merged.conversation?.map((t) => [t.role, t.content])).toEqual([
      ['user', 'ask'],
      ['assistant', 'PONG'],
    ]);
    expect(folded, 'each applied delta re-emits the session').toEqual(['s1', 's1', 's1']);
  });

  it('pokes when a delta outruns the seed snapshot', async () => {
    const { convo, calls } = rig();
    // Not contiguous with anything we hold — the anchored fetch heals it.
    convo.applyDelta(
      's1',
      { session_id: 's1', seq: 7, items: [{ kind: 'assistant_text', text: 'x' }] },
      true,
    );
    await Promise.resolve();
    expect(calls.length).toBe(1);
  });

  it('pokes on a seq gap (missed frames)', async () => {
    const { convo, calls } = rig();
    await convo.poke('s1'); // seed: lastSeq = 1
    const fetches = calls.length;
    convo.applyDelta(
      's1',
      { session_id: 's1', seq: 9, items: [{ kind: 'assistant_text', text: 'y' }] },
      true,
    );
    await Promise.resolve();
    expect(calls.length).toBe(fetches + 1);
  });

  it('adopts wholesale on reset (the daemon rebuilt the log)', async () => {
    const { convo } = rig();
    await convo.poke('s1');
    convo.applyDelta(
      's1',
      {
        session_id: 's1',
        seq: 1,
        reset: true,
        items: [{ kind: 'user_message', text: 'fresh thread' }],
      },
      true,
    );
    const merged = convo.merge({ sessionId: 's1', status: 'active' } as ClaudeSessionSnapshot);
    expect(merged.conversation?.map((t) => t.content)).toEqual(['fresh thread']);
    // And the counter followed the rebuild: the next contiguous delta folds.
    convo.applyDelta(
      's1',
      { session_id: 's1', seq: 2, items: [{ kind: 'assistant_text', text: 'ok' }] },
      true,
    );
    const after = convo.merge({ sessionId: 's1', status: 'active' } as ClaudeSessionSnapshot);
    expect(after.conversation?.map((t) => t.content)).toEqual(['fresh thread', 'ok']);
  });

  it('gives a changed fold fresh array and turn identities (the React.memo contract)', async () => {
    // ClaudePane memoizes on the conversation ARRAY's identity and
    // ConversationMessage on the TURN's. An in-place `content += fragment`
    // satisfied every content assertion in this file while the real DOM sat
    // frozen on the first fragment for an entire turn (observed live in
    // headless Chromium against a real serve stack) — so identity IS the
    // contract, and this pins it.
    const { convo } = rig();
    await convo.poke('s1');
    convo.applyDelta(
      's1',
      { session_id: 's1', seq: 2, items: [{ kind: 'assistant_text', text: 'hel' }] },
      true,
    );
    const snap = { sessionId: 's1', status: 'active' } as ClaudeSessionSnapshot;
    const first = convo.merge(snap);
    convo.applyDelta(
      's1',
      { session_id: 's1', seq: 3, items: [{ kind: 'assistant_text', text: 'lo' }] },
      true,
    );
    const second = convo.merge(snap);

    expect(second.conversation).not.toBe(first.conversation);
    const a = first.conversation!;
    const b = second.conversation!;
    expect(b[b.length - 1]).not.toBe(a[a.length - 1]); // the grown bubble re-renders
    expect(b[0]).toBe(a[0]); // the untouched turn keeps its memo identity
    expect(b[b.length - 1].content).toBe('hello');
  });

  it('ignores the ready handshake (no seq) and advances on empty heartbeats', async () => {
    const { convo, calls } = rig();
    await convo.poke('s1');
    const fetches = calls.length;
    convo.applyDelta('s1', { session_id: 's1', ready: true }, true); // no seq: not a delta
    convo.applyDelta('s1', { session_id: 's1', seq: 4, items: [] }, true); // heartbeat
    convo.applyDelta(
      's1',
      { session_id: 's1', seq: 5, items: [{ kind: 'assistant_text', text: 'hi' }] },
      true,
    );
    await Promise.resolve();
    expect(calls.length, 'neither frame may look like a gap').toBe(fetches);
    const merged = convo.merge({ sessionId: 's1', status: 'active' } as ClaudeSessionSnapshot);
    expect(merged.conversation?.map((t) => t.content)).toEqual(['ask', 'hi']);
  });
});

// ─── The seam test: a headless fleet, end to end through createWebBackend ────

interface BusEvent {
  data: unknown;
  hub?: string;
}
let handlers: Map<string, (ev: BusEvent) => void>;
let busCalls: Array<{ method: string; params: any }>;
let answers: Record<string, (params: any) => unknown>;
let reconnectCbs: Array<() => void>;

vi.mock('../../src/backend/hubBusClient', () => ({
  HubBusClient: class {
    constructor(
      readonly token: string,
      readonly busUrl?: string,
    ) {}
    start() {}
    isConnected() {
      return true;
    }
    onStatus() {
      return () => {};
    }
    onReconnect(cb: () => void) {
      reconnectCbs.push(cb);
      return () => {};
    }
    can() {
      return true;
    }
    call(method: string, params: any) {
      busCalls.push({ method, params });
      const fn = answers[method];
      return Promise.resolve(fn ? fn(params) : {});
    }
    subscribe(topic: string, cb: (ev: BusEvent) => void) {
      handlers.set(topic, cb);
      return () => handlers.delete(topic);
    }
  },
}));

/** What the headless brain actually publishes: claudemon's row with the
 *  desktop field names overlaid and `sparse: true` — no conversation at all.
 *  Copied from a live `agent.snapshot` frame off `workspacer serve`. */
const SPARSE_ROW = {
  sessionId: 'sess-1',
  session_id: 'sess-1',
  sparse: true,
  status: 'active',
  mode: 'input',
  ambientState: 'idle',
  provider: 'claude',
  transport: 'stream',
  cwd: '/work',
  tool_calls: 0,
  totalToolCalls: 0,
  pendingApproval: null,
  pendingQuestions: null,
  usage: { model: 'claude-opus-5', contextTokens: 3000, costUSD: 0.2 },
};

describe('webBackend against a headless (brain-provided) fleet', () => {
  beforeEach(() => {
    handlers = new Map();
    busCalls = [];
    answers = {};
    reconnectCbs = [];
  });

  it('gives a sparse session its transcript, so an optimistic send can retire', async () => {
    answers['sessions.snapshot'] = () => ({ ...SPARSE_ROW });
    answers['sessions.conversation'] = () => ({
      seq: 3,
      first_seq: 1,
      items: [
        { kind: 'user_message', text: 'Reply with exactly: PONG' },
        { kind: 'assistant_text', text: 'PONG', timestamp: '2026-08-26T14:40:11.268Z' },
      ],
    });

    const api = createWebBackend('tok');
    const seen: ClaudeSessionSnapshot[] = [];
    api.onClaudeSessionUpdate((_id, snap) => seen.push(snap as ClaudeSessionSnapshot));

    await api.getClaudeSession('sess-1');
    // the fetch getClaudeSession primed resolves on the next microtasks
    await new Promise((r) => setTimeout(r, 0));

    expect(busCalls.some((c) => c.method === 'sessions.conversation')).toBe(true);
    // The fold re-emits the session so a fetch landing between snapshots still
    // reaches the pane.
    const last = seen[seen.length - 1];
    expect(last, 'the fold must push the session back to its subscribers').toBeTruthy();
    expect(last.conversation?.map((t) => [t.role, t.content])).toEqual([
      ['user', 'Reply with exactly: PONG'],
      ['assistant', 'PONG'],
    ]);
    // The count ClaudePane's optimistic FIFO reads. Zero here is the bug:
    // "Sending…" waits for exactly this to grow.
    expect(last.conversationUserOffset ?? 0).toBe(0);
    expect(last.conversation?.filter((t) => t.role === 'user')).toHaveLength(1);
  });

  it('merges the transcript onto every later sparse push, and keeps polling', async () => {
    answers['sessions.snapshot'] = () => ({ ...SPARSE_ROW });
    let turn = 1;
    answers['sessions.conversation'] = () => ({
      seq: turn,
      first_seq: 1,
      items: [{ kind: 'user_message', text: `ask ${turn++}` }],
    });

    const api = createWebBackend('tok');
    const seen: ClaudeSessionSnapshot[] = [];
    api.onClaudeSessionUpdate((_id, snap) => seen.push(snap as ClaudeSessionSnapshot));
    await api.getClaudeSession('sess-1');
    await new Promise((r) => setTimeout(r, 0));

    const before = busCalls.filter((c) => c.method === 'sessions.conversation').length;
    handlers.get('agent.snapshot')?.({ data: { ...SPARSE_ROW, ambientState: 'streaming' } });
    await new Promise((r) => setTimeout(r, 0));

    expect(
      busCalls.filter((c) => c.method === 'sessions.conversation').length,
      'a state tick on a watched session re-arms the transcript fetch',
    ).toBeGreaterThan(before);
    const pushed = seen[seen.length - 1];
    expect(pushed.conversation?.length, 'the pushed row carries the transcript').toBeGreaterThan(0);
  });

  it('does not poll a session no pane has opened', async () => {
    answers['sessions.conversation'] = () => ({ seq: 1, first_seq: 1, items: [] });
    createWebBackend('tok');
    handlers.get('agent.snapshot')?.({ data: { ...SPARSE_ROW, sessionId: 'other' } });
    await new Promise((r) => setTimeout(r, 0));
    expect(busCalls.filter((c) => c.method === 'sessions.conversation')).toHaveLength(0);
  });

  // ── The delta push, end to end through createWebBackend ────────────────
  // The subscribe lifecycle is the crux (internal/bus/demand.go): the
  // subscription IS the hub-side demand, so these pin exactly when it exists.

  function convFetches() {
    return busCalls.filter((c) => c.method === 'sessions.conversation').length;
  }

  async function openStreamingSession(api: ReturnType<typeof createWebBackend>) {
    answers['sessions.snapshot'] = () => ({ ...SPARSE_ROW, ambientState: 'streaming' });
    answers['sessions.conversation'] = () => ({
      seq: 1,
      first_seq: 1,
      items: [{ kind: 'user_message', text: 'ask' }],
    });
    const seen: ClaudeSessionSnapshot[] = [];
    api.onClaudeSessionUpdate((_id, snap) => seen.push(snap as ClaudeSessionSnapshot));
    await api.getClaudeSession('sess-1');
    await vi.advanceTimersByTimeAsync(0);
    return seen;
  }

  it('replaces the tick with pushed deltas once the ready handshake proves the path', async () => {
    vi.useFakeTimers();
    const api = createWebBackend('tok');
    const seen = await openStreamingSession(api);

    const push = handlers.get('agent.conversation.sess-1');
    expect(push, 'a pane opening the session subscribes its delta topic').toBeTruthy();

    // Until proof arrives the 500ms tick runs — a new client against an old
    // hub (no demand table) or an old node (no forwarder) degrades, exactly
    // the 8f70e9c7 behaviour, and never silently freezes.
    const before = convFetches();
    await vi.advanceTimersByTimeAsync(1100);
    expect(convFetches(), 'the fallback tick runs before ready').toBeGreaterThan(before);

    // ready → the tick disarms; contiguous deltas render with ZERO fetches.
    push!({ data: { session_id: 'sess-1', ready: true } });
    const settled = convFetches();
    await vi.advanceTimersByTimeAsync(5000);
    expect(convFetches(), 'the tick is dead once push is live').toBe(settled);

    push!({
      data: { session_id: 'sess-1', seq: 2, items: [{ kind: 'assistant_text', text: 'PO' }] },
    });
    push!({
      data: { session_id: 'sess-1', seq: 3, items: [{ kind: 'assistant_text', text: 'NG' }] },
    });
    expect(convFetches(), 'a contiguous delta costs no RPC').toBe(settled);
    const last = seen[seen.length - 1];
    expect(last.conversation?.map((t) => [t.role, t.content])).toEqual([
      ['user', 'ask'],
      ['assistant', 'PONG'],
    ]);
    vi.useRealTimers();
  });

  it('drops back to the tick after a reconnect, until a fresh ready arrives', async () => {
    vi.useFakeTimers();
    const api = createWebBackend('tok');
    await openStreamingSession(api);
    const push = handlers.get('agent.conversation.sess-1')!;
    push({ data: { session_id: 'sess-1', ready: true } });
    const live = convFetches();
    await vi.advanceTimersByTimeAsync(2000);
    expect(convFetches()).toBe(live);

    // Reconnect: the bus client re-asserts the subscription, but the proof is
    // void — we may be talking to a different (older) hub now. Tick until the
    // 0→1 our re-subscribe causes makes the brain re-announce ready.
    for (const cb of reconnectCbs) cb();
    const before = convFetches();
    await vi.advanceTimersByTimeAsync(1100);
    expect(convFetches(), 'the tick re-arms after a reconnect').toBeGreaterThan(before);

    push({ data: { session_id: 'sess-1', ready: true } });
    const settled = convFetches();
    await vi.advanceTimersByTimeAsync(2000);
    expect(convFetches(), 'a fresh ready disarms it again').toBe(settled);
    vi.useRealTimers();
  });

  it('a delta seq gap falls back to one anchored fetch', async () => {
    vi.useFakeTimers();
    const api = createWebBackend('tok');
    await openStreamingSession(api);
    const push = handlers.get('agent.conversation.sess-1')!;
    push({ data: { session_id: 'sess-1', ready: true } });
    const settled = convFetches();
    // seq jumps 1 → 9: frames were missed (broker drop, tab suspend). One
    // incremental fetch — the worst case is the old behaviour, once.
    push({
      data: { session_id: 'sess-1', seq: 9, items: [{ kind: 'assistant_text', text: 'x' }] },
    });
    await vi.advanceTimersByTimeAsync(0);
    expect(convFetches()).toBe(settled + 1);
    vi.useRealTimers();
  });

  it('a session ending releases the delta subscription', async () => {
    vi.useFakeTimers();
    const api = createWebBackend('tok');
    await openStreamingSession(api);
    expect(handlers.has('agent.conversation.sess-1')).toBe(true);

    handlers.get('agent.snapshot')?.({
      data: { ...SPARSE_ROW, status: 'ended', mode: 'stopped', ambientState: 'idle' },
    });
    await vi.advanceTimersByTimeAsync(0);
    expect(
      handlers.has('agent.conversation.sess-1'),
      'no subscriber, no bytes: the unsubscribe releases the hub-side demand',
    ).toBe(false);
    vi.useRealTimers();
  });

  it('closing the last watch pane releases the subscription; reopening re-arms it', async () => {
    vi.useFakeTimers();
    const api = createWebBackend('tok');
    await openStreamingSession(api);
    await api.attachClaude('paneA', 'sess-1');
    await api.attachClaude('paneB', 'sess-1');

    await api.detachClaude('paneA');
    expect(handlers.has('agent.conversation.sess-1'), 'another pane still watches').toBe(true);
    await api.detachClaude('paneB');
    expect(handlers.has('agent.conversation.sess-1'), 'the last pane closed').toBe(false);

    // Any pane activation re-fetches the session, which re-arms the push.
    await api.getClaudeSession('sess-1');
    await vi.advanceTimersByTimeAsync(0);
    expect(handlers.has('agent.conversation.sess-1')).toBe(true);
    vi.useRealTimers();
  });

  it('a federated session never subscribes — its deltas cannot cross the peer link', async () => {
    vi.useFakeTimers();
    answers['sessions.snapshot'] = () => ({ ...SPARSE_ROW, sessionId: 'peer-sess' });
    answers['sessions.conversation'] = () => ({ seq: 1, first_seq: 1, items: [] });
    const api = createWebBackend('tok');
    api.onClaudeSessionUpdate(() => {});
    // The peer stamp arrives on the envelope; remember it the way live traffic
    // would, then open the session.
    handlers.get('agent.snapshot')?.({
      data: { ...SPARSE_ROW, sessionId: 'peer-sess' },
      hub: 'peer1',
    });
    await api.getClaudeSession('peer-sess');
    await vi.advanceTimersByTimeAsync(0);
    expect(handlers.has('agent.conversation.peer-sess')).toBe(false);
    vi.useRealTimers();
  });
});
