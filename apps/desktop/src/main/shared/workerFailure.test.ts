/**
 * The READER half of contracts/agent-error-marker-cases.json. The WRITER half
 * is claudemon's providers/mod.rs, whose own test loads the same fixture — a
 * marker change on either side fails on both.
 */
import { describe, it, expect } from 'vitest';
import * as fs from 'fs';
import * as path from 'path';
import { AGENT_ERROR_MARKER, errorMarkerReason, workerFailureReason } from './workerFailure';

interface Fixture {
  marker: string;
  cases: Array<{ name: string; finalMessage: string; failed: boolean; reason?: string }>;
}

const fixture: Fixture = JSON.parse(
  fs.readFileSync(
    path.join(
      __dirname,
      '..',
      '..',
      '..',
      '..',
      '..',
      'contracts',
      'agent-error-marker-cases.json',
    ),
    'utf8',
  ),
);

describe('agent-error marker (cross-language contract)', () => {
  it('the constant IS the fixture marker', () => {
    expect(AGENT_ERROR_MARKER).toBe(fixture.marker);
  });

  it('has cases (a fixture that stopped loading would pass vacuously)', () => {
    expect(fixture.cases.length).toBeGreaterThan(5);
  });

  for (const c of fixture.cases) {
    it(`${c.name}`, () => {
      const reason = errorMarkerReason(c.finalMessage);
      if (c.failed) {
        expect(reason).toBe(c.reason);
      } else {
        expect(reason).toBeNull();
      }
    });
  }
});

describe('workerFailureReason', () => {
  it('names out-of-credits from the structured statusLine bit, not the wording', () => {
    expect(workerFailureReason({ statusLine: { overageOutOfCredits: true } }, 'Done.')).toBe(
      'out of credits (overage disabled)',
    );
  });

  it('combines the structured bit with the marker text when both are present', () => {
    const reason = workerFailureReason(
      { statusLine: { overageOutOfCredits: true } },
      '⚠️ Error: Credit balance is too low.',
    );
    expect(reason).toContain('out of credits');
    expect(reason).toContain('Credit balance is too low.');
  });

  it('is null for an ordinary finish with no failure signal at all', () => {
    expect(workerFailureReason({}, 'All 42 tests pass.')).toBeNull();
    expect(workerFailureReason({ statusLine: { overageOutOfCredits: false } }, 'Done.')).toBeNull();
  });

  it('flattens a reason to one line so a wake bullet can never span lines', () => {
    const reason = errorMarkerReason('⚠️ Error: a  b\tc');
    expect(reason).toBe('a b c');
  });

  it('strips the bullet grammar’s own separator out of a reason', () => {
    // ` — ` delimits the bullet's tails; a reason carrying one would split the
    // entry at the wrong place when the GUI parses the wake back.
    expect(errorMarkerReason('⚠️ Error: rate limited — retry later')).toBe(
      'rate limited - retry later',
    );
  });

  it('caps a pathological reason', () => {
    const reason = errorMarkerReason(`⚠️ Error: ${'x'.repeat(1000)}`)!;
    expect(reason.length).toBeLessThan(220);
    expect(reason.endsWith('…')).toBe(true);
  });
});
