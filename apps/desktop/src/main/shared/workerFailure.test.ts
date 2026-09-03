/**
 * The READER half of contracts/agent-error-marker-cases.json. The WRITER half
 * is claudemon's providers/mod.rs, whose own test loads the same fixture — a
 * marker change on either side fails on both.
 */
import { describe, it, expect } from 'vitest';
import * as fs from 'fs';
import * as path from 'path';
import {
  AGENT_ERROR_MARKER,
  errorMarkerReason,
  workerFailureReason,
  isCreditBalanceTooLowError,
  isCreditBalanceFailureText,
} from './workerFailure';

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
  it('the overage bit ALONE does not fail a clean finish — it is standing account state, not a turn outcome', () => {
    expect(workerFailureReason({ statusLine: { overageOutOfCredits: true } }, 'Done.')).toBeNull();
    expect(
      workerFailureReason(
        { statusLine: { overageOutOfCredits: true } },
        'All 42 tests pass.\nMerged.',
      ),
    ).toBeNull();
  });

  it('enriches a marker-established failure with the structured out-of-credits bit', () => {
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

  it('does NOT enrich a transient 529 with "out of credits", even on an out-of-credits account', () => {
    // Live 2026-09-03 case: the fleet wake read "FAILED: out of credits
    // (overage disabled) - API Error: 529 Overloaded..." for a plain server
    // overload, sending the manager down the wrong troubleshooting path.
    const reason = workerFailureReason(
      { statusLine: { overageOutOfCredits: true } },
      '⚠️ Error: API Error: 529 Overloaded. This is a server-side issue, usually temporary',
    );
    expect(reason).not.toContain('out of credits');
    expect(reason).toBe(
      'API Error: 529 Overloaded. This is a server-side issue, usually temporary',
    );
  });

  it('does NOT enrich a 429 rate limit with "out of credits", even on an out-of-credits account', () => {
    const reason = workerFailureReason(
      { statusLine: { overageOutOfCredits: true } },
      '⚠️ Error: API Error: 429 Too Many Requests, please retry',
    );
    expect(reason).not.toContain('out of credits');
    expect(reason).toBe('API Error: 429 Too Many Requests, please retry');
  });

  it('still enriches a genuine out-of-credits message that names none of the transient phrasings', () => {
    const reason = workerFailureReason(
      { statusLine: { overageOutOfCredits: true } },
      '⚠️ Error: Your account has insufficient balance to complete this request.',
    );
    expect(reason).toContain('out of credits');
    expect(reason).toContain('insufficient balance');
  });
});

describe('isCreditBalanceFailureText', () => {
  it('attaches for a genuine marker-established credit-balance failure', () => {
    expect(isCreditBalanceFailureText('⚠️ Error: Credit balance is too low')).toBe(true);
  });

  it('does NOT attach for an ordinary reply that merely quotes the phrase', () => {
    // The adversarial case: a successful turn that talks ABOUT the error
    // (e.g. summarizing this very feature) must not be mistaken for a death.
    expect(
      isCreditBalanceFailureText(
        'Done. When Claude Code reports "Credit balance is too low" the fix is to re-login. I added handling for that.',
      ),
    ).toBe(false);
  });

  it('does NOT attach for a marker-established failure that is a different error', () => {
    expect(
      isCreditBalanceFailureText(
        '⚠️ Error: API Error: 529 Overloaded. This is a server-side issue, usually temporary',
      ),
    ).toBe(false);
  });

  it('does NOT attach for plain unmarked text, even if it contains the phrase mid-message', () => {
    expect(isCreditBalanceFailureText('Credit balance is too low, apparently.')).toBe(false);
  });
});

describe('isCreditBalanceTooLowError', () => {
  it('matches the exact text from the live incident', () => {
    expect(isCreditBalanceTooLowError('Error: Credit balance is too low')).toBe(true);
  });

  it('matches the doubled/concatenated render observed live', () => {
    expect(isCreditBalanceTooLowError('Credit balance is too lowCredit balance is too low')).toBe(
      true,
    );
  });

  it('matches case-insensitively and through the agent-error marker', () => {
    expect(isCreditBalanceTooLowError('⚠️ Error: CREDIT BALANCE IS TOO LOW.')).toBe(true);
  });

  it('matches on a structured error code first, when one is present', () => {
    expect(isCreditBalanceTooLowError('some unrelated text', 'credit_balance_too_low')).toBe(true);
  });

  it('does not match an unrelated error (529 overload)', () => {
    expect(
      isCreditBalanceTooLowError(
        'API Error: 529 Overloaded. This is a server-side issue, usually temporary',
      ),
    ).toBe(false);
  });

  it('does not match generic mentions of "balance" or "credit" alone', () => {
    expect(isCreditBalanceTooLowError('please balance the tool calls with results')).toBe(false);
    expect(isCreditBalanceTooLowError('give the user credit for finding this')).toBe(false);
  });
});
