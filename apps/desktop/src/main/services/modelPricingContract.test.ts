// Cross-language pricing drift guard.
//
// contracts/model-pricing-cases.json is the SHARED fixture: a Rust test
// (pricing.rs) consumes the exact same file. Every case pins a concrete model id
// to its USD-per-million input/output rate. Here we derive the effective rate by
// pricing 1,000,000 tokens on one side at a time — turnCostUSD returns USD, and
// 1M tokens ⇒ the per-million rate — and assert it matches the fixture. If TS
// (MODEL_RATES) and Rust (BUILTIN) ever disagree on a model's price, one side's
// contract test fails.

import { describe, it, expect } from 'vitest';
import { SweepTally, itSweptTheWholeCorpus } from '../../../tests/support/sweepTally';
import { readFileSync } from 'fs';
import { fileURLToPath } from 'url';
import * as path from 'path';
import { turnCostUSD } from './modelUsage';
import type { RawUsage } from './modelUsage';

interface PricingCase {
  model: string;
  input: number;
  output: number;
  note?: string;
}

/** One `cacheMultiplierCases` row. The tokens a turn reports, and what the whole
 *  turn must cost. See the block's `why` in the fixture. */
interface CacheMultiplierCase {
  name: string;
  model: string;
  cacheWriteTokens: number;
  ephemeral5m?: number;
  ephemeral1h?: number;
  cacheReadTokens?: number;
  inputTokens?: number;
  outputTokens?: number;
  expectedUSD: number;
  note: string;
}

// This file lives at apps/desktop/src/main/services/ — five levels below the
// repo root, where contracts/ sits.
const here = path.dirname(fileURLToPath(import.meta.url));
const fixturePath = path.resolve(
  here,
  '..',
  '..',
  '..',
  '..',
  '..',
  'contracts',
  'model-pricing-cases.json',
);
const fixture = JSON.parse(readFileSync(fixturePath, 'utf-8')) as {
  cases: PricingCase[];
  cacheMultiplierCases: CacheMultiplierCase[];
};

/** Rebuild the raw transcript `usage` block a case describes. Omitting BOTH
 *  ephemeral fields omits `cache_creation` entirely. That is the no-split case,
 *  and it has to be a genuine absence rather than a pair of zeros, because zeros
 *  are a turn that split its writes and wrote none. */
function usageFor(c: CacheMultiplierCase): RawUsage {
  const usage: RawUsage = {
    input_tokens: c.inputTokens ?? 0,
    output_tokens: c.outputTokens ?? 0,
    cache_creation_input_tokens: c.cacheWriteTokens,
    cache_read_input_tokens: c.cacheReadTokens ?? 0,
  };
  if (c.ephemeral5m !== undefined || c.ephemeral1h !== undefined) {
    usage.cache_creation = {};
    if (c.ephemeral5m !== undefined) usage.cache_creation.ephemeral_5m_input_tokens = c.ephemeral5m;
    if (c.ephemeral1h !== undefined) usage.cache_creation.ephemeral_1h_input_tokens = c.ephemeral1h;
  }
  return usage;
}

describe('model pricing contract (shared with Rust pricing.rs)', () => {
  it('the fixture loads and has cases', () => {
    expect(Array.isArray(fixture.cases)).toBe(true);
    expect(fixture.cases.length).toBeGreaterThan(0);
  });

  // A floor of ONE ("has cases") is met by a corpus that lost all but one of its
  // rows, and this corpus is the only thing holding the TS pricing table to the
  // Rust one.
  const tally = new SweepTally();
  for (const c of fixture.cases) {
    it(`${c.model} → input ${c.input}/M, output ${c.output}/M${c.note ? ` (${c.note})` : ''}`, () => {
      tally.ran('other');
      const inputRate = turnCostUSD(c.model, { input_tokens: 1_000_000 });
      const outputRate = turnCostUSD(c.model, { output_tokens: 1_000_000 });
      expect(inputRate).toBeCloseTo(c.input, 6);
      expect(outputRate).toBeCloseTo(c.output, 6);
    });
  }
  itSweptTheWholeCorpus(tally, 'the model-pricing corpus', 11, { allow: 0, deny: 0 });
});

// The other half of costing: a cache write costs a multiple of the input rate,
// and the multiplier depends on how long the write is kept alive. Both engines
// hardcoded 1.25 (the 5-minute rate) until 2026-08-24 while this project's
// sessions ran on the 1-hour TTL, which bills at 2x, so every displayed cost
// understated its write component. The Rust twin reads these exact rows.
describe('cache multiplier contract (shared with Rust usage.rs)', () => {
  it('the fixture loads and has cases', () => {
    expect(Array.isArray(fixture.cacheMultiplierCases)).toBe(true);
    expect(fixture.cacheMultiplierCases.length).toBeGreaterThan(0);
  });

  const tally = new SweepTally();
  for (const c of fixture.cacheMultiplierCases) {
    it(`${c.name} (${c.note})`, () => {
      tally.ran('other');
      expect(turnCostUSD(c.model, usageFor(c))).toBeCloseTo(c.expectedUSD, 6);
    });
  }
  itSweptTheWholeCorpus(tally, 'the cache-multiplier corpus', 9, { allow: 0, deny: 0 });
});
