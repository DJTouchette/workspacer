import { describe, it, expect } from 'vitest';
import {
  cacheBreakdown,
  deriveSessionStats,
  fmtTokens,
  fmtWindowLength,
  fmtWindowShort,
  isSnapshotStale,
  summarizeFileChanges,
  usageWindows,
  STALE_AFTER_MS,
} from './sessionStats';
import type { FileChange, SessionStatusLine, SessionUsage } from '../types/claudeSession';

const usage = (over: Partial<SessionUsage> = {}): SessionUsage =>
  ({
    model: 'claude-sonnet-4-6',
    contextTokens: 0,
    contextLimit: 200_000,
    totalInputTokens: 111,
    totalOutputTokens: 222,
    costUSD: 1,
    ...over,
  }) as SessionUsage;

describe('deriveSessionStats — cumulative BILLED tokens', () => {
  it('uses the statusLine when only totalOutputTokens is present', () => {
    const sl = { totalOutputTokens: 500 } as SessionStatusLine;
    // statusLine is authoritative; even with only output tokens it should win
    // over the transcript-derived usage fallback (which would give 111+222=333).
    expect(deriveSessionStats({ statusLine: sl, usage: usage() }).billedTokens).toBe(500);
  });

  it('uses the statusLine when only totalInputTokens is present', () => {
    const sl = { totalInputTokens: 400 } as SessionStatusLine;
    expect(deriveSessionStats({ statusLine: sl, usage: usage() }).billedTokens).toBe(400);
  });

  it('sums statusLine input+output when both present', () => {
    const sl = { totalInputTokens: 400, totalOutputTokens: 500 } as SessionStatusLine;
    expect(deriveSessionStats({ statusLine: sl, usage: usage() }).billedTokens).toBe(900);
  });

  it('falls back to usage when statusLine carries no token counts', () => {
    expect(
      deriveSessionStats({ statusLine: {} as SessionStatusLine, usage: usage() }).billedTokens,
    ).toBe(333);
  });
});

// ── THE 23M WORKER ───────────────────────────────────────────────────────────
//
// Pinned to a session captured live from claudemon on 2026-08-27 (a dispatched
// worker in .../workspacer-copilot-provider-parity-build, spawned `opus[1m]`).
// Its raw frames, deduped by message id exactly as both accumulators do:
//
//   141 assistant messages, 30,328,545 prompt tokens billed, of which
//   29,996,965 were cache reads — matching claudemon's own
//   `cache: {fresh: 282, read: 29,996,965, write: 331,298}` to the token.
//   Occupancy at that instant: 356,380. Claude Code's statusLine, for that same
//   session: `contextUsedPct: 100, contextWindowSize: 200000`.
//
// So the 30M is HONEST — 141 turns each re-sending ~215k of cached conversation
// — and it was the occupancy readouts that were wrong. These cases hold the two
// figures apart and hold the bar to the window the session has not disproved.
const WORKER_23M = {
  usage: usage({
    model: 'claude-opus-5',
    contextTokens: 356_380,
    // What resolveContextWindow now returns for this session: the reported 200k
    // is disproved by 356,380, so it falls through to the `opus[1m]` marker.
    contextLimit: 1_000_000,
    totalInputTokens: 30_328_545,
    totalOutputTokens: 120_429,
    costUSD: 22.58,
  }),
  statusLine: { contextUsedPct: 100, contextWindowSize: 200_000 } as SessionStatusLine,
};

describe('deriveSessionStats — the 23M worker (live specimen)', () => {
  it('keeps the honest cumulative figure, under a name that says it is the cost side', () => {
    const stats = deriveSessionStats(WORKER_23M);
    expect(stats.billedTokens).toBe(30_448_974);
    // The ambiguous `tokens` is gone: a caller must now choose which it means.
    expect('tokens' in stats).toBe(false);
  });

  it('reports occupancy separately, and it is bounded by the window', () => {
    const stats = deriveSessionStats(WORKER_23M);
    expect(stats.contextTokens).toBe(356_380);
    expect(stats.contextTokens!).toBeLessThan(stats.billedTokens!);
  });

  it('DISBELIEVES a statusLine percentage whose window the session has disproved', () => {
    // 356,380 tokens cannot fit a 200k window, so `contextUsedPct: 100` is a
    // reading of a window this session is not on. Before this guard the bar
    // pegged red at 100% on a worker sitting at 36%.
    const stats = deriveSessionStats(WORKER_23M);
    expect(Math.round(stats.ctxPct!)).toBe(36);
  });

  it('still trusts the statusLine percentage when its window holds up', () => {
    const stats = deriveSessionStats({
      usage: usage({ contextTokens: 90_000, contextLimit: 200_000 }),
      statusLine: { contextUsedPct: 45, contextWindowSize: 200_000 } as SessionStatusLine,
    });
    expect(stats.ctxPct).toBe(45);
  });

  it('never reconstructs occupancy from a disproved window (pct × wrong window)', () => {
    // The managed-provider path multiplies pct by the reported window. Fed a
    // disproved window that is how an absurd token count is manufactured, so it
    // must decline rather than produce one. Here there is no transcript count
    // to fall back on, so the honest answer is no number at all.
    const stats = deriveSessionStats({
      usage: usage({ contextTokens: 356_380, contextLimit: null }),
      statusLine: { contextUsedPct: 100, contextWindowSize: 200_000 } as SessionStatusLine,
    });
    expect(stats.contextTokens).toBe(356_380); // the direct count, never pct × 200k
    expect(stats.ctxPct).toBeUndefined(); // limit unknown ⇒ omit the meter
  });
});

