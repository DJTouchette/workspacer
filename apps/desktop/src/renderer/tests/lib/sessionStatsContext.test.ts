import { describe, expect, it } from 'vitest';
import { deriveSessionStats } from '../../src/lib/sessionStats';

const usage = (contextTokens: number, contextLimit: number | null, billed = 75_000_000) =>
  ({
    contextTokens,
    contextLimit,
    totalInputTokens: billed - 1_000,
    totalOutputTokens: 1_000,
    costUSD: 0,
  }) as any;

describe('runtime-confirmed active-context selector shared by both bars', () => {
  it('never turns a requested/resolved 1M estimate into a percentage before confirmation', () => {
    const stats = deriveSessionStats({ usage: usage(129_200, 1_000_000) });
    expect(stats.ctxPct).toBeUndefined();
    expect(stats.effectiveContextWindow).toBeUndefined();
    expect(stats.billedTokens).toBe(75_000_000);
  });

  it('late runtime correction uses 258400, while cumulative throughput remains separate', () => {
    const stats = deriveSessionStats({
      usage: usage(129_200, 1_000_000),
      statusLine: {
        contextWindowSize: 258_400,
        totalInputTokens: 74_000_000,
        totalOutputTokens: 1_000_000,
      },
    });
    expect(stats.ctxPct).toBe(50);
    expect(stats.contextTokens).toBe(129_200);
    expect(stats.effectiveContextWindow).toBe(258_400);
    expect(stats.billedTokens).toBe(75_000_000);
  });

  it('a compaction occupancy reset immediately lowers the bar against the same effective window', () => {
    const before = deriveSessionStats({
      usage: usage(129_200, 258_400),
      statusLine: { contextWindowSize: 258_400, contextUsedPct: 88 },
    });
    const after = deriveSessionStats({
      // The transcript accumulator can lag compaction; the newer provider
      // occupancy percentage is the active numerator over the same runtime
      // denominator and must win immediately.
      usage: usage(129_200, 258_400),
      statusLine: { contextWindowSize: 258_400, contextUsedPct: 10 },
    });
    expect(before.ctxPct).toBe(88);
    expect(after.ctxPct).toBe(10);
  });
});
