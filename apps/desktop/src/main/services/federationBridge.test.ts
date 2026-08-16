/**
 * Behavioural tests for the desktop side of hub federation:
 *
 *   - a hub-stamped `agent.snapshot` event upserts a REMOTE session into
 *     claudeSessionStore (`hub` set) and pushes it to the renderer over the
 *     normal claude-session:update channel — and is NEVER republished onto the
 *     bus (publishSnapshot must stay silent for remote sessions);
 *   - `hub.peer.disconnected` tombstones that hub's sessions (`hubOffline`),
 *     and a reconnect re-seeds from `hub:<peer>/sessions.snapshots`, clearing
 *     the flag and dropping sessions the peer no longer reports;
 *   - the `federation:conversation` IPC (registered by the bridge) fetches the
 *     peer's full conversation over `hub:<peer>/sessions.conversation`, folds
 *     it into the store (windows stop clobbering it; refetches go incremental
 *     off main's own folded seq; a peer seq reset rebuilds), and pauses while
 *     the hub is tombstoned;
 *   - `sparse` rows: live ones (a headless-brain peer's whole fleet) become
 *     usable remote cards via an explicit mapping that leaks no wire
 *     internals; ended ones (layout ghosts / stopped claudemon rows) finalize
 *     a held session or stay invisible;
 *   - security: snapshotGrantsFsRoot refuses any hub-stamped snapshot, so a
 *     remote cwd can never enter the fs.* allow-list (workspaceRoots).
 *
 * Strategy (mirrors claudeSessionStore.test.ts): mock every side-effect
 * collaborator, drive the REAL store through the bridge by capturing the
 * listener it registers with subscribeHubEvents, and flush the ~16 ms
 * coalesced snapshot pushes with fake timers.
 */

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';

const { listeners, callHub, publishSnapshot, ipcHandlers } = vi.hoisted(() => ({
  listeners: new Set<(ev: unknown) => void>(),
  callHub: vi.fn(async (): Promise<unknown> => []),
  publishSnapshot: vi.fn(),
  ipcHandlers: new Map<string, (...args: unknown[]) => unknown>(),
}));

vi.mock('electron', () => ({
  BrowserWindow: class {},
  ipcMain: {
    handle: (channel: string, fn: (...args: unknown[]) => unknown) => {
      ipcHandlers.set(channel, fn);
    },
    removeHandler: (channel: string) => {
      ipcHandlers.delete(channel);
    },
  },
}));
vi.mock('./hubClient', () => ({
  subscribeHubEvents: (l: (ev: unknown) => void) => {
    listeners.add(l);
    return () => listeners.delete(l);
  },
  callHub: (...a: unknown[]) => callHub(...(a as [])),
}));
vi.mock('./agentNotifier', () => ({ agentNotifier: { notifyOnTransition: vi.fn() } }));
vi.mock('./supervisorNudge', () => ({ supervisorNudge: { onBlock: vi.fn() } }));
vi.mock('./budgetWatcher', () => ({ checkBudget: vi.fn() }));
vi.mock('./workflowWatcher', () => ({
  workflowWatcher: { attach: vi.fn(), detach: vi.fn(), poke: vi.fn() },
}));
vi.mock('./hubTelemetry', () => ({
  publishWorkflowRuns: vi.fn(),
  publishSnapshot: (...a: unknown[]) => publishSnapshot(...(a as [])),
  forgetSession: vi.fn(),
}));
vi.mock('./claudemonDaemon', () => ({ CLAUDEMON_API_URL: 'http://127.0.0.1:0' }));
vi.mock('./sessionStore/usageAccumulator', () => ({
  SessionUsageAccumulator: class {
    applyUsage(): void {}
    forget(): void {}
  },
}));
vi.mock('./sessionStore/analyticsWriter', () => ({ writeHistory: vi.fn() }));
vi.mock('./remoteTokens', () => ({ revokeSessionFacadeTokens: vi.fn() }));

import { claudeSessionStore } from './claudeSessionStore';
import {
  startFederationBridge,
  stopFederationBridge,
  listFederationPeers,
} from './federationBridge';
import { snapshotGrantsFsRoot } from '../lib/snapshotLiveness';

