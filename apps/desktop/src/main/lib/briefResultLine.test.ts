/**
 * The two refusals and the one honesty promise that make append-from-result
 * worth having:
 *
 *  1. A RESULT ALONE IS NOT A LINE. The manager's sentence is required, exactly
 *     as a dispatch template's {{task}} slot is.
 *  2. A MALFORMED SESSION ID IS REFUSED. `session:6a-round2` — the real line a
 *     live manager wrote and had to repair by hand — is the fixture.
 *  3. CAVEATS ALWAYS APPEAR. Everything else may be capped; a caveat may not be
 *     dropped, capped or elided.
 */
import { describe, it, expect } from 'vitest';
import {
  FACT_ITEMS_SHOWN,
  MalformedSessionRef,
  composeResultLine,
  hasResultParams,
  isSessionRef,
  normalizeSessionRef,
  renderResultFacts,
} from './briefResultLine';

const AT = new Date(2026, 7, 26, 11, 0, 0); // 2026-08-26, local

describe('normalizeSessionRef — the error class this feature exists to kill', () => {
  it('REFUSES the malformed reference a manager actually wrote', () => {
    expect(() => normalizeSessionRef('6a-round2')).toThrow(MalformedSessionRef);
    expect(() => normalizeSessionRef('session:6a-round2')).toThrow(/not a session id/);
  });

  it('names the offending value, so the manager can see what it typed', () => {
    expect(() => normalizeSessionRef('round2')).toThrow(/"round2"/);
  });

  it.each(['', '   ', 'session:', 'abc', 'the-parser-worker', 'ffff-2', 'zzzzzzzz', '12345'])(
    'refuses %j rather than writing a dead link',
    (bad) => {
      expect(() => normalizeSessionRef(bad)).toThrow(MalformedSessionRef);
    },
  );

  it('canonicalizes a full UUID to its first group', () => {
    expect(normalizeSessionRef('c03bd8ce-1f4a-4b2c-9d3e-0123456789ab')).toBe('c03bd8ce');
  });

  it('tolerates a session: prefix, surrounding space and upper-case hex', () => {
    expect(normalizeSessionRef('  session:C03BD8CE  ')).toBe('c03bd8ce');
  });

  it('shortens a long hex run and leaves a short-but-valid one alone', () => {
    expect(normalizeSessionRef('a1b2c3d4e5f6a7b8')).toBe('a1b2c3d4');
    expect(normalizeSessionRef('a1b2c3')).toBe('a1b2c3');
  });

  it('produces a form the brief board recognises as a reference', () => {
    // briefBoard.ts's REF_RE — the thing that makes the reference clickable.
    const ref = normalizeSessionRef('c03bd8ce-1f4a-4b2c-9d3e-0123456789ab');
    expect(/^session:[0-9a-f]{6,}$/.test(`session:${ref}`)).toBe(true);
  });

  it('isSessionRef answers the same question without throwing', () => {
    expect(isSessionRef('c03bd8ce')).toBe(true);
    expect(isSessionRef('6a-round2')).toBe(false);
  });
});

describe('composeResultLine — the happy path', () => {
  const RESULT = {
    commit: 'a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0',
    filesChanged: ['src/parser.ts', 'src/lexer.ts'],
    checksRun: ['vitest', 'tsc'],
    caveats: [],
    followUps: ['delete the v1 path'],
  };

  it('renders date, sentence, facts and reference in that order', () => {
    const line = composeResultLine({
      significance: 'the parser no longer allocates per token, which unblocks the mobile client',
      sessionId: 'c03bd8ce-1f4a-4b2c-9d3e-0123456789ab',
      result: RESULT,
      now: AT,
    });
    expect(line).toBe(
      '2026-08-26  the parser no longer allocates per token, which unblocks the mobile client — ' +
        'commit: a1b2c3d4e5f6; filesChanged: src/parser.ts, src/lexer.ts; checksRun: vitest, tsc; ' +
        'followUps: delete the v1 path (session:c03bd8ce)',
    );
  });

  it('keeps the manager’s own date rather than stacking a second one', () => {
    const line = composeResultLine({
      significance: '2026-08-24  backfilled from the handoff',
      sessionId: 'c03bd8ce',
      now: AT,
    });
    expect(line).toBe('2026-08-24  backfilled from the handoff (session:c03bd8ce)');
  });

  it('does not append a reference the sentence already carries', () => {
    const line = composeResultLine({
      significance: 'landed, see session:c03bd8ce for the diagnosis',
      sessionId: 'c03bd8ce-1f4a-4b2c-9d3e-0123456789ab',
      now: AT,
    });
    expect(line.match(/session:c03bd8ce/g)).toHaveLength(1);
  });

  it('works with a session id and no result at all', () => {
    expect(composeResultLine({ significance: 'shipped X', sessionId: 'c03bd8ce', now: AT })).toBe(
      '2026-08-26  shipped X (session:c03bd8ce)',
    );
  });

  it('works with a result and no session id', () => {
    expect(
      composeResultLine({ significance: 'shipped X', result: { commit: 'abc1234' }, now: AT }),
    ).toBe('2026-08-26  shipped X — commit: abc1234');
  });

  it('refuses the malformed session id BEFORE composing anything', () => {
    expect(() =>
      composeResultLine({ significance: 'shipped X', sessionId: '6a-round2', result: RESULT }),
    ).toThrow(MalformedSessionRef);
  });
});

