// Characterization tests for src/main/services/modelUsage.ts
// These tests describe CURRENT behavior; do not change them to match a
// desired future behavior.

import { describe, it, expect } from 'vitest';
import {
  contextTokensOf,
  contextLimitFor,
  turnCostUSD,
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
// turnCostUSD — per-model rate table + cache multipliers
// ---------------------------------------------------------------------------
describe('turnCostUSD', () => {
  // Helper: turn 1M tokens into a cost to verify the formula
  // USD = (input * r.input + cacheWrite * r.input*1.25 + cacheRead * r.input*0.1 + output * r.output) / 1_000_000

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

    it('cache-write tokens cost 1.25× input rate', () => {
      // 1M cache-write tokens at opus: 5 * 1.25 = 6.25
      const cost = turnCostUSD(model, { cache_creation_input_tokens: 1_000_000 });
      expect(cost).toBeCloseTo(6.25, 6);
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
      // (100*5 + 300*6.25 + 400*0.5 + 200*25) / 1_000_000
      // = (500 + 1875 + 200 + 5000) / 1_000_000
      // = 7575 / 1_000_000
      const expected = 7_575 / 1_000_000;
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
  });

  describe('claude-sonnet rates (input=3, output=15)', () => {
    const model = 'claude-sonnet-4-5';

    it('plain input tokens only', () => {
      expect(turnCostUSD(model, { input_tokens: 1_000_000 })).toBeCloseTo(3, 6);
    });

    it('output tokens only', () => {
      expect(turnCostUSD(model, { output_tokens: 1_000_000 })).toBeCloseTo(15, 6);
    });

    it('cache-write tokens cost 1.25× sonnet input rate (3.75)', () => {
      expect(turnCostUSD(model, { cache_creation_input_tokens: 1_000_000 })).toBeCloseTo(3.75, 6);
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

    it('cache-write tokens cost 1.25× haiku input rate (1.25)', () => {
      expect(turnCostUSD(model, { cache_creation_input_tokens: 1_000_000 })).toBeCloseTo(1.25, 6);
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
