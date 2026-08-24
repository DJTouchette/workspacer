import { describe, it, expect } from 'vitest';

import {
  acknowledgeAnswer,
  bornWithEmptyPending,
  bornWithPending,
  PendingSlot,
  pendingSlotOwner,
} from './pendingSlot';
import type { SessionWithoutPending } from './pendingSlot';
import type { ClaudeSessionState } from '../claudeSessionStore';

function mkSession(over: Partial<ClaudeSessionState> = {}): ClaudeSessionState {
  return {
    sessionId: 's1',
    pendingApproval: null,
    pendingQuestions: null,
    ...over,
  } as unknown as ClaudeSessionState;
}

const card = (toolName = 'Bash', command = 'ls', timestamp = 1) =>
  ({ toolName, toolInput: { command }, timestamp }) as never;

describe('pendingSlotOwner — one answer, four callers', () => {
  it('claude on PTY is the hook feed; stream or any other provider is the daemon', () => {
    expect(pendingSlotOwner({ provider: 'claude', transport: 'pty' })).toBe('hooks');
    expect(pendingSlotOwner({} as ClaudeSessionState)).toBe('hooks'); // default claude/pty
    expect(pendingSlotOwner({ provider: 'claude', transport: 'stream' })).toBe('daemon');
    expect(pendingSlotOwner({ provider: 'codex', transport: 'pty' })).toBe('daemon');
    expect(pendingSlotOwner({ provider: 'pi' } as ClaudeSessionState)).toBe('daemon');
  });

  it('a row that arrived from a peer is the federation mirror, whatever it reports', () => {
    // `hub` wins over provider/transport: nothing on THIS machine holds the
    // request, so neither local feed may park or resolve it. The reverse
    // direction (a peer over a local row) is refused in upsertRemoteSession;
    // this is the missing half.
    expect(pendingSlotOwner({ provider: 'claude', transport: 'pty', hub: 'laptop' })).toBe(
      'federation',
    );
    expect(pendingSlotOwner({ provider: 'codex', transport: 'stream', hub: 'laptop' })).toBe(
      'federation',
    );
  });
});

describe('PendingSlot — park / resolve are gated on the declared feed', () => {
  it('the owning feed parks and resolves', () => {
    const s = mkSession({ provider: 'codex' });
    const slot = new PendingSlot(s, 'daemon');
    expect(slot.ownsSlot).toBe(true);
    expect(slot.parkApproval(card())?.toolName).toBe('Bash');
    expect(s.pendingApproval?.toolName).toBe('Bash');
    expect(slot.resolveApproval()).toBeNull();
    expect(s.pendingApproval).toBeNull();
  });

  it('a foreign feed cannot park — and is told what the slot really holds', () => {
    const s = mkSession({ provider: 'codex' });
    const foreign = new PendingSlot(s, 'hooks');
    expect(foreign.ownsSlot).toBe(false);
    expect(foreign.parkApproval(card())).toBeNull();
    expect(s.pendingApproval).toBeNull();
    expect(foreign.parkQuestions([{ question: 'Which?', options: [] }])).toBeNull();
    expect(s.pendingQuestions).toBeNull();
  });

  it('a foreign feed cannot resolve, and the return value mirrors rather than assumes', () => {
    // The freeze shape: the owner holds a request, the second feed nulls it.
    const s = mkSession({ provider: 'codex', pendingApproval: card('Read', 'x') });
    const foreign = new PendingSlot(s, 'hooks');
    expect(foreign.resolveApproval()?.toolName).toBe('Read');
    foreign.resolveAll();
    expect(s.pendingApproval?.toolName).toBe('Read');
  });

  it('park is Keep for an unchanged card: same tool + input keeps the original object', () => {
    // Re-stamping a re-sent request resurrects a card the user dismissed.
    const first = card('Bash', 'npm test', 111);
    const s = mkSession({ provider: 'codex', pendingApproval: first });
    const slot = new PendingSlot(s, 'daemon');
    slot.parkApproval(card('Bash', 'npm test', 999));
    expect(s.pendingApproval).toBe(first);
    expect(s.pendingApproval?.timestamp).toBe(111);
    // A genuinely different request does replace it.
    slot.parkApproval(card('Bash', 'rm -rf /', 999));
    expect(s.pendingApproval?.timestamp).toBe(999);
  });

  it('park is Keep for an unchanged question set too', () => {
    const questions = [{ question: 'Which?', options: [{ label: 'a' }] }];
    const s = mkSession({ provider: 'codex', pendingQuestions: questions });
    const slot = new PendingSlot(s, 'daemon');
    slot.parkQuestions([{ question: 'Which?', options: [{ label: 'a' }] }]);
    expect(s.pendingQuestions).toBe(questions);
    slot.parkQuestions([{ question: 'Something else?', options: [] }]);
    expect(s.pendingQuestions).not.toBe(questions);
  });
});

describe('acknowledgeAnswer — the word the hook feed does not have', () => {
  it('clears the picker on ANY feed: the answer resolves the very request parked', () => {
    for (const over of [
      { provider: 'claude', transport: 'pty' as const },
      { provider: 'codex' },
      { hub: 'laptop' },
    ]) {
      const s = mkSession({ ...over, pendingQuestions: [{ question: 'Which?', options: [] }] });
      expect(acknowledgeAnswer(s)).toBeNull();
      expect(s.pendingQuestions).toBeNull();
    }
  });

  it('never touches the approval half — claude.approve may still be rejected as unknown', () => {
    const s = mkSession({
      provider: 'codex',
      pendingApproval: card(),
      pendingQuestions: [{ question: 'Which?', options: [] }],
    });
    acknowledgeAnswer(s);
    expect(s.pendingQuestions).toBeNull();
    expect(s.pendingApproval?.toolName).toBe('Bash');
  });
});

describe('bornWithPending — construction states its intent too', () => {
  const draft = (over: Partial<ClaudeSessionState> = {}) =>
    ({ sessionId: 's1', ...over }) as unknown as SessionWithoutPending;

  it('a new row is born with an empty slot', () => {
    const s = bornWithEmptyPending(draft());
    expect(s.pendingApproval).toBeNull();
    expect(s.pendingQuestions).toBeNull();
  });

  it('seeds from what the row previously held, so Keep can still see the old card', () => {
    const first = card('Bash', 'npm test', 111);
    const s = bornWithPending(
      draft({ hub: 'laptop' }),
      'federation',
      mkSession({ pendingApproval: first }),
      (slot) => slot.parkApproval(card('Bash', 'npm test', 999)),
    );
    expect(s.pendingApproval).toBe(first);
  });

  it('a fill by a feed that does not own the row leaves the seeded slot alone', () => {
    const held = card('Read', 'x', 5);
    const s = bornWithPending(
      draft({ hub: 'laptop' }),
      'daemon',
      mkSession({ pendingApproval: held }),
      (slot) => slot.resolveApproval(),
    );
    expect(s.pendingApproval).toBe(held);
  });
});
