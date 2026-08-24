// Characterization tests for src/main/services/modelUsage.ts
// These tests describe CURRENT behavior; do not change them to match a
// desired future behavior.

import { describe, it, expect } from 'vitest';
import {
  contextTokensOf,
  contextLimitFor,
  requestedWindowFor,
  turnCostUSD,
  cacheSplitOf,
  emptyUsage,
  type RawUsage,
  type SessionUsage,
} from './modelUsage';

// ---------------------------------------------------------------------------
// contextTokensOf
// ---------------------------------------------------------------------------
describe('contextTokensOf', () => {
  it('returns 0 for empty usage', () => {
    expect(contextTokensOf({})).toBe(0);
  });

  it('sums input + cache_creation + cache_read tokens', () => {
    const usage: RawUsage = {
      input_tokens: 100,
      cache_creation_input_tokens: 50,
      cache_read_input_tokens: 25,
    };
    expect(contextTokensOf(usage)).toBe(175);
  });

  it('treats undefined fields as 0', () => {
    expect(contextTokensOf({ input_tokens: 300 })).toBe(300);
    expect(contextTokensOf({ cache_creation_input_tokens: 200 })).toBe(200);
    expect(contextTokensOf({ cache_read_input_tokens: 150 })).toBe(150);
  });

  it('ignores output_tokens', () => {
    const usage: RawUsage = { input_tokens: 10, output_tokens: 9999 };
    expect(contextTokensOf(usage)).toBe(10);
  });
});

// ---------------------------------------------------------------------------
// contextLimitFor — model-rate table entries + 200k→1M heuristic
// ---------------------------------------------------------------------------
describe('contextLimitFor', () => {
  it('claude-opus: base 200_000 when observed <= 200k', () => {
    expect(contextLimitFor('claude-opus-4', 100_000)).toBe(200_000);
  });

  it('fable/mythos: 1M-native from the first turn (no promotion gate)', () => {
    expect(contextLimitFor('claude-fable-5', 1_000)).toBe(1_000_000);
    expect(contextLimitFor('claude-mythos-1', 1_000)).toBe(1_000_000);
  });

  it('claude-opus: promotes to 1_000_000 when observed > 200k', () => {
    expect(contextLimitFor('claude-opus-4', 200_001)).toBe(1_000_000);
  });

  it('claude-sonnet: base 200_000 when observed <= 200k', () => {
    expect(contextLimitFor('claude-sonnet-4-5', 50_000)).toBe(200_000);
  });

  it('claude-sonnet: promotes to 1_000_000 when observed > 200k', () => {
    expect(contextLimitFor('claude-sonnet-4-5', 250_000)).toBe(1_000_000);
  });

  it('claude-haiku: base 200_000 when observed <= 200k', () => {
    expect(contextLimitFor('claude-haiku-3-5', 1_000)).toBe(200_000);
  });

  it('claude-haiku: promotes to 1_000_000 when observed > 200k', () => {
    expect(contextLimitFor('claude-haiku-3-5', 500_000)).toBe(1_000_000);
  });

  it('unknown model falls back to default 200_000 contextLimit', () => {
    expect(contextLimitFor('gpt-4', 10_000)).toBe(200_000);
  });

  it('unknown model also promotes to 1_000_000 when observed > 200k', () => {
    expect(contextLimitFor('gpt-4', 300_000)).toBe(1_000_000);
  });

  it('null model uses default 200_000', () => {
    expect(contextLimitFor(null, 0)).toBe(200_000);
  });

  it('undefined model uses default 200_000', () => {
    expect(contextLimitFor(undefined, 0)).toBe(200_000);
  });

  it('boundary: exactly 200_000 observed does NOT promote', () => {
    expect(contextLimitFor('claude-sonnet-4-5', 200_000)).toBe(200_000);
  });
});

