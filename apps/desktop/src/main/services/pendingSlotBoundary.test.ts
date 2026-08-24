/**
 * The pending slot past the STORE's file boundary.
 *
 * `pendingSlot.ts` fences the store's own class body and the hook router's:
 * inside them the slot is readonly, so every park and resolve must state which
 * feed it is. Two ways out of that fence were left open, and neither is broken
 * today — they are both ways a future edit reopens the class:
 *
 *   1. `this.sessions.values()` hands LIVE rows to six collaborators outside
 *      the file (the notifier, the supervisor nudger, the usage accumulator,
 *      the conversation applier, the budget watcher, the analytics writer).
 *      They only read the slot; nothing but code review said they must.
 *   2. `getSnapshot` / `getAllSnapshots` SHALLOW-clone, so `snap.pendingQuestions`
 *      was the store's own array. `snap.pendingQuestions.push(…)` reached
 *      straight past every fence — no assignment, no readonly, no gate.
 *
 * (1) is a type fact and its real proof is a compile error — see the
 * `PendingReadOnlySession` docblock. What is testable at runtime is the
 * BEHAVIOUR the type now pins: hand each collaborator a session with a card
 * parked, and the card must survive. Those cases fail against a mutant
 * collaborator that clears the slot, which is what the fence exists to stop.
 *
 * (2) is a runtime fact and fails outright without the deep copy.
 */

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';

vi.mock('electron', () => ({ BrowserWindow: class {}, Notification: class {}, shell: {} }));
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

import { publishSnapshot } from './hubTelemetry';
import { claudeSessionStore } from './claudeSessionStore';
import type { ClaudeSessionState, PendingQuestion } from './claudeSessionStore';
import { checkBudget } from './budgetWatcher';
import { writeHistory } from './sessionStore/analyticsWriter';
import { SessionUsageAccumulator } from './sessionStore/usageAccumulator';
import { applyConversationItems } from './sessionStore/conversationApplier';

let seq = 0;
const uid = (): string => `fence-${++seq}`;

function hook(sessionId: string, event: Record<string, unknown>): void {
  claudeSessionStore.handleHookEvent({ session_id: sessionId, cwd: '/proj', ...event });
}

/** A live claude/PTY session — the hook feed owns its slot — with a question
 *  picker parked on it, exactly as an AskUserQuestion tool call leaves it. */
function sessionBlockedOnAQuestion(): string {
  const id = uid();
  hook(id, { hook_event_name: 'SessionStart' });
  hook(id, {
    hook_event_name: 'PreToolUse',
    tool_name: 'AskUserQuestion',
    tool_use_id: `tu-${id}`,
    tool_input: {
      questions: [{ question: 'Ship it?', header: 'Ship', options: [{ label: 'Yes' }] }],
    },
  });
  return id;
}

/** A live claude/PTY session blocked on a tool approval. */
function sessionBlockedOnAnApproval(): string {
  const id = uid();
  hook(id, { hook_event_name: 'SessionStart' });
  hook(id, {
    hook_event_name: 'PermissionRequest',
    tool_name: 'Bash',
    tool_input: { command: 'rm -rf /' },
  });
  return id;
}

/**
 * What the STORE holds, re-read. Deliberately a second `getSnapshot` rather
 * than a test-only accessor for the live row: if the slot were still aliased,
 * mutating one snapshot would show up in the next one, so this read is exactly
 * the observation the test needs — and it needs no production seam to make it.
 */
function asStored(sessionId: string) {
  const snap = claudeSessionStore.getSnapshot(sessionId);
  expect(snap, 'the store must still hold this row').toBeTruthy();
  return snap!;
}

beforeEach(() => {
  vi.useFakeTimers();
});
afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
});

