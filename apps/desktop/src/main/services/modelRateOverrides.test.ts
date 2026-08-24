import { describe, it, expect, vi, beforeEach } from 'vitest';

// Back the module's rates file with an in-memory blob so we can exercise the
// override path without touching the real ~/.workspacer/model-rates.json.
let fileContent: string | null = null; // null = file absent
let fileMtime = 1;

vi.mock('fs', () => ({
  default: {
    statSync: () => {
      if (fileContent === null) throw new Error('ENOENT');
      return { mtimeMs: fileMtime };
    },
    readFileSync: () => {
      if (fileContent === null) throw new Error('ENOENT');
      return fileContent;
    },
    mkdirSync: () => {},
    rmSync: () => {
      fileContent = null;
      fileMtime += 1;
    },
  },
}));
vi.mock('os', () => ({ default: { homedir: () => '/home/test' } }));
vi.mock('../lib/atomicWriteFile', () => ({
  atomicWriteFileSync: (_p: string, data: string) => {
    fileContent = data;
    fileMtime += 1;
  },
}));

import {
  turnCostUSD,
  contextLimitFor,
  readModelRateOverrides,
  writeModelRateOverrides,
} from './modelUsage';

describe('model rate overrides', () => {
  beforeEach(() => {
    writeModelRateOverrides({}); // clears the file + the mtime cache
  });

  it('falls back to built-in rates when there is no override', () => {
    expect(turnCostUSD('claude-fable-5', { input_tokens: 1_000_000 })).toBe(10);
    expect(contextLimitFor('claude-fable-5', 0)).toBe(1_000_000); // 1M-native
  });

  it('an override re-rates both cost and context window', () => {
    writeModelRateOverrides({
      'claude-fable': { input: 1, output: 2, context_limit: 1_000_000 },
    });
    expect(turnCostUSD('claude-fable-5', { input_tokens: 1_000_000 })).toBe(1);
    expect(turnCostUSD('claude-fable-5', { output_tokens: 1_000_000 })).toBe(2);
    expect(contextLimitFor('claude-fable-5', 0)).toBe(1_000_000);
    expect(readModelRateOverrides()['claude-fable'].input).toBe(1);
  });

  it('a same-length override beats the built-in of the same prefix', () => {
    writeModelRateOverrides({ 'claude-opus': { input: 7, output: 70 } });
    // 'claude-opus-4-8-…' matches generic 'claude-opus'; override wins the tie.
    expect(turnCostUSD('claude-opus-4-8-20260101', { input_tokens: 1_000_000 })).toBe(7);
  });

  it('an override cached_input re-rates cache reads (parity with claudemon usage.rs)', () => {
    // usage.rs turn_cost_usd() bills cache reads at
    //   r.cached_input.unwrap_or(r.input * 0.1)
    // so an override with a custom cached_input must be honored on the TS side
    // too — otherwise the desktop cost readout diverges from claudemon's for
    // the identical transcript.
    writeModelRateOverrides({
      'claude-sonnet': { input: 3, output: 15, cached_input: 1.5 },
    });
    // 1M cache-read tokens: override says 1.5 → $1.50, NOT 3 * 0.1 = $0.30.
    expect(turnCostUSD('claude-sonnet-4-5', { cache_read_input_tokens: 1_000_000 })).toBeCloseTo(
      1.5,
      6,
    );
    // Plain input + cache-write are unaffected by the cached_input override.
    // The write carries no TTL split, so it takes the 1-hour fallback: 3 * 2.
    expect(turnCostUSD('claude-sonnet-4-5', { input_tokens: 1_000_000 })).toBeCloseTo(3, 6);
    expect(
      turnCostUSD('claude-sonnet-4-5', { cache_creation_input_tokens: 1_000_000 }),
    ).toBeCloseTo(6, 6);
  });

  it('without a cached_input override, cache reads still fall back to 0.1× input', () => {
    writeModelRateOverrides({ 'claude-sonnet': { input: 3, output: 15 } });
    expect(turnCostUSD('claude-sonnet-4-5', { cache_read_input_tokens: 1_000_000 })).toBeCloseTo(
      0.3,
      6,
    );
  });

  it('a wrong-typed OPTIONAL field does not discard the whole override (parity with pricing.rs)', () => {
    // The shared ~/.workspacer/model-rates.json is read by BOTH this engine and
    // claudemon's pricing.rs. An entry with numeric input+output must APPLY even
    // when an optional field is wrong-typed — the bad optional is treated as
    // absent, not as grounds to drop the whole entry. pricing.rs used to
    // deserialize the entry as a unit and fell back to the built-in $3 sonnet
    // rate on a fractional context_limit / string cached_input while this side
    // applied the $5 override: same file, 40% per-turn cost divergence.
    for (const bad of [
      { input: 5, output: 25, context_limit: 1_000_000.5 },
      { input: 5, output: 25, context_limit: -1 },
      { input: 5, output: 25, cached_input: 'oops' },
    ]) {
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      writeModelRateOverrides({ 'claude-sonnet': bad as any });
      expect(turnCostUSD('claude-sonnet-4-6', { input_tokens: 1_000_000 })).toBe(5);
      expect(turnCostUSD('claude-sonnet-4-6', { output_tokens: 1_000_000 })).toBe(25);
      // The wrong-typed window is discarded, inheriting the built-in sonnet 200k
      // rather than the fractional/negative value the Rust reader also rejects.
      expect(contextLimitFor('claude-sonnet-4-6', 0)).toBe(200_000);
    }
  });

  it('reset (empty map) reverts to built-ins', () => {
    writeModelRateOverrides({ 'claude-fable': { input: 1, output: 2 } });
    expect(turnCostUSD('claude-fable-5', { input_tokens: 1_000_000 })).toBe(1);
    writeModelRateOverrides({});
    expect(turnCostUSD('claude-fable-5', { input_tokens: 1_000_000 })).toBe(10);
    expect(readModelRateOverrides()).toEqual({});
  });
});