// A minimal but complete remote snapshot as the peer's publisher would send it.
function remoteSnap(sessionId: string, extra: Record<string, unknown> = {}) {
  return {
    sessionId,
    cwd: '/peer/proj',
    ptyId: sessionId,
    status: 'active',
    conversation: [],
    activeToolCalls: [],
    completedToolCalls: [],
    fileChanges: [],
    pendingApproval: null,
    pendingQuestions: null,
    subagents: [],
    workflows: [],
    ambientState: 'idle',
    lastActivity: 1,
    totalToolCalls: 0,
    usage: null,
    ...extra,
  };
}

const sent: Array<{ channel: string; sessionId: string; snapshot: Record<string, unknown> }> = [];

function emit(ev: Record<string, unknown>): void {
  for (const l of Array.from(listeners)) l(ev);
}

/** Deliver an event and settle both the async seed and the coalesced flush. */
async function emitAndFlush(ev: Record<string, unknown>): Promise<void> {
  emit(ev);
  await vi.advanceTimersByTimeAsync(50);
}

let seq = 0;
const uid = (p: string): string => `${p}-${++seq}`;

/** Invoke the federation:conversation IPC handler the bridge registered. */
async function invokeConversation(
  sessionId: string,
  sinceSeq?: number,
): Promise<{ seq: number; items: unknown[] } | null> {
  const handler = ipcHandlers.get('federation:conversation');
  expect(handler).toBeDefined();
  const result = (await handler!(null, sessionId, sinceSeq)) as {
    seq: number;
    items: unknown[];
  } | null;
  await vi.advanceTimersByTimeAsync(50); // settle the coalesced snapshot flush
  return result;
}

/** A live sparse row as the headless brain publishes it: desktop field names
 *  overlaid onto claudemon's raw snake_case row (enrich.go compatSnapshot). */
function brainSparseRow(sessionId: string, extra: Record<string, unknown> = {}) {
  return {
    // camelCase overlay
    sessionId,
    sparse: true,
    status: 'active',
    ambientState: 'waiting_approval',
    lastActivity: 1755000000000,
    usage: { model: 'claude-opus-4', contextTokens: 1000, contextLimit: 200000, costUSD: 0.5 },
    pendingApproval: { toolName: 'Bash', toolInput: { command: 'ls' } },
    pendingQuestions: null,
    label: 'brain agent',
    // snake_case originals ride along on the real wire
    session_id: sessionId,
    cwd: '/brain/proj',
    provider: 'claude',
    transport: 'pty',
    mode: 'approval',
    updated_at: '2026-08-15T12:00:00Z',
    tool_calls: 4,
    ...extra,
  };
}

beforeEach(() => {
  vi.useFakeTimers();
  sent.length = 0;
  callHub.mockClear();
  callHub.mockResolvedValue([]);
  publishSnapshot.mockClear();
  claudeSessionStore.setMainWindow({
    webContents: {
      send: (channel: string, sessionId: string, snapshot: Record<string, unknown>) => {
        sent.push({ channel, sessionId, snapshot });
      },
    },
    isDestroyed: () => false,
  } as never);
  startFederationBridge();
});

afterEach(() => {
  stopFederationBridge();
  vi.useRealTimers();
});

