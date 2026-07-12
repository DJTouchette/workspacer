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
import { readFileSync } from 'fs';
import { fileURLToPath } from 'url';
import * as path from 'path';
import { turnCostUSD } from './modelUsage';

interface PricingCase {
  model: string;
  input: number;
  output: number;
  note?: string;
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
const fixture = JSON.parse(readFileSync(fixturePath, 'utf-8')) as { cases: PricingCase[] };

describe('model pricing contract (shared with Rust pricing.rs)', () => {
  it('the fixture loads and has cases', () => {
    expect(Array.isArray(fixture.cases)).toBe(true);
    expect(fixture.cases.length).toBeGreaterThan(0);
  });

  for (const c of fixture.cases) {
    it(`${c.model} → input ${c.input}/M, output ${c.output}/M${c.note ? ` (${c.note})` : ''}`, () => {
      const inputRate = turnCostUSD(c.model, { input_tokens: 1_000_000 });
      const outputRate = turnCostUSD(c.model, { output_tokens: 1_000_000 });
      expect(inputRate).toBeCloseTo(c.input, 6);
      expect(outputRate).toBeCloseTo(c.output, 6);
    });
  }
});
