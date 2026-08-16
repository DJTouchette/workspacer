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
 *   - security: snapshotGrantsFsRoot refuses any hub-stamped snapshot, so a
 *     remote cwd can never enter the fs.* allow-list (workspaceRoots).
 *
 * Strategy (mirrors claudeSessionStore.test.ts): mock every side-effect
 * collaborator, drive the REAL store through the bridge by capturing the
 * listener it registers with subscribeHubEvents, and flush the ~16 ms
 * coalesced snapshot pushes with fake timers.
 */

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';

const { listeners, callHub, publishSnapshot } = vi.hoisted(() => ({
  listeners: new Set<(ev: unknown) => void>(),
  callHub: vi.fn(async (): Promise<unknown> => []),
  publishSnapshot: vi.fn(),
}));

vi.mock('electron', () => ({ BrowserWindow: class {} }));
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