describe('fmtTokens', () => {
  it('formats sub-million counts with a k suffix', () => {
    expect(fmtTokens(0)).toBe('0');
    expect(fmtTokens(999)).toBe('999');
    expect(fmtTokens(142_345)).toBe('142k');
    expect(fmtTokens(999_499)).toBe('999k');
  });

  it('switches to M rather than emitting a 4-digit "1000k" near the boundary', () => {
    // Math.round(999_999 / 1000) === 1000, which must not render as "1000k".
    expect(fmtTokens(999_500)).toBe('1.0M');
    expect(fmtTokens(999_999)).toBe('1.0M');
    expect(fmtTokens(1_000_000)).toBe('1.0M');
  });

  it('drops the decimal at/above 10M', () => {
    expect(fmtTokens(1_200_000)).toBe('1.2M');
    expect(fmtTokens(12_000_000)).toBe('12M');
  });
});

describe('isSnapshotStale', () => {
  const NOW = 1_000_000_000;
  const OLD = NOW - STALE_AFTER_MS - 1;
  const FRESH = NOW - 1_000;

  it('flags a working agent whose snapshot has gone quiet', () => {
    expect(isSnapshotStale('streaming', OLD, NOW)).toBe(true);
    expect(isSnapshotStale('thinking', OLD, NOW)).toBe(true);
  });

  it('does not flag a working agent with recent activity', () => {
    expect(isSnapshotStale('streaming', FRESH, NOW)).toBe(false);
  });

  it('never flags idle/waiting/stopped agents — silence is normal for them', () => {
    expect(isSnapshotStale('idle', OLD, NOW)).toBe(false);
    expect(isSnapshotStale('waiting_approval', OLD, NOW)).toBe(false);
    expect(isSnapshotStale(undefined, OLD, NOW)).toBe(false);
  });

  it('does not flag when lastActivity is unknown', () => {
    expect(isSnapshotStale('streaming', undefined, NOW)).toBe(false);
  });
});

describe('summarizeFileChanges', () => {
  const fc = (toolName: string, path: string, input: any = {}): FileChange => ({
    path,
    toolName,
    input,
    timestamp: 0,
  });

  it('counts unique files with estimated +/- from tool inputs', () => {
    const out = summarizeFileChanges([
      fc('Edit', '/r/a.ts', { file_path: '/r/a.ts', old_string: 'x\ny', new_string: 'z' }),
      fc('Edit', '/r/a.ts', { file_path: '/r/a.ts', old_string: 'q', new_string: 'r\ns' }),
      fc('Write', '/r/b.md', { file_path: '/r/b.md', content: 'one\ntwo' }),
    ]);
    expect(out).toEqual({ files: 2, added: 5, removed: 3 });
  });

  it('falls back to the change path when the input lacks one (codex apply_patch)', () => {
    const out = summarizeFileChanges([fc('apply_patch', 'src/main.rs')]);
    expect(out).toEqual({ files: 1, added: 0, removed: 0 });
  });

  it('is empty for no changes', () => {
    expect(summarizeFileChanges([])).toEqual({ files: 0, added: 0, removed: 0 });
  });
});

describe('window length formatting', () => {
  it('spells whole days, whole hours and loose minutes', () => {
    expect(fmtWindowShort(300)).toBe('5h');
    expect(fmtWindowShort(10_080)).toBe('7d');
    expect(fmtWindowShort(90)).toBe('90m');
    expect(fmtWindowLength(300)).toBe('5 hours');
    expect(fmtWindowLength(10_080)).toBe('7 days');
    expect(fmtWindowLength(1440)).toBe('1 day');
    expect(fmtWindowLength(90)).toBe('1 hour 30 min');
  });

  it('reports nothing for an unknown or nonsensical length', () => {
    expect(fmtWindowShort(undefined)).toBeUndefined();
    expect(fmtWindowLength(undefined)).toBeUndefined();
    expect(fmtWindowShort(0)).toBeUndefined();
  });
});

