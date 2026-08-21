/**
 * Round-trip pin for the fleet/supervisor wake format: the same module builds
 * the message (supervisorNudge) and parses it back (the GUI's card). A format
 * change that breaks either direction fails HERE, not as a silent regression
 * to raw-text bubbles.
 */
import { describe, it, expect } from 'vitest';
import {
  buildFleetMessage,
  excerptReply,
  parseFleetMessage,
  type FleetMessageEntry,
} from './fleetMessages';

const finished: FleetMessageEntry = {
  label: 'alpha: fix tests',
  sessionId: 'w1',
  cwd: '/home/u/Work/alpha',
  lastReply: excerptReply('All 42 tests pass.\nDone.'),
};

describe('buildFleetMessage → parseFleetMessage round trip', () => {
  it('worker-finished survives, including a flattened reply excerpt', () => {
    const text = buildFleetMessage('worker-finished', [finished]);
    expect(text).toContain('[fleet] Worker finished');
    expect(text).toContain('brief.md'); // the instruction tail rides along
    const parsed = parseFleetMessage(text);
    expect(parsed).toEqual({
      kind: 'worker-finished',
      entries: [{ ...finished, lastReply: 'All 42 tests pass. Done.' }],
    });
  });

  it('replies full of the delimiters themselves cannot break parsing', () => {
    const nasty = excerptReply(
      'Merged a; b; c (session:fake, cwd /x) — last reply: not really. Review the result (get_conversation',
    );
    const text = buildFleetMessage('worker-finished', [{ ...finished, lastReply: nasty }]);
    const parsed = parseFleetMessage(text);
    expect(parsed?.entries).toHaveLength(1);
    expect(parsed?.entries[0].sessionId).toBe('w1');
    expect(parsed?.entries[0].lastReply).toBe(nasty);
  });

  it('multi-entry coalesced wakes keep every worker', () => {
    const text = buildFleetMessage('worker-finished', [
      finished,
      { label: 'beta: docs', sessionId: 'w2', cwd: '/home/u/Work/beta' },
    ]);
    const parsed = parseFleetMessage(text);
    expect(parsed?.entries.map((e) => e.sessionId)).toEqual(['w1', 'w2']);
    expect(parsed?.entries[1].lastReply).toBeUndefined();
  });

  it('catch-up and blocked kinds round-trip too', () => {
    const catchUp = parseFleetMessage(
      buildFleetMessage('catch-up', [{ label: 'alpha', sessionId: 'w1', cwd: '/w/alpha' }]),
    );
    expect(catchUp?.kind).toBe('catch-up');

    const blocked = parseFleetMessage(
      buildFleetMessage('blocked', [
        { label: 'alpha: fix', sessionId: 'w1', blockedOn: 'approval' },
        { label: 'beta: ship', sessionId: 'w2', blockedOn: 'question' },
      ]),
    );
    expect(blocked?.kind).toBe('blocked');
    expect(blocked?.entries.map((e) => e.blockedOn)).toEqual(['approval', 'question']);
  });
});

describe('parseFleetMessage on non-wake text', () => {
  it('rejects ordinary user text, prefixes alone, and malformed bullets', () => {
    expect(parseFleetMessage('Please deploy the fleet of workers')).toBeNull();
    expect(parseFleetMessage('[fleet] Worker finished:')).toBeNull();
    expect(
      parseFleetMessage('[fleet] Worker finished:\n- gibberish without a ref\nTail'),
    ).toBeNull();
    expect(parseFleetMessage('[supervisor] hello')).toBeNull();
  });
});

describe('legacy single-paragraph wakes (already sitting in old transcripts)', () => {
  it('parses the old worker-finished shape', () => {
    const text =
      `[fleet] Worker finished: alpha: fix tests (session:w1, cwd /home/u/Work/alpha) — ` +
      `last reply: All 42 tests pass. Done. ` +
      `Review the result (get_conversation with sinceSeq for detail), append one line to that ` +
      `project's .workspacer/brief.md "## Recently" (and adjust "## Now"), then report the ` +
      `outcome briefly with session:<id> references. If it was not one of your dispatches, ` +
      `a one-line acknowledgement is enough.`;
    const parsed = parseFleetMessage(text);
    expect(parsed?.kind).toBe('worker-finished');
    expect(parsed?.entries[0]).toMatchObject({
      label: 'alpha: fix tests',
      sessionId: 'w1',
      cwd: '/home/u/Work/alpha',
    });
  });

  it('re-joins a reply the old "; " join had split, and parses old blocked wakes', () => {
    const text =
      `[fleet] Worker finished: alpha (session:w1, cwd /w/a) — last reply: did x; then y. ` +
      `Review the result (get_conversation with sinceSeq for detail), …`;
    const parsed = parseFleetMessage(text);
    expect(parsed?.entries).toHaveLength(1);
    expect(parsed?.entries[0].lastReply).toContain('then y');

    const blocked = parseFleetMessage(
      `[supervisor] An agent is now blocked on a decision: beta: ship (session:w2, question). ` +
        `Run a /supervise pass: gather the context and notify me with a recommendation.`,
    );
    expect(blocked?.entries[0]).toMatchObject({ sessionId: 'w2', blockedOn: 'question' });
  });
});