describe('ingest — hub-stamped agent.snapshot', () => {
  it('upserts a remote session with hub set and pushes a claude-session:update', async () => {
    const sid = uid('remote');
    // First stamped event also triggers the implicit-discovery seed, whose
    // result replaces this hub's sessions wholesale — answer what a live peer
    // would: the same session.
    callHub.mockResolvedValue([remoteSnap(sid, { ambientState: 'waiting_approval' })]);
    await emitAndFlush({
      type: 'agent.snapshot',
      hub: 'work',
      data: remoteSnap(sid, { ambientState: 'waiting_approval' }),
    });

    const snap = claudeSessionStore.getSnapshot(sid);
    expect(snap).not.toBeNull();
    expect(snap!.hub).toBe('work');
    expect(snap!.hubOffline).toBe(false);
    expect(snap!.ambientState).toBe('waiting_approval');
    // A remote transcript path names a file on the peer machine — blanked.
    expect(snap!.transcriptPath).toBe('');

    const update = sent.find((s) => s.sessionId === sid);
    expect(update).toBeDefined();
    expect(update!.channel).toBe('claude-session:update');
    expect(update!.snapshot.hub).toBe('work');

    // Never republished onto the bus: it CAME from the bus, and re-emitting it
    // unstamped would duplicate the peer's session for every bus client.
    expect(publishSnapshot).not.toHaveBeenCalled();
  });

  it('refuses a remote upsert over a local session with the same id', async () => {
    const sid = uid('local');
    claudeSessionStore.handleHookEvent({
      hook_event_name: 'UserPromptSubmit',
      session_id: sid,
      cwd: '/local/proj',
    });
    await emitAndFlush({
      type: 'agent.snapshot',
      hub: 'work',
      data: remoteSnap(sid),
    });
    expect(claudeSessionStore.getSnapshot(sid)!.hub).toBeUndefined();
  });

  it('a stamped event from a never-announced peer counts as discovery and seeds it', async () => {
    const sid = uid('remote');
    callHub.mockResolvedValue([remoteSnap(sid)]);
    await emitAndFlush({ type: 'agent.snapshot', hub: 'lazy-peer', data: remoteSnap(sid) });
    expect(callHub).toHaveBeenCalledWith('hub:lazy-peer/sessions.snapshots', {});
    expect(listFederationPeers()).toContainEqual(
      expect.objectContaining({ name: 'lazy-peer', connected: true }),
    );
  });
});

describe('tombstones — hub.peer.disconnected and reconnect reseed', () => {
  it('marks the hub sessions hubOffline, and reconnect+reseed clears/replaces', async () => {
    const kept = uid('kept');
    const dropped = uid('dropped');
    callHub.mockResolvedValue([remoteSnap(kept), remoteSnap(dropped)]);
    await emitAndFlush({ type: 'hub.peer.connected', data: { peer: 'sprite' } });
    expect(callHub).toHaveBeenCalledWith('hub:sprite/sessions.snapshots', {});
    expect(claudeSessionStore.getSnapshot(kept)!.hub).toBe('sprite');
    expect(claudeSessionStore.getSnapshot(dropped)!.hub).toBe('sprite');

    await emitAndFlush({
      type: 'hub.peer.disconnected',
      data: { peer: 'sprite', lastSeen: '2026-08-15T12:00:00Z' },
    });
    expect(claudeSessionStore.getSnapshot(kept)!.hubOffline).toBe(true);
    expect(claudeSessionStore.getSnapshot(dropped)!.hubOffline).toBe(true);
    const peer = listFederationPeers().find((p) => p.name === 'sprite');
    expect(peer).toMatchObject({ connected: false, lastSeen: Date.parse('2026-08-15T12:00:00Z') });

    // Reconnect: the peer now reports only `kept` — the tombstone clears on it
    // and `dropped` gets one final 'ended' push, then leaves the store.
    callHub.mockResolvedValue([remoteSnap(kept)]);
    sent.length = 0;
    await emitAndFlush({ type: 'hub.peer.connected', data: { peer: 'sprite' } });
    expect(claudeSessionStore.getSnapshot(kept)!.hubOffline).toBe(false);
    expect(claudeSessionStore.getSnapshot(dropped)).toBeNull();
    const final = sent.find((s) => s.sessionId === dropped);
    expect(final).toBeDefined();
    expect(final!.snapshot.status).toBe('ended');
    expect(listFederationPeers().find((p) => p.name === 'sprite')!.connected).toBe(true);
  });
});