// ---------------------------------------------------------------------------
// requestedWindowFor — the `[1m]` marker the transcript model id strips
// ---------------------------------------------------------------------------
describe('requestedWindowFor', () => {
  it('reads the [1m] marker off a bare alias and a full id', () => {
    expect(requestedWindowFor('opus[1m]')).toBe(1_000_000);
    expect(requestedWindowFor('sonnet[1m]')).toBe(1_000_000);
    expect(requestedWindowFor('claude-opus-5[1m]')).toBe(1_000_000);
    expect(requestedWindowFor('claude-sonnet-5-1m')).toBe(1_000_000);
  });

  it('treats fable/mythos as 1M-native (they carry no marker)', () => {
    expect(requestedWindowFor('fable')).toBe(1_000_000);
    expect(requestedWindowFor('claude-mythos-5')).toBe(1_000_000);
  });

  it('says NOTHING (not 200k) for an unmarked alias — an absent marker is not a 200k claim', () => {
    expect(requestedWindowFor('opus')).toBeUndefined();
    expect(requestedWindowFor('claude-opus-5')).toBeUndefined();
    expect(requestedWindowFor('')).toBeUndefined();
    expect(requestedWindowFor(null)).toBeUndefined();
  });
});

// ---------------------------------------------------------------------------
// contextLimitFor — real signals outrank the retrospective promotion
// ---------------------------------------------------------------------------
describe('contextLimitFor — hints', () => {
  it('THE BUG: a 1M session under 200k reported 200k; the requested alias fixes it at token zero', () => {
    // 190k of a 1M window read as 95% full, which is what made a healthy
    // manager conclude it was about to die.
    expect(contextLimitFor('claude-opus-5', 190_000)).toBe(200_000);
    expect(contextLimitFor('claude-opus-5', 190_000, { requestedModel: 'opus[1m]' })).toBe(
      1_000_000,
    );
    // …and from the very first turn, not just near the old crossover.
    expect(contextLimitFor('claude-opus-5', 0, { requestedModel: 'opus[1m]' })).toBe(1_000_000);
  });

  it('the provider-reported window wins over every guess', () => {
    expect(contextLimitFor('claude-opus-5', 1_000, { reportedWindow: 1_000_000 })).toBe(1_000_000);
    // Including over the retrospective promotion: if the provider says 200k
    // while a stale high-water mark says otherwise, the provider is right.
    expect(
      contextLimitFor('claude-opus-5', 300_000, {
        reportedWindow: 200_000,
        requestedModel: 'opus',
      }),
    ).toBe(200_000);
  });

  it('ignores a non-positive / non-finite reported window and falls through', () => {
    expect(contextLimitFor('claude-opus-5', 1_000, { reportedWindow: 0 })).toBe(200_000);
    expect(contextLimitFor('claude-opus-5', 1_000, { reportedWindow: null })).toBe(200_000);
    expect(contextLimitFor('claude-opus-5', 1_000, { reportedWindow: NaN })).toBe(200_000);
  });

  it('does NOT default everything to 1M — an unmarked request stays 200k', () => {
    expect(contextLimitFor('claude-opus-5', 50_000, { requestedModel: 'opus' })).toBe(200_000);
    expect(contextLimitFor('claude-sonnet-5', 50_000, { requestedModel: '' })).toBe(200_000);
    expect(contextLimitFor('claude-haiku-4-5', 50_000, {})).toBe(200_000);
  });

  it('a requested alias may raise the window but never lower one already resolved higher', () => {
    // Fable is 1M in the table; a coarse `opus` request must not drag it down.
    expect(contextLimitFor('claude-fable-5', 1_000, { requestedModel: 'opus' })).toBe(1_000_000);
  });

  it('the retrospective promotion still covers a session with no signals at all', () => {
    expect(contextLimitFor('claude-opus-5', 300_000)).toBe(1_000_000);
    expect(contextLimitFor('claude-opus-5', 300_000, {})).toBe(1_000_000);
  });
});

