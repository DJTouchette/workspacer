/**
 * Succession after a CRASH: the dead-manager tombstone.
 *
 * `adopt_workers` (agents.reparent → reparentChildren) needs the id of the
 * manager it is replacing. A manager that hands over writes it into a handoff
 * file; a manager that CRASHES writes nothing, and ~30 s after its SessionEnd
 * the store evicts its row — so the only trace left was a dangling
 * `parentSessionId` on the live workers, which proves nothing about the parent
 * (manager? worker with subagents? which of two?).
 *
 * The tombstone is the missing half: at eviction, a session marked
 * `isSupervisor` leaves behind its id / label / cwd / time of death, so the
 * successor can be TOLD which dangling parents were managers instead of
 * guessing. These tests pin what it must and must not do — most importantly
 * that it is a forensic record and NEVER a wake destination, and that a
 * candidate list is reported rather than one being picked.
 *
 * Strategy: drive the real store with the hook traffic a real life produces,
 * mock every side-effect collaborator, and run the 30 s eviction on fake timers.
 */

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';

vi.mock('electron', () => ({ BrowserWindow: class {} }));
vi.mock('./agentNotifier', () => ({ agentNotifier: { notifyOnTransition: vi.fn() } }));
vi.mock('./supervisorNudge', () => ({
  supervisorNudge: {
    onBlock: vi.fn(),
    onBlockCleared: vi.fn(),
    onFinished: vi.fn(),
    sweepMissedFinishes: vi.fn(),
    forgetWorker: vi.fn(),
    reassignPendingFinish: vi.fn(),
  },
}));
vi.mock('./workflowWatcher', () => ({
  workflowWatcher: { attach: vi.fn(), detach: vi.fn(), poke: vi.fn() },
}));
vi.mock('./hubTelemetry', () => ({
  publishWorkflowRuns: vi.fn(),
  publishSnapshot: vi.fn(),
  forgetSession: vi.fn(),
}));
vi.mock('./claudemonDaemon', () => ({ CLAUDEMON_API_URL: 'http://127.0.0.1:0' }));
vi.mock('./sessionStore/analyticsWriter', () => ({ writeHistory: vi.fn() }));
vi.mock('./remoteTokens', () => ({ revokeSessionFacadeTokens: vi.fn() }));

import { claudeSessionStore } from './claudeSessionStore';

let seq = 0;
const uid = (p: string): string => `${p}-tomb-${++seq}`;

function hook(sessionId: string, hookName: string, cwd = '/proj'): void {
  claudeSessionStore.handleHookEvent({
    hook_event_name: hookName,
    session_id: sessionId,
    cwd,
  });
}

/** A live Fleet Manager — what spawn_agent({manager: true}) records. */
function manager(id: string, label = 'Fleet Manager', cwd = '/home/me/Work'): string {
  claudeSessionStore.setSpawnMeta(id, { label, isSupervisor: true });
  hook(id, 'SessionStart', cwd);
  return id;
}

/** A worker dispatched under `parent`. */
function worker(id: string, parent: string, label = 'alpha: ship it', cwd = '/proj'): string {
  claudeSessionStore.setSpawnMeta(id, { label, parentSessionId: parent });
  hook(id, 'SessionStart', cwd);
  return id;
}

/** A session dies the way a crash looks to the store, and is then evicted. */
function dieAndEvict(id: string): void {
  hook(id, 'SessionEnd');
  vi.advanceTimersByTime(31_000);
}

function candidateFor(id: string) {
  return claudeSessionStore.orphanCandidates().find((c) => c.sessionId === id);
}

beforeEach(() => {
  vi.useFakeTimers();
});

afterEach(() => {
  vi.useRealTimers();
});