describe('conversation — federation:conversation fetch + fold', () => {
  it('folds the peer conversation into the store; windows stop clobbering; refetch is incremental', async () => {
    const sid = uid('remote');
    const items = [
      { kind: 'user_message', text: 'first question', timestamp: '2026-08-15T10:00:00Z' },
      { kind: 'assistant_text', text: 'first answer', timestamp: '2026-08-15T10:00:05Z' },
      { kind: 'user_message', text: 'second question', timestamp: '2026-08-15T10:01:00Z' },
    ];
    callHub.mockImplementation(async (...args: unknown[]) =>
      (args[0] as string) === 'hub:work/sessions.conversation'
        ? { seq: 3, items }
        : [remoteSnap(sid)],
    );
    await emitAndFlush({ type: 'agent.snapshot', hub: 'work', data: remoteSnap(sid) });

    const res = await invokeConversation(sid);
    expect(res).toMatchObject({ seq: 3 });
    // First fetch is full (no sinceSeq — nothing folded yet).
    expect(callHub).toHaveBeenCalledWith('hub:work/sessions.conversation', { sessionId: sid });
    let snap = claudeSessionStore.getSnapshot(sid)!;
    expect(snap.conversation.map((t) => t.content)).toEqual([
      'first question',
      'first answer',
      'second question',
    ]);
    // The fold reached the renderer over the normal snapshot channel.
    const pushed = sent.find(
      (s) => s.sessionId === sid && (s.snapshot.conversation as unknown[])?.length === 3,
    );
    expect(pushed).toBeDefined();

    // A later bounded window push refreshes state but must NOT truncate the
    // folded history back down to its twelve-turn window.
    await emitAndFlush({
      type: 'agent.snapshot',
      hub: 'work',
      data: remoteSnap(sid, {
        ambientState: 'streaming',
        conversation: [{ role: 'user', content: 'second question' }],
        conversationOffset: 2,
        conversationUserOffset: 2,
      }),
    });
    snap = claudeSessionStore.getSnapshot(sid)!;
    expect(snap.ambientState).toBe('streaming');
    expect(snap.conversation).toHaveLength(3);
    // The window's anchors describe its slice, not our folded history.
    expect((snap as unknown as { conversationOffset?: number }).conversationOffset).toBe(0);

    // The next poke passes main's own folded seq and appends the new items.
    callHub.mockImplementation(async (...args: unknown[]) =>
      (args[0] as string) === 'hub:work/sessions.conversation'
        ? {
            seq: 4,
            items: [
              { kind: 'assistant_text', text: 'second answer', timestamp: '2026-08-15T10:02:00Z' },
            ],
          }
        : [remoteSnap(sid)],
    );
    await invokeConversation(sid, 3);
    expect(callHub).toHaveBeenCalledWith('hub:work/sessions.conversation', {
      sessionId: sid,
      sinceSeq: 3,
    });
    expect(claudeSessionStore.getSnapshot(sid)!.conversation.map((t) => t.content)).toEqual([
      'first question',
      'first answer',
      'second question',
      'second answer',
    ]);
  });

  it('answers null for local and unknown sessions without touching the bus', async () => {
    const sid = uid('local');
    claudeSessionStore.handleHookEvent({
      hook_event_name: 'UserPromptSubmit',
      session_id: sid,
      cwd: '/local/proj',
    });
    await vi.advanceTimersByTimeAsync(50);
    callHub.mockClear();
    expect(await invokeConversation(sid)).toBeNull();
    expect(await invokeConversation(uid('never-seen'))).toBeNull();
    expect(callHub).not.toHaveBeenCalled();
  });

  it('a peer seq reset (claudemon restart) rebuilds from the top instead of appending', async () => {
    const sid = uid('remote');
    callHub.mockImplementation(async (...args: unknown[]) =>
      (args[0] as string) === 'hub:work/sessions.conversation'
        ? {
            seq: 9,
            items: [{ kind: 'user_message', text: 'old life', timestamp: '2026-08-15T10:00:00Z' }],
          }
        : [remoteSnap(sid)],
    );
    await emitAndFlush({ type: 'agent.snapshot', hub: 'work', data: remoteSnap(sid) });
    await invokeConversation(sid);
    expect(claudeSessionStore.getSnapshot(sid)!.conversation).toHaveLength(1);

    // Peer restarted: its seq regressed below our folded 9 — the incremental
    // probe is followed by a full refetch, and history is replaced, not merged.
    const convCalls: unknown[][] = [];
    callHub.mockImplementation(async (...args: unknown[]) => {
      if ((args[0] as string) === 'hub:work/sessions.conversation') {
        convCalls.push(args);
        return {
          seq: 1,
          items: [{ kind: 'user_message', text: 'new life', timestamp: '2026-08-15T11:00:00Z' }],
        };
      }
      return [remoteSnap(sid)];
    });
    await invokeConversation(sid);
    expect(convCalls[0][1]).toEqual({ sessionId: sid, sinceSeq: 9 });
    expect(convCalls[1][1]).toEqual({ sessionId: sid });
    expect(claudeSessionStore.getSnapshot(sid)!.conversation.map((t) => t.content)).toEqual([
      'new life',
    ]);
  });

  it('a tombstoned hub pauses the fetching; the reconnect reseed resumes it incrementally', async () => {
    const sid = uid('remote');
    callHub.mockImplementation(async (...args: unknown[]) =>
      (args[0] as string) === 'hub:sprite/sessions.conversation'
        ? {
            seq: 2,
            items: [{ kind: 'user_message', text: 'hi', timestamp: '2026-08-15T10:00:00Z' }],
          }
        : [remoteSnap(sid)],
    );
    await emitAndFlush({ type: 'hub.peer.connected', data: { peer: 'sprite' } });
    expect(await invokeConversation(sid)).toMatchObject({ seq: 2 });

    await emitAndFlush({
      type: 'hub.peer.disconnected',
      data: { peer: 'sprite', lastSeen: '2026-08-15T12:00:00Z' },
    });
    callHub.mockClear();
    expect(await invokeConversation(sid)).toBeNull();
    expect(callHub).not.toHaveBeenCalled();

    // Link back: the reseed clears the tombstone AND keeps the folded history;
    // the next fetch resumes from the folded seq instead of starting over.
    callHub.mockImplementation(async (...args: unknown[]) =>
      (args[0] as string) === 'hub:sprite/sessions.conversation'
        ? {
            seq: 3,
            items: [
              { kind: 'assistant_text', text: 'welcome back', timestamp: '2026-08-15T12:01:00Z' },
            ],
          }
        : [remoteSnap(sid)],
    );
    await emitAndFlush({ type: 'hub.peer.connected', data: { peer: 'sprite' } });
    const snap = claudeSessionStore.getSnapshot(sid)!;
    expect(snap.hubOffline).toBe(false);
    expect(snap.conversation).toHaveLength(1); // folded history survived the reseed
    expect(await invokeConversation(sid)).toMatchObject({ seq: 3 });
    expect(callHub).toHaveBeenCalledWith('hub:sprite/sessions.conversation', {
      sessionId: sid,
      sinceSeq: 2,
    });
    expect(claudeSessionStore.getSnapshot(sid)!.conversation).toHaveLength(2);
  });
});