describe('a snapshot never aliases the store’s pending slot', () => {
  it('mutating snap.pendingQuestions in place does not reach the live row', () => {
    const id = sessionBlockedOnAQuestion();
    const snap = claudeSessionStore.getSnapshot(id)!;
    expect(snap.pendingQuestions).toHaveLength(1);

    // The whole point: no assignment, no readonly violation — just `push` on
    // an array the caller was handed. Before the deep copy this WAS the store's
    // array, so the live row grew a second question nobody parked.
    (snap.pendingQuestions as PendingQuestion[]).push({
      question: 'smuggled',
      options: [],
    } as PendingQuestion);
    (snap.pendingQuestions as PendingQuestion[])[0].question = 'rewritten';

    expect(asStored(id).pendingQuestions).toHaveLength(1);
    expect(asStored(id).pendingQuestions![0].question).toBe('Ship it?');
  });

  it('emptying snap.pendingQuestions in place does not unblock the live session', () => {
    // The freeze shape, arrived at without ever naming the field: a caller
    // "clears" what it thinks is its own copy and the real card vanishes with
    // the session still blocked on it.
    const id = sessionBlockedOnAQuestion();
    const snap = claudeSessionStore.getSnapshot(id)!;
    (snap.pendingQuestions as PendingQuestion[]).length = 0;

    expect(asStored(id).pendingQuestions).toHaveLength(1);
    expect(claudeSessionStore.getSnapshot(id)!.pendingQuestions).toHaveLength(1);
  });

  it('mutating snap.pendingApproval does not rewrite the card the user is deciding on', () => {
    const id = sessionBlockedOnAnApproval();
    const snap = claudeSessionStore.getSnapshot(id)!;
    expect(snap.pendingApproval?.toolName).toBe('Bash');

    const card = snap.pendingApproval as { toolName: string; toolInput: { command: string } };
    card.toolName = 'Read';
    card.toolInput.command = 'ls';

    // A rewritten card is worse than a cleared one: the Approve button still
    // works, it just approves something other than what is shown.
    expect(asStored(id).pendingApproval!.toolName).toBe('Bash');
    expect(asStored(id).pendingApproval!.toolInput.command).toBe('rm -rf /');
  });

  it('getAllSnapshots detaches the slot on every row, not just the one asked for', () => {
    const a = sessionBlockedOnAQuestion();
    const b = sessionBlockedOnAnApproval();
    for (const snap of claudeSessionStore.getAllSnapshots()) {
      if (snap.pendingQuestions) (snap.pendingQuestions as PendingQuestion[]).length = 0;
      if (snap.pendingApproval) snap.pendingApproval.toolName = 'clobbered';
    }
    expect(asStored(a).pendingQuestions).toHaveLength(1);
    expect(asStored(b).pendingApproval!.toolName).toBe('Bash');
  });

  it('still hands back the real values — the copy is a copy, not a blank', () => {
    const id = sessionBlockedOnAnApproval();
    const snap = claudeSessionStore.getSnapshot(id)!;
    expect(snap.pendingApproval).toEqual(asStored(id).pendingApproval);
    expect(snap.pendingApproval).not.toBe(asStored(id).pendingApproval);

    const q = sessionBlockedOnAQuestion();
    const qsnap = claudeSessionStore.getSnapshot(q)!;
    expect(qsnap.pendingQuestions).toEqual(asStored(q).pendingQuestions);
    expect(qsnap.pendingQuestions).not.toBe(asStored(q).pendingQuestions);
    // Deep, not one level: the option objects are copies too.
    expect(qsnap.pendingQuestions![0].options).not.toBe(asStored(q).pendingQuestions![0].options);
  });

  it('the copy mirrored onto the hub bus is detached too', () => {
    // `publishSnapshot` takes a FACTORY so the spread is skipped when nothing
    // is listening — which means the snapshot it would build is only reachable
    // by invoking the recorded factory here.
    const id = sessionBlockedOnAnApproval();
    vi.advanceTimersByTime(20); // snapshot pushes are coalesced on a 16 ms timer
    const calls = vi.mocked(publishSnapshot).mock.calls;
    const make = calls[calls.length - 1]?.[0];
    expect(make, 'a live local row must have been published').toBeTypeOf('function');

    const busSnap = make!();
    (busSnap.pendingApproval as { toolName: string }).toolName = 'clobbered';
    expect(asStored(id).pendingApproval!.toolName).toBe('Bash');
  });

  it('a session with nothing parked still reports null, not an empty copy', () => {
    const id = uid();
    hook(id, { hook_event_name: 'SessionStart' });
    const snap = claudeSessionStore.getSnapshot(id)!;
    expect(snap.pendingApproval).toBeNull();
    expect(snap.pendingQuestions).toBeNull();
  });
});

