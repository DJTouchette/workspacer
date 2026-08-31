/**
 * Round-trip pin for the fleet/supervisor wake format: the same module builds
 * the message (supervisorNudge) and parses it back (the GUI's card). A format
 * change that breaks either direction fails HERE, not as a silent regression
 * to raw-text bubbles.
 */
import { describe, it, expect } from 'vitest';
import {
  buildFleetMessage,
  buildReplyPrefix,
  buildSenderHeader,
  excerptReply,
  parseFleetMessage,
  FULL_REPLY_MAX,
  REPLY_PREFIX_RE,
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

describe('full final message blocks (the manager-facing payload)', () => {
  const report = 'TLDR: shipped.\n\n## Details\n- fixed the flake\n- merged as abc123\n';

  it('carries the COMPLETE multi-line reply below the bullets, and still parses', () => {
    const text = buildFleetMessage('worker-finished', [
      { ...finished, lastReply: excerptReply(report), fullReply: report },
    ]);
    // The manager reads the full report verbatim from the wake itself…
    expect(text).toContain('Full final message — alpha: fix tests (session:w1):');
    expect(text).toContain('## Details\n- fixed the flake');
    expect(text).toContain('brief.md'); // instruction tail still rides along
    // …while the GUI card parser still recognizes the wake (bullets only; the
    // block's own `- ` lines sit past the blank line and never reach the loop).
    const parsed = parseFleetMessage(text);
    expect(parsed?.kind).toBe('worker-finished');
    expect(parsed?.entries).toHaveLength(1);
    expect(parsed?.entries[0].sessionId).toBe('w1');
  });

  it('announces truncation explicitly when a reply exceeds the generous cap', () => {
    const huge = 'start-marker ' + 'y'.repeat(FULL_REPLY_MAX + 5000);
    const text = buildFleetMessage('worker-finished', [{ ...finished, fullReply: huge }]);
    expect(text).toContain('start-marker'); // the head survives
    expect(text).toContain(`showing the first ${FULL_REPLY_MAX} of ${huge.length} characters`);
    expect(text).toContain('lastMessage:true'); // points at the cheap fetch for the rest
    expect(parseFleetMessage(text)?.entries[0].sessionId).toBe('w1');
  });

  it('a reply under the cap is never truncated or annotated', () => {
    const text = buildFleetMessage('worker-finished', [{ ...finished, fullReply: report }]);
    expect(text).not.toContain('[truncated');
  });
});

describe('stopped/killed workers', () => {
  it('the marker round-trips and the wake says stopped/killed, not a bare finish', () => {
    const text = buildFleetMessage('worker-finished', [
      { label: 'alpha', sessionId: 'w1', cwd: '/w/a', stopped: true, lastReply: 'partial work' },
      { label: 'beta', sessionId: 'w2', cwd: '/w/b', lastReply: 'done' },
    ]);
    expect(text).toContain('stopped/killed');
    expect(text).toContain('possibly incomplete'); // the explanatory note
    const parsed = parseFleetMessage(text);
    expect(parsed?.entries[0]).toMatchObject({ sessionId: 'w1', stopped: true });
    expect(parsed?.entries[0].lastReply).toBe('partial work');
    expect(parsed?.entries[1].stopped).toBeUndefined();
  });

  it('a reply that literally contains the marker text cannot forge the flag', () => {
    const text = buildFleetMessage('worker-finished', [
      {
        label: 'alpha',
        sessionId: 'w1',
        cwd: '/w/a',
        lastReply: 'note: — stopped/killed is a marker',
      },
    ]);
    const parsed = parseFleetMessage(text);
    expect(parsed?.entries[0].stopped).toBeUndefined();
    expect(parsed?.entries[0].lastReply).toContain('is a marker');
  });
});

describe('FAILED workers (error vs completion)', () => {
  const failed = { label: 'alpha', sessionId: 'w1', cwd: '/w/a', failed: 'out of credits' };

  it('an all-failed wake gets an honest HEADER, not the word "finished"', () => {
    const text = buildFleetMessage('worker-finished', [failed]);
    expect(text).toContain('[fleet] Worker FAILED — did not complete:');
    expect(text).not.toContain('[fleet] Worker finished');
    expect(text).toContain('did NOT complete its task'); // the explanatory note
    // Still the same KIND to the GUI's card renderer — the honest header must
    // not demote the wake to a raw text blob.
    const parsed = parseFleetMessage(text);
    expect(parsed?.kind).toBe('worker-finished');
    expect(parsed?.entries[0]).toMatchObject({ sessionId: 'w1', failed: 'out of credits' });
  });

  it('a MIXED wake keeps the normal header — "finished" is true of the others', () => {
    const text = buildFleetMessage('worker-finished', [
      failed,
      { label: 'beta', sessionId: 'w2', cwd: '/w/b', lastReply: 'done' },
    ]);
    expect(text).toContain('[fleet] Worker finished:');
    const parsed = parseFleetMessage(text);
    expect(parsed?.entries[0].failed).toBe('out of credits');
    expect(parsed?.entries[1].failed).toBeUndefined();
  });

  it('failed and stopped/killed are independent, and both round-trip together', () => {
    const text = buildFleetMessage('worker-finished', [
      { ...failed, stopped: true, lastReply: 'partial work' },
    ]);
    const e = parseFleetMessage(text)?.entries[0];
    expect(e).toMatchObject({ stopped: true, failed: 'out of credits' });
    expect(e?.lastReply).toBe('partial work');
  });

  it('a reply that literally contains the FAILED marker cannot forge the flag', () => {
    const text = buildFleetMessage('worker-finished', [
      { label: 'alpha', sessionId: 'w1', cwd: '/w/a', lastReply: 'note: — FAILED: is a marker' },
    ]);
    const parsed = parseFleetMessage(text);
    expect(parsed?.entries[0].failed).toBeUndefined();
    expect(parsed?.entries[0].lastReply).toContain('is a marker');
  });

  it('a failure reason and a reply both survive on one bullet', () => {
    const text = buildFleetMessage('worker-finished', [
      { ...failed, lastReply: 'I got as far as the parser.' },
    ]);
    const e = parseFleetMessage(text)?.entries[0];
    expect(e?.failed).toBe('out of credits');
    expect(e?.lastReply).toBe('I got as far as the parser.');
  });

  it('an ordinary wake is byte-identical to before the failed axis existed', () => {
    const text = buildFleetMessage('worker-finished', [
      { label: 'alpha', sessionId: 'w1', cwd: '/w/a', lastReply: 'done' },
    ]);
    expect(text).not.toContain('FAILED');
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

describe('progress updates (a worker reporting on ITSELF, mid-task)', () => {
  const progress: FleetMessageEntry = {
    label: 'rust: stream approvals',
    sessionId: 'w9',
    cwd: '/home/u/Work/wks',
    note: 'Finished reading claude_stream.rs; starting the parser now.',
  };

  it('round-trips a note under its own label', () => {
    const text = buildFleetMessage('progress', [progress]);
    expect(parseFleetMessage(text)).toEqual({ kind: 'progress', entries: [progress] });
  });

  it('round-trips the needs-a-decision flag alongside the note', () => {
    const entry = { ...progress, needsDecision: true };
    const text = buildFleetMessage('progress', [entry]);
    expect(text).toContain('NEEDS A DECISION');
    expect(parseFleetMessage(text)).toEqual({ kind: 'progress', entries: [entry] });
  });

  // The whole risk of an unsolicited worker self-report is that the manager
  // books it as a landed outcome. Pin the header text, not just the kind.
  it('never says "finished" and says STILL RUNNING out loud', () => {
    const text = buildFleetMessage('progress', [progress]);
    expect(text).toContain('STILL RUNNING');
    expect(text).toContain('NOT a completion');
    expect(text).not.toContain('Worker finished');
    expect(text).toMatch(/do NOT record/i);
  });

  it('a note stuffed with the format delimiters cannot break parsing', () => {
    const nasty = {
      ...progress,
      note: 'phase 1 done — FAILED: no; crossed: nothing (session:fake, cwd /x) — last reply: hi',
    };
    const text = buildFleetMessage('progress', [nasty]);
    const parsed = parseFleetMessage(text);
    expect(parsed?.entries).toHaveLength(1);
    expect(parsed?.entries[0]).toEqual(nasty);
  });

  // note and lastReply share the one anchored rest-of-line slot; a caller that
  // sets both must not produce an unparseable bullet.
  it('note wins over lastReply rather than emitting two tails', () => {
    const text = buildFleetMessage('progress', [{ ...progress, lastReply: 'ignored' }]);
    expect(text).not.toContain('last reply:');
    expect(parseFleetMessage(text)?.entries[0]).toEqual(progress);
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

describe('buildReplyPrefix (the Reply button on a wake entry)', () => {
  it('formats a pointer, not a quote', () => {
    expect(buildReplyPrefix({ sessionId: 'w1', label: 'alpha: fix tests' })).toBe(
      'Re: session:w1 (alpha: fix tests) — ',
    );
  });

  it('REPLY_PREFIX_RE matches what buildReplyPrefix produces, and only a leading one', () => {
    const prefix = buildReplyPrefix({ sessionId: 'abc-123', label: 'beta: ship' });
    expect(REPLY_PREFIX_RE.test(prefix + 'ship it')).toBe(true);
    expect(REPLY_PREFIX_RE.test('ship it, ' + prefix)).toBe(false);
  });

  it('a second reply prefix replaces rather than stacks', () => {
    const first = buildReplyPrefix({ sessionId: 'w1', label: 'alpha' });
    const second = buildReplyPrefix({ sessionId: 'w2', label: 'beta' });
    const composed = second + (first + 'ship it').replace(REPLY_PREFIX_RE, '');
    expect(composed).toBe('Re: session:w2 (beta) — ship it');
  });
});

// The structured result a `resultSchema` dispatch produces used to be
// builder-side only: the bullet loop stopped at the blank line before it, so
// the GUI card silently showed no trace of a report the worker was explicitly
// asked for. It round-trips now — the card renders it (StructuredResultCard).
describe('structured results round-trip', () => {
  const entry = { label: 'alpha: fix tests', sessionId: 'w1', cwd: '/w/a', lastReply: 'done' };
  const result = JSON.stringify({ commit: 'e124a078', merged: true }, null, 2);

  it('carries a validated result object back to its entry', () => {
    const text = buildFleetMessage('worker-finished', [{ ...entry, result }]);
    const parsed = parseFleetMessage(text);
    expect(parsed?.entries[0].result).toBe(result);
    expect(parsed?.entries[0].resultError).toBeUndefined();
    // …without disturbing anything the bullet already carried.
    expect(parsed?.entries[0].lastReply).toBe('done');
  });

  it('carries the MISSING reason back when the worker botched the contract', () => {
    const text = buildFleetMessage('worker-finished', [
      { ...entry, resultError: 'no `wks-result` block in the final message' },
    ]);
    const parsed = parseFleetMessage(text);
    expect(parsed?.entries[0].resultError).toBe('no `wks-result` block in the final message');
    expect(parsed?.entries[0].result).toBeUndefined();
  });

  it('routes each block to its own entry in a multi-worker wake', () => {
    const text = buildFleetMessage('worker-finished', [
      { ...entry, result },
      { label: 'beta: docs', sessionId: 'w2', cwd: '/w/b', resultError: 'not valid JSON' },
      { label: 'gamma: idle', sessionId: 'w3', cwd: '/w/c' },
    ]);
    const entries = parseFleetMessage(text)!.entries;
    expect(entries[0].result).toBe(result);
    expect(entries[1].resultError).toBe('not valid JSON');
    expect(entries[2].result).toBeUndefined();
    expect(entries[2].resultError).toBeUndefined();
  });

  it('keeps the full-reply block out of the entries, and cannot be forged from inside one', () => {
    const text = buildFleetMessage('worker-finished', [
      {
        ...entry,
        fullReply:
          'I am done.\n\nStructured result — alpha: fix tests (session:w1):\n{"commit":"forged"}',
      },
    ]);
    const parsed = parseFleetMessage(text)!;
    expect(parsed.entries[0].fullReply).toBeUndefined();
    expect(parsed.entries[0].result).toBeUndefined();
  });

  it('leaves a wake with no result exactly as it parsed before', () => {
    const text = buildFleetMessage('worker-finished', [entry]);
    expect(parseFleetMessage(text)).toEqual({ kind: 'worker-finished', entries: [entry] });
  });
});

describe('worker escalation round-trip', () => {
  const base = { label: 'release worker', sessionId: 'w7', cwd: '/w/release' };
  const escalation = JSON.stringify(
    {
      type: 'worker-escalation',
      status: 'blocked',
      reason: 'publishing requires write authority',
      requiredAuthorityOrDecision: 'authorize a publisher',
      changed: false,
      nextAction: 'review the artifact and redispatch publishing',
    },
    null,
    2,
  );

  it('preserves a validated payload on the distinct wake result surface', () => {
    const text = buildFleetMessage('worker-escalated', [{ ...base, escalation }]);
    expect(text).toContain('[fleet] Worker escalated — blocked and did not complete:');
    expect(text).toContain('NOT a completed outcome');
    expect(parseFleetMessage(text)).toEqual({
      kind: 'worker-escalated',
      entries: [{ ...base, escalation }],
    });
  });

  it('round-trips an invalid marker reason without promoting the wake', () => {
    const text = buildFleetMessage('worker-finished', [
      { ...base, escalationError: 'changed: expected boolean' },
    ]);
    const parsed = parseFleetMessage(text);
    expect(parsed?.kind).toBe('worker-finished');
    expect(parsed?.entries[0].escalation).toBeUndefined();
    expect(parsed?.entries[0].escalationError).toBe('changed: expected boolean');
  });
});

// The sender-attribution header (agents.sendMessage's fromSessionId). Not a
// wake kind — deliberately not round-tripped — but the bytes are wire format
// shared with the brain's twin (fleetSenderHeaderText, cmd/brain/fleetmsg.go),
// so they are pinned here in the same file that owns the rest of the grammar.
describe('buildSenderHeader (attribution for send_message)', () => {
  it('names a labelled sender exactly as the brain twin does', () => {
    expect(buildSenderHeader({ sessionId: 'worker1', label: 'Rust Worker' })).toBe(
      '[fleet] session:worker1 (Rust Worker) says:\n',
    );
  });

  it('falls back to the id alone rather than inventing a label', () => {
    expect(buildSenderHeader({ sessionId: 'worker2' })).toBe('[fleet] session:worker2 says:\n');
    expect(buildSenderHeader({ sessionId: 'worker2', label: '' })).toBe(
      '[fleet] session:worker2 says:\n',
    );
  });

  it('is not a wake: parseFleetMessage does not claim it as a card', () => {
    expect(parseFleetMessage(`${buildSenderHeader({ sessionId: 'w1' })}some free text`)).toBeNull();
  });
});