// ---------------------------------------------------------------------------
// turnCostUSD — per-model rate table + cache multipliers
// ---------------------------------------------------------------------------
describe('turnCostUSD', () => {
  // Helper: turn 1M tokens into a cost to verify the formula
  //   USD = (input * r.input
  //          + cacheWrite priced per TTL (1.25× at 5m, 2× at 1h)
  //          + cacheRead * r.input*0.1
  //          + output * r.output) / 1_000_000
  //
  // A `cache_creation_input_tokens` count with no `cache_creation` block cannot
  // say which lifetime it bought, and takes the documented 1-hour fallback, so
  // the cases below that pass writes alone are fallback cases, not 5m cases.

  describe('claude-opus rates (input=5, output=25)', () => {
    const model = 'claude-opus-4-8';

    it('plain input tokens only', () => {
      const cost = turnCostUSD(model, { input_tokens: 1_000_000 });
      expect(cost).toBeCloseTo(5, 6);
    });

    it('output tokens only', () => {
      const cost = turnCostUSD(model, { output_tokens: 1_000_000 });
      expect(cost).toBeCloseTo(25, 6);
    });

    it('cache-write tokens with no TTL split fall back to the 1-hour rate', () => {
      // 1M cache-write tokens at opus: 5 * 2 = 10
      const cost = turnCostUSD(model, { cache_creation_input_tokens: 1_000_000 });
      expect(cost).toBeCloseTo(10, 6);
    });

    it('cache-read tokens cost 0.1× input rate', () => {
      // 1M cache-read tokens at opus: 5 * 0.1 = 0.5
      const cost = turnCostUSD(model, { cache_read_input_tokens: 1_000_000 });
      expect(cost).toBeCloseTo(0.5, 6);
    });

    it('combined all token types', () => {
      const usage: RawUsage = {
        input_tokens: 100,
        output_tokens: 200,
        cache_creation_input_tokens: 300,
        cache_read_input_tokens: 400,
      };
      // The writes carry no TTL split, so they take the 1-hour fallback ($10/M).
      // (100*5 + 300*10 + 400*0.5 + 200*25) / 1_000_000
      // = (500 + 3000 + 200 + 5000) / 1_000_000
      // = 8700 / 1_000_000
      const expected = 8_700 / 1_000_000;
      expect(turnCostUSD(model, usage)).toBeCloseTo(expected, 10);
    });
  });

  describe('claude-fable rates (input=10, output=50)', () => {
    const model = 'claude-fable-5';

    it('plain input tokens only', () => {
      expect(turnCostUSD(model, { input_tokens: 1_000_000 })).toBeCloseTo(10, 6);
    });

    it('output tokens only', () => {
      expect(turnCostUSD(model, { output_tokens: 1_000_000 })).toBeCloseTo(50, 6);
    });
  });

  describe('legacy opus rates (input=15, output=75)', () => {
    it('claude-opus-4-1 keeps the older pricing', () => {
      expect(turnCostUSD('claude-opus-4-1-20250805', { input_tokens: 1_000_000 })).toBeCloseTo(
        15,
        6,
      );
      expect(turnCostUSD('claude-opus-4-0', { output_tokens: 1_000_000 })).toBeCloseTo(75, 6);
    });

    it("claude-opus-4-20250514 (Opus 4.0's real dated id) prices at 15/75, not the generic opus 5/25", () => {
      // The transcript carries the dated id, which does NOT start with the
      // 'claude-opus-4-0' alias — it needs the 'claude-opus-4-20' entry.
      expect(turnCostUSD('claude-opus-4-20250514', { input_tokens: 1_000_000 })).toBeCloseTo(15, 6);
      expect(turnCostUSD('claude-opus-4-20250514', { output_tokens: 1_000_000 })).toBeCloseTo(
        75,
        6,
      );
    });

    it('claude-3-opus-20240229 prices at 15/75, not the 3/15 default', () => {
      expect(turnCostUSD('claude-3-opus-20240229', { input_tokens: 1_000_000 })).toBeCloseTo(15, 6);
      expect(turnCostUSD('claude-3-opus-20240229', { output_tokens: 1_000_000 })).toBeCloseTo(
        75,
        6,
      );
    });

    it('dated Opus 4.5+ ids still get current opus rates (no over-greedy legacy match)', () => {
      expect(turnCostUSD('claude-opus-4-5-20251101', { input_tokens: 1_000_000 })).toBeCloseTo(
        5,
        6,
      );
    });

    it('Opus minor versions 4.10–4.19 get current opus rates, not the legacy 4.1 rate', () => {
      // 'claude-opus-4-1' must not swallow '4-10'…'4-19'; those are current
      // Opus generations and price at 5/25, not the legacy 15/75.
      expect(turnCostUSD('claude-opus-4-10-20260101', { input_tokens: 1_000_000 })).toBeCloseTo(
        5,
        6,
      );
      expect(turnCostUSD('claude-opus-4-15', { output_tokens: 1_000_000 })).toBeCloseTo(25, 6);
    });
  });

  describe('claude-sonnet rates (input=3, output=15)', () => {
    const model = 'claude-sonnet-4-5';

    it('plain input tokens only', () => {
      expect(turnCostUSD(model, { input_tokens: 1_000_000 })).toBeCloseTo(3, 6);
    });

    it('output tokens only', () => {
      expect(turnCostUSD(model, { output_tokens: 1_000_000 })).toBeCloseTo(15, 6);
    });

    it('cache-write tokens with no TTL split fall back to 2× sonnet input (6)', () => {
      expect(turnCostUSD(model, { cache_creation_input_tokens: 1_000_000 })).toBeCloseTo(6, 6);
    });

    it('cache-read tokens cost 0.1× sonnet input rate (0.3)', () => {
      expect(turnCostUSD(model, { cache_read_input_tokens: 1_000_000 })).toBeCloseTo(0.3, 6);
    });
  });

  describe('claude-haiku rates (input=1, output=5)', () => {
    const model = 'claude-haiku-3-5';

    it('plain input tokens only', () => {
      expect(turnCostUSD(model, { input_tokens: 1_000_000 })).toBeCloseTo(1, 6);
    });

    it('output tokens only', () => {
      expect(turnCostUSD(model, { output_tokens: 1_000_000 })).toBeCloseTo(5, 6);
    });

    it('cache-write tokens with no TTL split fall back to 2× haiku input (2)', () => {
      expect(turnCostUSD(model, { cache_creation_input_tokens: 1_000_000 })).toBeCloseTo(2, 6);
    });

    it('cache-read tokens cost 0.1× haiku input rate (0.1)', () => {
      expect(turnCostUSD(model, { cache_read_input_tokens: 1_000_000 })).toBeCloseTo(0.1, 6);
    });
  });

  describe('default / unknown model rates (input=3, output=15)', () => {
    it('null model uses default rates', () => {
      expect(turnCostUSD(null, { input_tokens: 1_000_000 })).toBeCloseTo(3, 6);
    });

    it('undefined model uses default rates', () => {
      expect(turnCostUSD(undefined, { output_tokens: 1_000_000 })).toBeCloseTo(15, 6);
    });

    it('unrecognised model string uses default rates', () => {
      expect(turnCostUSD('gpt-99', { input_tokens: 1_000_000 })).toBeCloseTo(3, 6);
    });
  });

  describe('longest-prefix-first matching', () => {
    it('claude-opus prefix matches any claude-opus-* variant', () => {
      // Both should use current opus rates (5/25); the longer claude-opus-4-1
      // prefix only diverts the legacy models.
      const costA = turnCostUSD('claude-opus-4-7', { input_tokens: 1_000_000 });
      const costB = turnCostUSD('claude-opus-4-5', { input_tokens: 1_000_000 });
      expect(costA).toBeCloseTo(5, 6);
      expect(costB).toBeCloseTo(5, 6);
    });

    it('claude-sonnet prefix matches any claude-sonnet-* variant', () => {
      const cost = turnCostUSD('claude-sonnet-4-6', { input_tokens: 1_000_000 });
      expect(cost).toBeCloseTo(3, 6);
    });

    it('claude-haiku prefix matches any claude-haiku-* variant', () => {
      const cost = turnCostUSD('claude-haiku-3', { input_tokens: 1_000_000 });
      expect(cost).toBeCloseTo(1, 6);
    });
  });

  it('returns 0 for all-zero usage', () => {
    expect(turnCostUSD('claude-opus-4', {})).toBe(0);
  });
});

