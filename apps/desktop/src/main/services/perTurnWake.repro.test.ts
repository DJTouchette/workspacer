/**
 * REPRODUCTION (2026-08-22) — "a conversational child wakes its manager once
 * per turn". Settles a claim made by code reading alone.
 *
 * This drives the REAL claudeSessionStore and the REAL supervisorNudge with the
 * hook + conversation-delta traffic a live child session produces. The only
 * mocked boundary is claudemonSessionClient.message — the wire the wake goes
 * out on — which is precisely what we are counting.
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
import { parseFleetMessage } from '../shared/fleetMessages';

let seq = 0;
const uid = (p: string): string => `${p}-repro-${++seq}`;

function hook(sessionId: string, hookName: string, cwd = '/proj'): void {
  claudeSessionStore.handleHookEvent({ hook_event_name: hookName, session_id: sessionId, cwd });
}

/** One conversation frame, in the daemon's delta shape. `n` is the running
 *  item count for this session (the store gap-checks seq against it). */
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

/** A full child turn as it really arrives: the prompt, the hook flip to
 *  working, the reply, then Stop (working→idle). */
function turn(child: string, prompt: string, reply: string): void {
  say(child, 'user', prompt);
  hook(child, 'UserPromptSubmit');
  say(child, 'assistant', reply);
  hook(child, 'Stop');
}

beforeEach(() => {
  vi.useFakeTimers();
  message.mockClear();
});
afterEach(() => vi.useRealTimers());

describe('REPRO: worker-finished wake fires on EVERY working→idle edge', () => {
  it('a child conversed with three times wakes its manager three times', async () => {
    const mgr = uid('mgr');
    const child = uid('child');

    // A live Fleet Manager, and a worker dispatched under it — exactly what
    // spawn_agent({parentSessionId: mgr}) records.
    claudeSessionStore.setSpawnMeta(mgr, { label: 'Fleet Manager', isSupervisor: true });
    hook(mgr, 'SessionStart');
    claudeSessionStore.setSpawnMeta(child, { label: 'alpha: fix tests', parentSessionId: mgr });
    hook(child, 'SessionStart');

    // Turn 1 — the dispatch itself. One wake is CORRECT here.
    turn(child, 'fix the failing tests', 'Fixed. All 42 pass.');
    await vi.advanceTimersByTimeAsync(3000);
    expect(message).toHaveBeenCalledTimes(1);

    // Turn 2 — the user opens the worker's pane and asks a question. Nothing
    // finished; this is conversation.
    turn(child, 'which test was flaky?', 'The retry one in api.test.ts.');
    await vi.advanceTimersByTimeAsync(3000);

    // Turn 3 — another conversational exchange.
    turn(child, 'why did it flake?', 'A 200ms timeout that CI trips.');
    await vi.advanceTimersByTimeAsync(3000);

    // THE BUG, if it is real: three edges, three wakes at the manager.
    expect(message).toHaveBeenCalledTimes(3);
    for (const [target, text] of message.mock.calls as Array<[string, string]>) {
      expect(target).toBe(mgr);
      expect(parseFleetMessage(text)?.kind).toBe('worker-finished');
    }
    // …and each one carries the tail telling the manager to go edit a brief.
    expect(message.mock.calls[2][1]).toContain('brief.md');
    // The last wake reports a mid-conversation answer as if it were a result.
    expect(message.mock.calls[2][1]).toContain('A 200ms timeout that CI trips.');
  });

  it('the same edge from an approval blip (no new prompt) also re-wakes', async () => {
    const mgr = uid('mgr');
    const child = uid('child');
    claudeSessionStore.setSpawnMeta(mgr, { label: 'Fleet Manager', isSupervisor: true });
    hook(mgr, 'SessionStart');
    claudeSessionStore.setSpawnMeta(child, { label: 'beta: build', parentSessionId: mgr });
    hook(child, 'SessionStart');

    turn(child, 'run the release build', 'Claude needs your permission to run Bash');
    await vi.advanceTimersByTimeAsync(3000);
    expect(message).toHaveBeenCalledTimes(1);

    // The block clears, the agent resumes and stops again — no user turn at
    // all, and the reply has not changed. Observed live 2026-08-21 16:16:32
    // and 16:16:43 in the fleet-manager transcript, 11 s apart.
    hook(child, 'PreToolUse');
    hook(child, 'Stop');
    await vi.advanceTimersByTimeAsync(3000);
    expect(message).toHaveBeenCalledTimes(2);
    expect(message.mock.calls[1][1]).toContain('Claude needs your permission');
  });
});
