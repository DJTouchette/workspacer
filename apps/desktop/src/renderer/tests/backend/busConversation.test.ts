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

// ─── The seam test: a headless fleet, end to end through createWebBackend ────

interface BusEvent {
  data: unknown;
  hub?: string;
}
let handlers: Map<string, (ev: BusEvent) => void>;
let busCalls: Array<{ method: string; params: any }>;
let answers: Record<string, (params: any) => unknown>;

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
    onReconnect() {
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
});
