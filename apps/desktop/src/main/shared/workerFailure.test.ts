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
  CREDIT_BALANCE_REMEDY,
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
});

// ---------------------------------------------------------------------------
// The overage enrichment, against the wordings Claude Code ACTUALLY emits.
//
// Each `message` below is lifted from an `isApiErrorMessage: true` row in this
// machine's ~/.claude/projects transcripts (90 rows, read 2026-09-03), not
// invented — which is the whole point: the first version of this gate was an
// opt-OUT list of transient spellings, and it had to guess wordings the CLI
// does not use. It excluded "429" (no real 429 row spells the number in its
// prose) and let every authentication failure through, so with the overage bit
// set a stale login woke its manager as "FAILED: out of credits (overage
// disabled) - Not logged in".
// ---------------------------------------------------------------------------
describe('workerFailureReason enrichment is opt-in on the marker wording', () => {
  const OVERAGE = { statusLine: { overageOutOfCredits: true } };
  const PREFIX = 'out of credits (overage disabled) - ';

  const enriched: Array<[string, string]> = [
    [
      'out of usage credits (429, rate_limit)',
      "You're out of usage credits. Switch to another model, or manage usage credits at claude.ai/settings/usage, to continue.",
    ],
    [
      'session limit (429, rate_limit)',
      "You've hit your session limit · resets 6:30pm (America/Edmonton)",
    ],
    [
      'weekly limit (429, rate_limit)',
      "You've hit your weekly limit · resets Aug 10, 4am (America/Edmonton)",
    ],
    ['the credit-balance refusal', 'Credit balance is too low to access the Anthropic API.'],
  ];

  const untouched: Array<[string, string]> = [
    [
      '529 overload (server_error)',
      'API Error: 529 Overloaded. This is a server-side issue, usually temporary.',
    ],
    ['not logged in (authentication_failed)', 'Not logged in · Please run /login'],
    ['login expired (authentication_failed)', 'Login expired · Please run /login'],
    [
      'revoked token (401, authentication_failed)',
      'Failed to authenticate. API Error: 401 OAuth access token has been revoked.',
    ],
    [
      'connection closed mid-response (server_error)',
      'API Error: Connection closed mid-response. The response above may be incomplete.',
    ],
    [
      'server error mid-response (server_error)',
      'API Error: Server error mid-response. The response above may be incomplete.',
    ],
    [
      'safeguards refusal (invalid_request)',
      "API Error: Fable 5's safeguards flagged this message (https://www.anthropic.com/legal/aup).",
    ],
    // Not wordings the CLI emits, but the shapes the dropped `\b5\d\d\b` branch
    // used to read as an HTTP status: a source line number and a millisecond
    // delay. Neither may move the classification now.
    ['a source line number is not a status code', 'build failed at src/app.ts:512'],
    ['a millisecond delay is not a status code', 'Retry after 500 ms'],
    ['a port number is not a status code', 'connect ECONNREFUSED 127.0.0.1:500'],
  ];

  for (const [name, message] of enriched) {
    it(`enriches ${name}`, () => {
      expect(workerFailureReason(OVERAGE, `⚠️ Error: ${message}`)).toBe(`${PREFIX}${message}`);
    });
  }

  for (const [name, message] of untouched) {
    it(`leaves ${name} unenriched`, () => {
      const reason = workerFailureReason(OVERAGE, `⚠️ Error: ${message}`);
      expect(reason).not.toContain('out of credits');
      expect(reason).toBe(message);
    });
  }

  it('never enriches when the overage bit is not set', () => {
    for (const [, message] of enriched) {
      expect(workerFailureReason({}, `⚠️ Error: ${message}`)).toBe(message);
    }
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

describe('CREDIT_BALANCE_REMEDY', () => {
  it('covers the API-key / console-billing user, for whom re-login cannot help', () => {
    // The CLI writes the IDENTICAL row whatever the auth source is, and nothing
    // on the wire names the credential type — so a console-billed user with a
    // genuinely empty balance reads this same text, and must not be sent round
    // a /logout, /login loop that cannot fix anything.
    expect(CREDIT_BALANCE_REMEDY).toContain('API key');
    expect(CREDIT_BALANCE_REMEDY).toContain('Anthropic Console');
  });

  it('leads with the subscription case, which is the common one', () => {
    expect(CREDIT_BALANCE_REMEDY.indexOf('subscription')).toBeLessThan(
      CREDIT_BALANCE_REMEDY.indexOf('API key'),
    );
  });
});