// ---------------------------------------------------------------------------
// emptyUsage — shape and zero values
// ---------------------------------------------------------------------------
// cache-write TTL multipliers
// ---------------------------------------------------------------------------
//
// A cache write costs a multiple of the input rate, and the multiple depends on
// how long the write is kept alive. Both costing engines hardcoded 1.25 (the
// 5-minute rate) until 2026-08-24 while this project's sessions ran on the
// 1-hour TTL, which bills at 2x, so the write component of every displayed
// cost was understated. `usage.cache_creation` has carried the answer all along.
describe('turnCostUSD cache-write TTL multipliers', () => {
  const opus = 'claude-opus-4-8'; // input $5/M

  it('prices a 1-hour write at 2× the input rate', () => {
    // THE BUG: this returned $6.25 while the account was billed $10.
    const cost = turnCostUSD(opus, {
      cache_creation_input_tokens: 1_000_000,
      cache_creation: { ephemeral_5m_input_tokens: 0, ephemeral_1h_input_tokens: 1_000_000 },
    });
    expect(cost).toBeCloseTo(10, 6);
  });

  it('prices a 5-minute write at 1.25× the input rate', () => {
    const cost = turnCostUSD(opus, {
      cache_creation_input_tokens: 1_000_000,
      cache_creation: { ephemeral_5m_input_tokens: 1_000_000, ephemeral_1h_input_tokens: 0 },
    });
    expect(cost).toBeCloseTo(6.25, 6);
  });

  it('prices each portion of a mixed-TTL turn at its own rate', () => {
    // 400k at $6.25/M + 600k at $10/M. No single blended multiplier gets here.
    const cost = turnCostUSD(opus, {
      cache_creation_input_tokens: 1_000_000,
      cache_creation: { ephemeral_5m_input_tokens: 400_000, ephemeral_1h_input_tokens: 600_000 },
    });
    expect(cost).toBeCloseTo(8.5, 6);
  });

  it('falls back to the 1-hour rate when a turn reports writes but no TTL split', () => {
    // The cheaper rate is exactly the bug being fixed, so the fallback is the
    // dearer one: a cost that reads lower than the bill is the worse failure.
    expect(turnCostUSD(opus, { cache_creation_input_tokens: 1_000_000 })).toBeCloseTo(10, 6);
  });

  it('takes the same fallback for writes the TTL split does not account for', () => {
    // 250k at $6.25/M + 250k at $10/M + the unattributed 500k at $10/M.
    const cost = turnCostUSD(opus, {
      cache_creation_input_tokens: 1_000_000,
      cache_creation: { ephemeral_5m_input_tokens: 250_000, ephemeral_1h_input_tokens: 250_000 },
    });
    expect(cost).toBeCloseTo(9.0625, 6);
  });

  it('leaves the cache-read multiplier at 0.1× input', () => {
    expect(turnCostUSD(opus, { cache_read_input_tokens: 1_000_000 })).toBeCloseTo(0.5, 6);
  });
});