describe('the collaborators holding a live row leave the slot alone', () => {
  // The behaviour half of the `PendingReadOnlySession` fence. Each of these
  // modules is handed the store's own mutable row by `this.sessions.values()`;
  // the type is what makes a write to the slot impossible, and these pin that
  // none of them already does it. They fail against a mutant collaborator that
  // clears the card — which is the freeze this whole fence exists to stop.

  function blockedRow(over: Partial<ClaudeSessionState> = {}): ClaudeSessionState {
    return {
      sessionId: 'collab-1',
      cwd: '/proj',
      conversation: [],
      activeToolCalls: [],
      completedToolCalls: [],
      fileChanges: [],
      subagents: [],
      workflows: [],
      ambientState: 'waiting_approval',
      status: 'active',
      totalToolCalls: 0,
      startTime: 1,
      lastActivity: 1,
      pendingApproval: { toolName: 'Bash', toolInput: { command: 'rm -rf /' }, timestamp: 1 },
      pendingQuestions: [{ question: 'Ship it?', options: [{ label: 'Yes' }] }],
      ...over,
    } as unknown as ClaudeSessionState;
  }

  function expectSlotIntact(s: ClaudeSessionState): void {
    expect(s.pendingApproval?.toolName).toBe('Bash');
    expect(s.pendingQuestions).toHaveLength(1);
  }

  it('the budget watcher reads a blocked session without touching its card', () => {
    const s = blockedRow();
    checkBudget(s);
    expectSlotIntact(s);
  });

  it('the analytics writer records a blocked session without resolving it', () => {
    const s = blockedRow();
    writeHistory(s, 'active');
    expectSlotIntact(s);
  });

  it('the usage accumulator enriches usage without clearing the card', () => {
    const s = blockedRow();
    const acc = new SessionUsageAccumulator();
    acc.applyUsage(s, 'claude-opus-5', { input_tokens: 10, output_tokens: 5 }, 'k1');
    SessionUsageAccumulator.refreshContextLimit(s);
    expect(s.usage).toBeTruthy();
    expectSlotIntact(s);
  });

  it('the conversation applier folds in an ordinary turn without clearing the card', () => {
    const s = blockedRow({ ambientState: 'streaming' });
    applyConversationItems(
      s,
      [{ kind: 'assistant_text', text: 'working on it' }] as never,
      () => {},
    );
    expect(s.conversation.length).toBeGreaterThan(0);
    expectSlotIntact(s);
  });

  it('the applier DOES still resolve the slot on an interrupt — through the gate', () => {
    // Not a counterexample to the fence: this clear goes through
    // `applyStopEvent`, which builds a `PendingSlot` declaring the hook feed.
    // A hook-owned session is cleared; the point of the fence is that the
    // applier cannot do it any OTHER way.
    const s = blockedRow({ ambientState: 'streaming' });
    applyConversationItems(
      s,
      [{ kind: 'user_message', text: '[Request interrupted by user]' }] as never,
      () => {},
    );
    expect(s.pendingApproval).toBeNull();
  });

  it('…and cannot resolve the slot of a session it does NOT own', () => {
    // Same interrupt, on a daemon-owned row (a peer-mirrored one here). The
    // gate refuses: nothing local holds that request, so clearing it would
    // leave the peer blocked with nothing to answer.
    const s = blockedRow({ ambientState: 'streaming', hub: 'laptop' });
    applyConversationItems(
      s,
      [{ kind: 'user_message', text: '[Request interrupted by user]' }] as never,
      () => {},
    );
    expectSlotIntact(s);
  });
});