describe('ingest — sparse rows (headless-brain peers)', () => {
  it('a live sparse row becomes a usable remote card and leaks no wire internals', async () => {
    const sid = uid('brain');
    callHub.mockResolvedValue([brainSparseRow(sid)]);
    await emitAndFlush({ type: 'agent.snapshot', hub: 'brainpc', data: brainSparseRow(sid) });
    const snap = claudeSessionStore.getSnapshot(sid)!;
    expect(snap.hub).toBe('brainpc');
    expect(snap.hubOffline).toBe(false);
    expect(snap.status).toBe('active');
    expect(snap.ambientState).toBe('waiting_approval');
    expect(snap.cwd).toBe('/brain/proj');
    expect(snap.provider).toBe('claude');
    expect(snap.label).toBe('brain agent');
    expect(snap.usage).toMatchObject({ costUSD: 0.5 });
    // The approval card is actionable: same shape as every pendingApproval,
    // stamped on arrival (brain rows carry no timestamp of their own).
    expect(snap.pendingApproval).toMatchObject({
      toolName: 'Bash',
      toolInput: { command: 'ls' },
    });
    expect(typeof snap.pendingApproval!.timestamp).toBe('number');
    // The raw wire row's internals must not leak into the renderer snapshot.
    const raw = snap as unknown as Record<string, unknown>;
    expect(raw.sparse).toBeUndefined();
    expect(raw.mode).toBeUndefined();
    expect(raw.session_id).toBeUndefined();
    expect(raw.updated_at).toBeUndefined();
    // A remote cwd still grants no local fs root, sparse or not.
    expect(snapshotGrantsFsRoot(snap)).toBe(false);
    // And it is never republished onto the bus (it came from the bus).
    expect(publishSnapshot).not.toHaveBeenCalled();
  });

  it('an unchanged re-sent approval keeps its timestamp (dismissals stay dismissed)', async () => {
    const sid = uid('brain');
    callHub.mockResolvedValue([brainSparseRow(sid)]);
    await emitAndFlush({ type: 'agent.snapshot', hub: 'brainpc', data: brainSparseRow(sid) });
    const ts = claudeSessionStore.getSnapshot(sid)!.pendingApproval!.timestamp;
    await vi.advanceTimersByTimeAsync(5_000);
    await emitAndFlush({
      type: 'agent.snapshot',
      hub: 'brainpc',
      data: brainSparseRow(sid, { lastActivity: 1755000005000 }),
    });
    expect(claudeSessionStore.getSnapshot(sid)!.pendingApproval!.timestamp).toBe(ts);
  });

  it('an ended sparse row finalizes a held session and never creates one', async () => {
    const sid = uid('brain');
    callHub.mockResolvedValue([brainSparseRow(sid)]);
    await emitAndFlush({ type: 'agent.snapshot', hub: 'brainpc', data: brainSparseRow(sid) });
    expect(claudeSessionStore.getSnapshot(sid)).not.toBeNull();

    sent.length = 0;
    await emitAndFlush({
      type: 'agent.snapshot',
      hub: 'brainpc',
      data: brainSparseRow(sid, { status: 'ended', ambientState: 'idle', mode: 'stopped' }),
    });
    expect(claudeSessionStore.getSnapshot(sid)).toBeNull();
    const final = sent.find((s) => s.sessionId === sid);
    expect(final).toBeDefined();
    expect(final!.snapshot.status).toBe('ended');

    // A layout-ghost stopped row for a session never held stays invisible.
    const ghost = uid('ghost');
    await emitAndFlush({
      type: 'agent.snapshot',
      hub: 'brainpc',
      data: {
        sessionId: ghost,
        sparse: true,
        status: 'ended',
        cwd: '',
        label: 'old agent',
        ambientState: 'idle',
        pendingApproval: null,
        pendingQuestions: null,
      },
    });
    expect(claudeSessionStore.getSnapshot(ghost)).toBeNull();
  });

  it('the reseed keeps live sparse rows and skips ended ones', async () => {
    const live = uid('brain');
    const ghost = uid('ghost');
    callHub.mockResolvedValue([
      brainSparseRow(live),
      {
        sessionId: ghost,
        sparse: true,
        status: 'ended',
        cwd: '',
        label: 'old agent',
        ambientState: 'idle',
        pendingApproval: null,
        pendingQuestions: null,
      },
    ]);
    await emitAndFlush({ type: 'hub.peer.connected', data: { peer: 'brainpc' } });
    expect(claudeSessionStore.getSnapshot(live)).not.toBeNull();
    expect(claudeSessionStore.getSnapshot(live)!.hub).toBe('brainpc');
    expect(claudeSessionStore.getSnapshot(ghost)).toBeNull();
  });
});