describe('the significance sentence is REQUIRED — a result alone is never a line', () => {
  const RESULT = { commit: 'abc1234', filesChanged: ['a.ts'] };

  it.each(['', '   ', '\n\t'])('refuses %j with a result present', (blank) => {
    expect(() =>
      composeResultLine({ significance: blank, sessionId: 'c03bd8ce', result: RESULT }),
    ).toThrow(/one-sentence significance/);
  });

  it('says WHY, so the manager writes the sentence instead of retrying', () => {
    let msg = '';
    try {
      composeResultLine({ significance: '', result: RESULT });
    } catch (e) {
      msg = (e as Error).message;
    }
    expect(msg).toMatch(/judgement is the part only/);
    expect(msg).toMatch(/nothing was written/i);
  });
});

describe('facts are rendered compactly, lossily, and honestly', () => {
  it('caps a long list and SAYS how many it kept back', () => {
    const files = ['a.ts', 'b.ts', 'c.ts', 'd.ts', 'e.ts', 'f.ts'];
    const out = renderResultFacts({ filesChanged: files });
    expect(out).toBe(`filesChanged: a.ts, b.ts, c.ts, +${files.length - FACT_ITEMS_SHOWN} more`);
    expect(out).not.toContain('f.ts');
  });

  it('NEVER caps or drops caveats, however many there are', () => {
    const caveats = [
      'the migration is not reversible',
      'the fixture still uses the old schema',
      'CI is red on windows for an unrelated reason',
      'the flag defaults to on',
      'nobody has run this against prod data',
    ];
    const out = renderResultFacts({ commit: 'abc1234', caveats });
    for (const c of caveats) expect(out).toContain(c);
    expect(out).not.toContain('more');
  });

  it('keeps a very long caveat whole where any other field would be cut', () => {
    const long = 'x'.repeat(600);
    expect(renderResultFacts({ caveats: [long] })).toContain(long);
    expect(renderResultFacts({ notes: long })).toMatch(/… \(600 chars\)/);
  });

  it('a caveat survives into the composed line', () => {
    const line = composeResultLine({
      significance: 'landed',
      sessionId: 'c03bd8ce',
      result: { commit: 'abc1234', caveats: ['the migration is not reversible'] },
      now: AT,
    });
    expect(line).toContain('caveats: the migration is not reversible');
  });

  it('drops empty values — an empty caveats list is "none", not a caveat', () => {
    expect(renderResultFacts({ commit: 'abc1234', caveats: [], followUps: null, notes: '' })).toBe(
      'commit: abc1234',
    );
  });

  it('shortens a full sha and leaves an abbreviated one alone', () => {
    expect(renderResultFacts({ commit: 'a'.repeat(40) })).toBe(`commit: ${'a'.repeat(12)}`);
    expect(renderResultFacts({ commit: 'abc1234' })).toBe('commit: abc1234');
  });

  it('treats the result as ARBITRARY json — unknown keys are kept, not dropped', () => {
    const out = renderResultFacts({ benchmarkMs: 12, regressed: false, owner: { team: 'core' } });
    expect(out).toContain('benchmarkMs: 12');
    expect(out).toContain('regressed: false');
    expect(out).toContain('owner: {"team":"core"}');
  });

  it('puts the known keys first, whatever order they arrived in', () => {
    const out = renderResultFacts({
      zzz: 'last',
      followUps: ['f'],
      commit: 'abc1234',
      filesChanged: ['a'],
    });
    expect(out).toBe('commit: abc1234; filesChanged: a; followUps: f; zzz: last');
  });

  it('renders nothing at all for an empty or absent result', () => {
    expect(renderResultFacts(undefined)).toBe('');
    expect(renderResultFacts(null)).toBe('');
    expect(renderResultFacts({})).toBe('');
  });
});

describe('hasResultParams — the switch that keeps plain brief_append untouched', () => {
  it('is false when neither param is present', () => {
    expect(hasResultParams({})).toBe(false);
    expect(hasResultParams({ sessionId: undefined, result: undefined })).toBe(false);
  });

  it('is true for either one alone', () => {
    expect(hasResultParams({ sessionId: 'c03bd8ce' })).toBe(true);
    expect(hasResultParams({ result: {} })).toBe(true);
  });
});