// ---------------------------------------------------------------------------
// cacheSplitOf
// ---------------------------------------------------------------------------
describe('cacheSplitOf', () => {
  it('splits a turn that reports cache fields', () => {
    expect(
      cacheSplitOf({
        input_tokens: 2,
        cache_creation_input_tokens: 23_393,
        cache_read_input_tokens: 17_589,
      }),
    ).toEqual({ fresh: 2, write: 23_393, read: 17_589 });
  });

  it('returns null for a turn that reports no cache fields at all', () => {
    // Null, not three zeros: a provider that itemizes nothing must not read as
    // one whose cache never hit.
    expect(cacheSplitOf({ input_tokens: 100, output_tokens: 20 })).toBeNull();
    expect(cacheSplitOf({})).toBeNull();
  });

  it('treats a reported zero as reported', () => {
    expect(cacheSplitOf({ input_tokens: 100, cache_read_input_tokens: 0 })).toEqual({
      fresh: 100,
      write: 0,
      read: 0,
    });
  });
});

// ---------------------------------------------------------------------------
describe('emptyUsage', () => {
  it('returns an object with the correct shape', () => {
    const u: SessionUsage = emptyUsage();
    expect(u).toMatchObject({
      model: null,
      contextTokens: 0,
      contextLimit: 200_000,
      totalInputTokens: 0,
      totalOutputTokens: 0,
      costUSD: 0,
    });
  });

  it('returns a fresh object each call (no shared reference)', () => {
    const a = emptyUsage();
    const b = emptyUsage();
    a.costUSD = 99;
    expect(b.costUSD).toBe(0);
  });

  it('default contextLimit matches the default model context window (200k)', () => {
    expect(emptyUsage().contextLimit).toBe(200_000);
  });
});
