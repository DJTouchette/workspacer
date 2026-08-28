/**
 * Fleet wakes are PARENT-KEYED, so replacing a Fleet Manager used to orphan
 * every dispatch it had in flight — the successor could never receive their
 * reports. `claudeSessionStore.reparentChildren` re-points the routing key.
 *
 * These drive the REAL store and the REAL supervisorNudge with the hook +
 * conversation-delta traffic a live child produces (same rig as
 * perTurnWake.repro.test.ts); only claudemonSessionClient.message — the wire
 * the wake goes out on — is mocked, because who receives it is the whole
 * question.
 */

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';

const message = vi.fn().mockResolvedValue(undefined);
vi.mock('./claudemonSessionClient', () => ({
  claudemonSessionClient: { message: (...a: unknown[]) => message(...a) },
}));
vi.mock('electron', () => ({ BrowserWindow: class {} }));
vi.mock('./agentNotifier', () => ({ agentNotifier: { notifyOnTransition: vi.fn() } }));
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

import { claudeSessionStore } from './claudeSessionStore';
import { supervisorNudge } from './supervisorNudge';
import { parseFleetMessage } from '../shared/fleetMessages';

let seq = 0;
const uid = (p: string): string => `${p}-reparent-${++seq}`;

function hook(sessionId: string, hookName: string, cwd = '/proj'): void {
  claudeSessionStore.handleHookEvent({ hook_event_name: hookName, session_id: sessionId, cwd });
}

const convCursor = new Map<string, number>();
function say(sessionId: string, role: 'user' | 'assistant', text: string): void {
  const n = (convCursor.get(sessionId) ?? 0) + 1;
  convCursor.set(sessionId, n);
  claudeSessionStore.applyConversationDelta({
    session_id: sessionId,
    seq: n,
    reset: false,
    items: [{ type: role === 'user' ? 'user_message' : 'assistant_text', text }] as never,
  });
}

/** A full child turn as it really arrives: prompt, hook flip to working,
 *  reply, then Stop (the working→idle edge that wakes the parent). */
function turn(child: string, prompt: string, reply: string): void {
  say(child, 'user', prompt);
  hook(child, 'UserPromptSubmit');
  say(child, 'assistant', reply);
  hook(child, 'Stop');
}

/** A live Fleet Manager — what spawn_agent({manager: true}) records. */
function manager(id: string, label = 'Fleet Manager'): string {
  claudeSessionStore.setSpawnMeta(id, { label, isWakeTarget: true });
  hook(id, 'SessionStart');
  return id;
}

/** A worker dispatched under `parent`. */
function worker(id: string, parent: string, label = 'alpha: ship it'): string {
  claudeSessionStore.setSpawnMeta(id, { label, parentSessionId: parent });
  hook(id, 'SessionStart');
  return id;
}

/** Every id a wake was delivered to during this test. */
function wakeTargets(): string[] {
  return (message.mock.calls as Array<[string, string]>).map(([target]) => target);
}

beforeEach(() => {
  vi.useFakeTimers();
  message.mockClear();
});
afterEach(() => vi.useRealTimers());