describe('security — remote snapshots grant no local fs roots', () => {
  it('snapshotGrantsFsRoot refuses a hub-stamped snapshot in any live state', () => {
    // The same row WITHOUT the stamp is live (that is what makes this a test
    // of the hub clause and not of some other refusal).
    expect(snapshotGrantsFsRoot({ cwd: '/peer/proj', status: 'active' })).toBe(true);
    expect(snapshotGrantsFsRoot({ cwd: '/peer/proj', status: 'active', hub: 'work' })).toBe(false);
    expect(snapshotGrantsFsRoot({ cwd: '/peer/proj', mode: 'input', hub: 'work' })).toBe(false);
    // Fail closed on a wrong-typed stamp too ("will not decode" clause).
    expect(snapshotGrantsFsRoot({ cwd: '/peer/proj', status: 'active', hub: 42 })).toBe(false);
    // Empty string means local — unchanged verdict.
    expect(snapshotGrantsFsRoot({ cwd: '/p', status: 'active', hub: '' })).toBe(true);
  });

  it('an ingested remote session is refused by the fs-root predicate end to end', async () => {
    const sid = uid('remote');
    callHub.mockResolvedValue([remoteSnap(sid)]);
    await emitAndFlush({ type: 'agent.snapshot', hub: 'work', data: remoteSnap(sid) });
    const snap = claudeSessionStore.getSnapshot(sid)!;
    expect(snap.cwd).toBe('/peer/proj');
    expect(snapshotGrantsFsRoot(snap)).toBe(false);
  });
});