describe('usageWindows', () => {
  it('carries the window length into the label and the short name', () => {
    const ws = usageWindows({
      fiveHourPct: 11,
      fiveHourResetsAt: 1_787_593_199,
      fiveHourWindowMins: 300,
      sevenDayPct: 2,
      sevenDayWindowMins: 10_080,
    });
    expect(ws.map((w) => w.key)).toEqual(['fiveHour', 'sevenDay']);
    expect(ws[0]).toMatchObject({ short: '5h', label: '5-hour limit', windowMins: 300, pct: 11 });
    expect(ws[1]).toMatchObject({ short: '7d', label: '7-day limit', windowMins: 10_080 });
  });

  it('labels a window by the length the provider actually reported', () => {
    // A Codex primary window need not be five hours; the slot it lands in must
    // not put "5-hour" on a 90-minute window.
    const [w] = usageWindows({ fiveHourPct: 40, fiveHourWindowMins: 90 });
    expect(w.label).toBe('90-minute limit');
    expect(w.short).toBe('90m');
  });

  // ── degrade gracefully ──────────────────────────────────────────────────
  it('omits a window the provider does not report at all', () => {
    // Codex reports primary + secondary and never a monthly window. The monthly
    // slot must be absent, not a 0% meter.
    const ws = usageWindows({
      fiveHourPct: 19,
      fiveHourWindowMins: 300,
      sevenDayPct: 3,
      sevenDayWindowMins: 10_080,
    });
    expect(ws.map((w) => w.key)).toEqual(['fiveHour', 'sevenDay']);
    expect(ws.some((w) => w.key === 'monthly')).toBe(false);
  });

  it('keeps a window that reported only a reset time, with no percentage', () => {
    // Claude's stream sends resetsAt without utilization outside warning events.
    // The row is real; the meter is not, so pct stays undefined rather than 0.
    const ws = usageWindows({ fiveHourResetsAt: 1_787_593_199 });
    expect(ws).toHaveLength(1);
    expect(ws[0].pct).toBeUndefined();
    expect(ws[0].resetsAt).toBe(1_787_593_199);
  });

  it('falls back to the slot name when no length is reported', () => {
    const ws = usageWindows({ monthlyPct: 5 });
    expect(ws[0]).toMatchObject({ key: 'monthly', short: 'Mo', label: 'Monthly limit' });
    expect(ws[0].windowMins).toBeUndefined();
  });

  it('is empty when no window reported anything', () => {
    expect(usageWindows({})).toEqual([]);
  });
});

// ── cacheBreakdown ───────────────────────────────────────────────────────────
//
// The same degrade rule usageWindows established: a figure appears only when a
// provider actually reported it, and a share with a zero denominator is
// undefined rather than 0%.
describe('cacheBreakdown', () => {
  it("reads Claude's itemized transcript split, writes included", () => {
    const b = cacheBreakdown({
      usage: usage({ cache: { fresh: 2, write: 23_393, read: 17_589 } }),
    });
    expect(b).toMatchObject({ fresh: 2, write: 23_393, read: 17_589, total: 40_984 });
    expect(b!.hitRatePct).toBeCloseTo((17_589 / 40_984) * 100, 6);
  });

  it('returns null when the provider reported no cache data at all', () => {
    // Not a zeroed breakdown: "did not say" and "cached nothing" are different
    // claims, and only one of them is ours to make.
    expect(cacheBreakdown({ usage: usage({}) })).toBeNull();
    expect(cacheBreakdown({ statusLine: { totalInputTokens: 50_000 } })).toBeNull();
    expect(cacheBreakdown({})).toBeNull();
    expect(cacheBreakdown(null)).toBeNull();
  });

  it('derives fresh/read from a Codex status line, and omits writes entirely', () => {
    // Codex reports a cache-read subset of its input and nothing about writes.
    // `write` stays undefined so the dialog drops the row rather than claiming
    // the session wrote nothing to cache.
    const b = cacheBreakdown({
      statusLine: { totalInputTokens: 4_402_946, cachedInputTokens: 3_733_376 },
    });
    expect(b).toMatchObject({ fresh: 669_570, read: 3_733_376, total: 4_402_946 });
    expect(b!.write).toBeUndefined();
    expect(b!.hitRatePct).toBeCloseTo((3_733_376 / 4_402_946) * 100, 6);
  });

  it('leaves the hit rate undefined when the denominator is zero', () => {
    // A session that reported cache fields but has counted nothing yet has no
    // hit rate. 0% would read as a cache that never hit, which is a claim the
    // numbers do not support.
    const b = cacheBreakdown({ usage: usage({ cache: { fresh: 0, write: 0, read: 0 } }) });
    expect(b).toMatchObject({ fresh: 0, write: 0, read: 0, total: 0 });
    expect(b!.hitRatePct).toBeUndefined();
  });

  it('prefers the itemized split over the status line when both are present', () => {
    // Only the transcript split separates writes from reads, so it wins.
    const b = cacheBreakdown({
      usage: usage({ cache: { fresh: 10, write: 20, read: 30 } }),
      statusLine: { totalInputTokens: 999, cachedInputTokens: 111 },
    });
    expect(b).toMatchObject({ fresh: 10, write: 20, read: 30, total: 60 });
  });
});