describe('reparentChildren re-points fleet wakes at the successor', () => {
  it('a worker dispatched by the outgoing manager reports to the NEW one', async () => {
    const outgoing = manager(uid('mgr-out'));
    const child = worker(uid('child'), outgoing);
    const successor = manager(uid('mgr-in'), 'Fleet Manager (successor)');

    expect(claudeSessionStore.reparentChildren(outgoing, successor)).toEqual({
      moved: [child],
      pending: [],
    });

    turn(child, 'ship the thing', 'Shipped — 3 commits on the branch.');
    await vi.advanceTimersByTimeAsync(3000);

    expect(wakeTargets()).toEqual([successor]);
    const wake = parseFleetMessage(message.mock.calls[0][1] as string);
    expect(wake?.kind).toBe('worker-finished');
    expect(message.mock.calls[0][1]).toContain('Shipped');
  });

  it('is allowed while the outgoing manager is still alive — that IS the handoff', async () => {
    // The outgoing manager is mid-turn when it hands over (it is the one asking),
    // so a guard on "old parent still live" would refuse the only real case.
    const outgoing = manager(uid('mgr-out'));
    const child = worker(uid('child'), outgoing);
    const successor = manager(uid('mgr-in'));
    hook(outgoing, 'UserPromptSubmit'); // outgoing manager is working right now

    expect(() => claudeSessionStore.reparentChildren(outgoing, successor)).not.toThrow();

    turn(child, 'finish up', 'Done.');
    await vi.advanceTimersByTimeAsync(3000);

    // Routing moved COMPLETELY: the retiring manager hears nothing further.
    expect(wakeTargets()).toEqual([successor]);
  });

  it('moves a dispatch that has not registered yet (spawned, no hook)', async () => {
    const outgoing = manager(uid('mgr-out'));
    const successor = manager(uid('mgr-in'));
    // Spawn recorded, but the worker's first hook has not landed — the most
    // orphan-prone worker of all, its whole life still ahead of it.
    const child = uid('child');
    claudeSessionStore.setSpawnMeta(child, { label: 'beta: build', parentSessionId: outgoing });

    expect(claudeSessionStore.reparentChildren(outgoing, successor)).toEqual({
      moved: [],
      pending: [child],
    });

    hook(child, 'SessionStart');
    turn(child, 'build it', 'Built.');
    await vi.advanceTimersByTimeAsync(3000);

    expect(wakeTargets()).toEqual([successor]);
  });

  it('re-addresses a wake already queued in the coalesce window', async () => {
    const outgoing = manager(uid('mgr-out'));
    const child = worker(uid('child'), outgoing);
    const successor = manager(uid('mgr-in'));

    // The worker finishes seconds BEFORE the handoff: its wake is scheduled,
    // addressed to the manager on its way out, but not yet delivered.
    turn(child, 'run the tests', 'All 42 pass.');
    await vi.advanceTimersByTimeAsync(500);
    expect(message).not.toHaveBeenCalled();

    claudeSessionStore.reparentChildren(outgoing, successor);
    await vi.advanceTimersByTimeAsync(3000);

    expect(wakeTargets()).toEqual([successor]);
    expect(message.mock.calls[0][1]).toContain('All 42 pass.');
  });

  it('the dropped-wake backstop follows the new parent too', async () => {
    const outgoing = manager(uid('mgr-out'));
    const child = worker(uid('child'), outgoing);
    const successor = manager(uid('mgr-in'));
    // The sweep only catches up a manager that has not acted since the child
    // finished, so the child's finish must land after the successor's own boot.
    await vi.advanceTimersByTimeAsync(1000);
    turn(child, 'do it', 'Done.');
    claudeSessionStore.reparentChildren(outgoing, successor);
    message.mockClear();

    // sweepMissedFinishes matches c.parentSessionId === manager.sessionId, so
    // it needs no re-pointing of its own — it re-reads the field we moved.
    const sessions = claudeSessionStore.getAllSnapshots();
    const finished = sessions.find((s) => s.sessionId === child)!;
    supervisorNudge.sweepMissedFinishes(sessions, finished.lastActivity + 10 * 60_000);

    expect(wakeTargets()).toEqual([successor]);
    expect(parseFleetMessage(message.mock.calls[0][1] as string)?.kind).toBe('catch-up');
  });

  it('never makes the successor its own parent', async () => {
    const outgoing = manager(uid('mgr-out'));
    // The successor is usually dispatched BY the manager it replaces.
    const successor = uid('mgr-in');
    claudeSessionStore.setSpawnMeta(successor, {
      label: 'Fleet Manager (successor)',
      parentSessionId: outgoing,
      isWakeTarget: true,
    });
    hook(successor, 'SessionStart');

    expect(claudeSessionStore.reparentChildren(outgoing, successor).moved).toEqual([]);
    expect(
      claudeSessionStore.getAllSnapshots().find((s) => s.sessionId === successor)!.parentSessionId,
    ).toBe(outgoing);

    // …and it is never woken about itself.
    turn(successor, 'take over', 'Taking over.');
    await vi.advanceTimersByTimeAsync(3000);
    expect(wakeTargets()).not.toContain(successor);
  });
});

describe('reparentChildren refuses a successor no wake could reach', () => {
  it('refuses an unknown session, leaving routing untouched', async () => {
    const outgoing = manager(uid('mgr-out'));
    const child = worker(uid('child'), outgoing);

    expect(() => claudeSessionStore.reparentChildren(outgoing, uid('ghost'))).toThrow(/no session/);

    turn(child, 'carry on', 'Carried on.');
    await vi.advanceTimersByTimeAsync(3000);
    expect(wakeTargets()).toEqual([outgoing]);
  });

  it('refuses an ENDED successor — a dead parent orphans exactly as before', () => {
    const outgoing = manager(uid('mgr-out'));
    worker(uid('child'), outgoing);
    const successor = manager(uid('mgr-in'));
    hook(successor, 'SessionEnd');

    expect(() => claudeSessionStore.reparentChildren(outgoing, successor)).toThrow(/has ended/);
  });

  it('refuses a successor that is not a manager — wakes need isWakeTarget', () => {
    const outgoing = manager(uid('mgr-out'));
    worker(uid('child'), outgoing);
    const plain = worker(uid('plain'), outgoing, 'just a worker');

    // Silently re-pointing here would SILENCE every dispatch rather than
    // reroute it: nudgeParentOnFinish requires a supervisor parent.
    expect(() => claudeSessionStore.reparentChildren(outgoing, plain)).toThrow(/not a manager/);
  });

  it('refuses the no-op and the missing argument', () => {
    const mgr = manager(uid('mgr'));
    expect(() => claudeSessionStore.reparentChildren(mgr, mgr)).toThrow(/already the parent/);
    expect(() => claudeSessionStore.reparentChildren('', mgr)).toThrow(/required/);
    expect(() => claudeSessionStore.reparentChildren(mgr, '')).toThrow(/required/);
  });
});