describe('a dead manager leaves a tombstone its successor can identify', () => {
  it('names the evicted manager — label, cwd, time of death — and its live children', () => {
    const dead = uid('mgr');
    const w1 = uid('w');
    const w2 = uid('w');
    manager(dead, 'Fleet Manager', '/home/me/Work');
    worker(w1, dead, 'rivet: tombstone', '/proj/a');
    worker(w2, dead, 'rivet: parity', '/proj/b');

    dieAndEvict(dead);

    // The row itself is gone — that is the hole the tombstone fills.
    expect(claudeSessionStore.getSnapshot(dead)).toBeFalsy();

    const candidate = candidateFor(dead);
    expect(candidate, 'the evicted manager must still be identifiable').toBeTruthy();
    expect(candidate!.confirmedManager).toBe(true);
    expect(candidate!.label).toBe('Fleet Manager');
    expect(candidate!.cwd).toBe('/home/me/Work');
    expect(candidate!.endedAt).toBeTypeOf('number');
    expect(candidate!.children.map((c) => c.sessionId).sort()).toEqual([w1, w2].sort());
  });

  it('a dead parent that was NOT a manager is reported, but not as a confirmed one', () => {
    // A worker can itself spawn agents (parentSessionId of a non-supervisor).
    // Its dangling id looked exactly like a dead manager's before the
    // tombstone; it must now be visibly distinguishable, not silently equal.
    const deadWorker = uid('w');
    const child = uid('w');
    claudeSessionStore.setSpawnMeta(deadWorker, { label: 'not a manager' });
    hook(deadWorker, 'SessionStart');
    worker(child, deadWorker);

    dieAndEvict(deadWorker);

    const candidate = candidateFor(deadWorker);
    expect(candidate, 'a dangling parent is still worth reporting').toBeTruthy();
    expect(candidate!.confirmedManager).toBe(false);
    expect(candidate!.label, 'nothing survived eviction to name it').toBeNull();
  });

  it('reports EVERY dead manager with live children rather than picking one', () => {
    // The whole objection to automatic adoption: with two dangling groups, a
    // guess is silent and wrong half the time. The read must stay a list.
    const deadA = uid('mgr');
    const deadB = uid('mgr');
    manager(deadA, 'Manager A', '/work/a');
    manager(deadB, 'Manager B', '/work/b');
    worker(uid('w'), deadA);
    worker(uid('w'), deadB);

    dieAndEvict(deadA);
    dieAndEvict(deadB);

    const ids = claudeSessionStore.orphanCandidates().map((c) => c.sessionId);
    expect(ids).toContain(deadA);
    expect(ids).toContain(deadB);
  });

  it('a LIVE manager is never a candidate — it is not orphaning anyone', () => {
    const live = uid('mgr');
    manager(live);
    worker(uid('w'), live);

    expect(candidateFor(live)).toBeFalsy();
  });
});

describe('the tombstone is a record, never a route', () => {
  it('a tombstoned manager cannot be adopted TO — reparent still refuses a dead successor', () => {
    // If the successor lookup ever consulted tombstones, workers would be
    // re-pointed at a session no wake can reach: the failure reparentChildren
    // exists to refuse, re-introduced by the fix for the opposite problem.
    const dead = uid('mgr');
    manager(dead);
    const orphan = worker(uid('w'), dead);
    dieAndEvict(dead);

    const live = uid('mgr');
    manager(live);
    worker(uid('w'), live);

    expect(() => claudeSessionStore.reparentChildren(live, dead)).toThrow(/no session/);
    expect(
      claudeSessionStore.getSnapshot(orphan)?.parentSessionId,
      'the refused move must not have half-happened',
    ).toBe(dead);
  });

  it('adopting the orphans retires the candidate', () => {
    const dead = uid('mgr');
    manager(dead);
    const orphan = worker(uid('w'), dead);
    dieAndEvict(dead);

    const successor = uid('mgr');
    manager(successor);

    expect(candidateFor(dead), 'before adoption it is the answer to "who was mine"').toBeTruthy();
    claudeSessionStore.reparentChildren(dead, successor);
    expect(claudeSessionStore.getSnapshot(orphan)?.parentSessionId).toBe(successor);
    expect(candidateFor(dead), 'adopted workers leave nothing orphaned').toBeFalsy();
  });
});

describe('retention: a tombstone map that only grows is a leak', () => {
  // These count tombstones rather than read candidates on purpose. A childless
  // tombstone is INVISIBLE through orphanCandidates — candidates are derived
  // from the children, so a parent with none is simply absent — which means an
  // unpruned map behaves identically from the outside while growing once per
  // manager for the process's whole life. Only the count can see it.
  it('drops a tombstone once nothing it parented is still around', () => {
    const before = claudeSessionStore.managerTombstoneCount();
    const dead = uid('mgr');
    manager(dead);
    const only = worker(uid('w'), dead);
    dieAndEvict(dead);
    expect(candidateFor(dead)).toBeTruthy();
    expect(claudeSessionStore.managerTombstoneCount()).toBe(before + 1);

    // The last worker finishes and is itself evicted: there is no longer a
    // question the tombstone can answer, and nothing to keep it for.
    dieAndEvict(only);
    expect(candidateFor(dead), 'a childless tombstone answers nothing').toBeFalsy();
    expect(
      claudeSessionStore.managerTombstoneCount(),
      'a tombstone with no live children must be released',
    ).toBe(before);
  });

  it('is bounded even when every dead manager keeps a live child', () => {
    const ids: string[] = [];
    for (let i = 0; i < 40; i++) {
      const dead = uid('mgr');
      ids.push(dead);
      manager(dead, `Manager ${i}`);
      worker(uid('w'), dead);
      dieAndEvict(dead);
    }
    expect(claudeSessionStore.managerTombstoneCount()).toBeLessThanOrEqual(32);
    const candidates = claudeSessionStore.orphanCandidates().filter((c) => c.confirmedManager);
    expect(candidates.length).toBeLessThanOrEqual(32);
    // The cap must drop the OLDEST, not the newest — the successor spawning
    // now is replacing one of the recent deaths.
    expect(candidates.map((c) => c.sessionId)).toContain(ids[ids.length - 1]);
  });
});
