import { describe, it, expect } from 'vitest';
import * as fs from 'fs';
import * as path from 'path';
import {
  dayKey,
  emptyProviderState,
  emptySchedule,
  providerNeedsCheck,
  scheduleDue,
  windowActive,
  fiveHourWindowFromReport,
  type KeepWarmConfig,
  type UsageReportWire,
} from './keepWarmLogic';

const cfg = (over: Partial<KeepWarmConfig> = {}): KeepWarmConfig => ({
  enabled: true,
  providers: ['claude'],
  mode: 'auto',
  intervalHours: 5,
  dailyAt: '08:00',
  ...over,
});

// Local-time constructor so daily-mode tests are TZ-independent.
const at = (hh: number, mm: number) => new Date(2026, 6, 20, hh, mm, 0);

describe('scheduleDue', () => {
  it('auto: every tick opens a slot', () => {
    expect(scheduleDue(cfg(), emptySchedule(), at(3, 0))).toBe(true);
  });

  it('interval: first slot is immediate, then respects the cadence', () => {
    const c = cfg({ mode: 'interval', intervalHours: 2 });
    const state = emptySchedule();
    expect(scheduleDue(c, state, at(9, 0))).toBe(true);
    state.lastIntervalCheckMs = at(9, 0).getTime();
    expect(scheduleDue(c, state, at(10, 59))).toBe(false);
    expect(scheduleDue(c, state, at(11, 0))).toBe(true);
  });

  it('interval: a non-positive cadence falls back to 5h instead of spinning', () => {
    const c = cfg({ mode: 'interval', intervalHours: 0 });
    const state = { ...emptySchedule(), lastIntervalCheckMs: at(9, 0).getTime() };
    expect(scheduleDue(c, state, at(9, 1))).toBe(false);
    expect(scheduleDue(c, state, at(14, 0))).toBe(true);
  });

  it('daily: opens at/after the configured time, once per day', () => {
    const c = cfg({ mode: 'daily', dailyAt: '08:30' });
    const state = emptySchedule();
    expect(scheduleDue(c, state, at(8, 29))).toBe(false);
    expect(scheduleDue(c, state, at(8, 30))).toBe(true);
    state.lastDailyKey = dayKey(at(8, 30));
    expect(scheduleDue(c, state, at(9, 0))).toBe(false);
    // Next day it's due again.
    expect(scheduleDue(c, state, new Date(2026, 6, 21, 8, 30))).toBe(true);
  });

  it('daily: an unparseable time does nothing rather than guessing', () => {
    const c = cfg({ mode: 'daily', dailyAt: 'morning' });
    expect(scheduleDue(c, emptySchedule(), at(12, 0))).toBe(false);
  });
});

describe('providerNeedsCheck', () => {
  const nowMs = at(9, 0).getTime();

  it('a fresh provider needs a check', () => {
    expect(providerNeedsCheck(emptyProviderState(), nowMs)).toBe(true);
  });

  it('quiet while a window is assumed running, due again after it lapses', () => {
    const state = { ...emptyProviderState(), assumedResetsAtMs: nowMs + 60_000 };
    expect(providerNeedsCheck(state, nowMs)).toBe(false);
    state.assumedResetsAtMs = nowMs - 1;
    expect(providerNeedsCheck(state, nowMs)).toBe(true);
  });

  it('failure backoff suppresses checks until it expires', () => {
    const state = { ...emptyProviderState(), notBeforeMs: nowMs + 1 };
    expect(providerNeedsCheck(state, nowMs)).toBe(false);
    state.notBeforeMs = nowMs;
    expect(providerNeedsCheck(state, nowMs)).toBe(true);
  });
});

describe('windowActive', () => {
  const nowMs = at(9, 0).getTime();

  it('nonzero utilization counts even without resets_at (seen live)', () => {
    expect(windowActive({ five_hour_pct: 19, five_hour_resets_at: null }, nowMs)).toBe(true);
  });

  it('a future reset counts even at 0% utilization', () => {
    expect(
      windowActive({ five_hour_pct: 0, five_hour_resets_at: nowMs / 1000 + 3600 }, nowMs),
    ).toBe(true);
  });

  it('0% with a past/absent reset means no window', () => {
    expect(windowActive({ five_hour_pct: 0, five_hour_resets_at: null }, nowMs)).toBe(false);
    expect(
      windowActive({ five_hour_pct: null, five_hour_resets_at: nowMs / 1000 - 10 }, nowMs),
    ).toBe(false);
    expect(windowActive({}, nowMs)).toBe(false);
  });
});

describe('fiveHourWindowFromReport', () => {
  const nowMs = at(9, 0).getTime();
  const sec = (offsetMs: number) => Math.floor((nowMs + offsetMs) / 1000);

  const report = (provider: string, accounts: unknown[]): UsageReportWire =>
    ({
      generated_at: Math.floor(nowMs / 1000),
      providers: [{ provider, accounts }],
    }) as UsageReportWire;

  const acct = (five_hour: unknown) => ({ account: '', label: 'x', windows: { five_hour } });

  it('a future reset is a running window, and feeds windowActive', () => {
    const w = fiveHourWindowFromReport(
      report('codex', [
        acct({
          used_percent: { state: 'ok', value: 12 },
          resets_at: sec(3_600_000),
          is_current: true,
        }),
      ]),
      'codex',
      nowMs,
    );
    expect(w).toEqual({ five_hour_resets_at: sec(3_600_000) });
    expect(windowActive(w!, nowMs)).toBe(true);
  });

  // The trap this whole function exists to avoid. The live daemon serves codex
  // at 67% against a resets_at two days gone: the percentage is real history
  // and a false present, and windowActive() reads any pct > 0 as a live window.
  // Carrying it across would suppress warming permanently.
  it('a high percentage on a ROLLED-OVER window is not a running window', () => {
    const w = fiveHourWindowFromReport(
      report('codex', [
        acct({
          used_percent: { state: 'ok', value: 67 },
          resets_at: sec(-172_800_000),
          is_current: false,
        }),
      ]),
      'codex',
      nowMs,
    );
    expect(w).toEqual({});
    expect(windowActive(w!, nowMs)).toBe(false);
  });

  it('distinguishes "definitely lapsed" from "cannot tell"', () => {
    // No reset time at all: the report leaves is_current null, and so do we.
    expect(
      fiveHourWindowFromReport(
        report('codex', [acct({ used_percent: { state: 'ok', value: 67 }, resets_at: null })]),
        'codex',
        nowMs,
      ),
    ).toBeNull();
    // An unreadable window is unknown, not lapsed.
    expect(
      fiveHourWindowFromReport(
        report('codex', [
          acct({ used_percent: { state: 'unavailable', reason: 'no local quota record' } }),
        ]),
        'codex',
        nowMs,
      ),
    ).toBeNull();
  });

  it('takes the latest reset when several accounts report one', () => {
    expect(
      fiveHourWindowFromReport(
        report('codex', [
          acct({ resets_at: sec(-1000) }),
          acct({ resets_at: sec(600_000) }),
          acct({ resets_at: sec(3_600_000) }),
        ]),
        'codex',
        nowMs,
      ),
    ).toEqual({ five_hour_resets_at: sec(3_600_000) });
  });

  it('a provider the report does not carry is unknown, not lapsed', () => {
    expect(
      fiveHourWindowFromReport(report('claude', [acct({ resets_at: sec(-1) })]), 'codex', nowMs),
    ).toBeNull();
    expect(fiveHourWindowFromReport({}, 'codex', nowMs)).toBeNull();
    expect(fiveHourWindowFromReport(report('codex', []), 'codex', nowMs)).toBeNull();
  });
});

// ---------------------------------------------------------------------------
// THE CROSS-LANGUAGE HALF.
//
// contracts/usage-window-currency-cases.json is the WINDOW-CURRENCY RULE: a
// reading may be used only if resets_at is present and strictly after the
// moment of the decision, and the wire's own `is_current` is a display hint
// that is never the decision input. The other loader is
// services/hub/internal/limits/window_test.go, which holds the hub's routing
// reader to the same verdicts.
//
// The two implementations answer deliberately different questions, so only the
// currency verdict is shared. fiveHourWindowFromReport's three returns ARE the
// three verdicts:
//
//   {five_hour_resets_at}  current      — a window is running
//   {}                     rolled-over  — every readable reading has lapsed
//   null                   unreadable   — nothing readable; ask elsewhere
//
// and the percentage is dropped on all three, which is why usedPercent is
// asserted absent here rather than compared.
// ---------------------------------------------------------------------------

interface CurrencyCase {
  name: string;
  now: number;
  window: {
    used_percent?: { state: string; value?: number | null; reason?: string | null } | null;
    resets_at?: number | null;
    window_minutes?: number | null;
    is_current?: boolean | null;
  } | null;
  expect: 'current' | 'rolled-over' | 'unreadable';
  unknownBecause?: string;
  usedPercent: number | null;
  secondsToReset: number | null;
  why: string;
}

const currencyFixture: {
  cases: CurrencyCase[];
  vocabulary?: {
    rolledOverReasons?: Record<string, string>;
    unreadableReasons?: Record<string, string>;
  };
} = JSON.parse(
  fs.readFileSync(
    path.join(
      __dirname,
      '..',
      '..',
      '..',
      '..',
      '..',
      'contracts',
      'usage-window-currency-cases.json',
    ),
    'utf8',
  ),
);

/** The floor is what stops a fixture edit that drops half the cases from
 *  reporting a clean sweep over the two that survived. */
const CURRENCY_CASE_FLOOR = 11;

/** The per-verdict spread, TWINNED with the same three minima in
 *  services/hub/internal/limits/window_test.go.
 *
 *  The total floor alone is satisfied by eleven cases of one verdict, which
 *  exercises one arm of the rule and reports a clean sweep — the gap the P0
 *  review named. Both loaders now assert the distribution, so replacing the
 *  semantic spread fails on both sides of the language boundary. */
const CURRENCY_VERDICT_FLOOR: Record<CurrencyCase['expect'], number> = {
  current: 3,
  'rolled-over': 4,
  unreadable: 3,
};

/** The reason vocabulary, re-derived from the case's own inputs.
 *
 *  `fiveHourWindowFromReport` does not RETURN a reason — its three-way return is
 *  narrower than the hub's reading. But the rule that produces the reason is the
 *  shared half, so the TS side can still hold the fixture to it: derive what the
 *  reason must be from `now` and `window`, and require the case to say so. That
 *  is what makes `unknownBecause` load-bearing here rather than a field only Go
 *  reads. */
function expectedReason(c: CurrencyCase): string | undefined {
  if (c.window == null) return 'no-window-reading-at-all';
  const resets = c.window.resets_at;
  if (resets == null) return 'no-reset-time-reported';
  if (resets === c.now) return 'reset-time-equals-now';
  if (resets < c.now) return 'reset-time-has-passed';
  return undefined; // current — no reason
}

describe('usage-window currency (cross-language contract)', () => {
  it('the corpus loaded (a fixture that stopped loading would pass vacuously)', () => {
    expect(currencyFixture.cases.length).toBeGreaterThanOrEqual(CURRENCY_CASE_FLOOR);
  });

  const oneWindow = (c: CurrencyCase): UsageReportWire =>
    ({
      generated_at: c.now,
      providers: [
        {
          provider: 'codex',
          accounts: [{ account: '', label: 'x', windows: { five_hour: c.window } }],
        },
      ],
    }) as UsageReportWire;

  for (const c of currencyFixture.cases) {
    it(c.name, () => {
      const nowMs = c.now * 1000;
      const got = fiveHourWindowFromReport(oneWindow(c), 'codex', nowMs);

      switch (c.expect) {
        case 'current':
          expect(got).toEqual({ five_hour_resets_at: c.window?.resets_at });
          expect(windowActive(got!, nowMs)).toBe(true);
          break;
        case 'rolled-over':
          // A DEFINITE "no window is running" — strictly more than "unknown",
          // and what lets keep-warm ping without claiming it was in the dark.
          expect(got).toEqual({});
          expect(windowActive(got!, nowMs)).toBe(false);
          break;
        case 'unreadable':
          expect(got).toBeNull();
          break;
      }

      // The percentage never crosses, on ANY verdict. windowActive() reads any
      // pct above zero as a live window, so a stale 67% forwarded here would
      // suppress codex warming permanently — which is why this reader answers
      // a narrower question than the hub's.
      if (got != null) expect('five_hour_pct' in got).toBe(false);

      // ── The fields this loader used to ignore ────────────────────────────
      //
      // Until this block, TS asserted only the three-way verdict and "no
      // percentage forwarded", so `usedPercent`, `secondsToReset` and
      // `unknownBecause` could be mutated in the shared fixture and fail Go
      // ALONE — which made "breaking any case fails both loaders" stronger than
      // what was actually checked. They are held here against the case's own
      // inputs, which is a claim TS can make without needing the hub's wider
      // return shape.

      // unknownBecause: derived from `now` and `window` by the shared rule.
      expect(c.unknownBecause ?? undefined, `${c.name}: unknownBecause`).toBe(expectedReason(c));

      if (c.expect === 'current') {
        // secondsToReset is the distance the CALLER's clock is from the reset,
        // and it is exactly what got.five_hour_resets_at lets a reader compute.
        expect(c.secondsToReset, `${c.name}: secondsToReset`).toBe(c.window!.resets_at! - c.now);
        expect(got!.five_hour_resets_at! - c.now).toBe(c.secondsToReset);
        expect(c.secondsToReset!).toBeGreaterThan(0);

        // usedPercent is pinned iff the source could read one. TS still refuses
        // to forward it — that is the assertion above — but the fixture's claim
        // about WHETHER there is one is checkable here.
        const measured = c.window!.used_percent;
        if (measured?.state === 'ok' && typeof measured.value === 'number') {
          expect(c.usedPercent, `${c.name}: usedPercent`).toBe(measured.value);
        } else {
          expect(c.usedPercent, `${c.name}: an unreadable scalar has no percentage`).toBeNull();
        }
      } else {
        // 'UNKNOWN' has to mean NO NUMBER AT ALL. A reader that answered 0% and
        // no reset would satisfy "it refused" while handing a router a figure.
        expect(c.usedPercent, `${c.name}: usedPercent on a non-current case`).toBeNull();
        expect(c.secondsToReset, `${c.name}: secondsToReset on a non-current case`).toBeNull();
      }
    });
  }

  it('carries every verdict, not eleven of one', () => {
    const counted: Record<string, number> = {};
    for (const c of currencyFixture.cases) counted[c.expect] = (counted[c.expect] ?? 0) + 1;
    for (const [verdict, min] of Object.entries(CURRENCY_VERDICT_FLOOR)) {
      expect(
        counted[verdict] ?? 0,
        `the corpus carries ${counted[verdict] ?? 0} ${verdict} case(s); the total floor alone is satisfied by eleven cases of one verdict`,
      ).toBeGreaterThanOrEqual(min);
    }
  });

  it('every unknownBecause is in the fixture’s own declared vocabulary', () => {
    // Closure the other way: a reason the cases use that the vocabulary block
    // does not declare is a case pinned against a word no reader has heard of.
    const declared = new Set([
      ...Object.keys(currencyFixture.vocabulary?.rolledOverReasons ?? {}),
      ...Object.keys(currencyFixture.vocabulary?.unreadableReasons ?? {}),
    ]);
    expect(declared.size, 'the vocabulary blocks were renamed').toBeGreaterThan(0);
    for (const c of currencyFixture.cases) {
      if (!c.unknownBecause) continue;
      expect(declared, `${c.name} names an undeclared reason`).toContain(c.unknownBecause);
    }
  });

  it('the wire’s is_current is never the decision input', () => {
    // A reader that consulted the hint would pass every case above, because the
    // daemon computed it correctly at generated_at. Flipping it on every case
    // and requiring the verdicts not to move is what makes "display-only"
    // checkable rather than a comment.
    let flipped = 0;
    for (const c of currencyFixture.cases) {
      if (c.window == null) continue;
      const nowMs = c.now * 1000;
      const before = fiveHourWindowFromReport(oneWindow(c), 'codex', nowMs);
      const mutant: CurrencyCase = {
        ...c,
        window: {
          ...c.window,
          is_current: c.window.is_current == null ? true : !c.window.is_current,
        },
      };
      flipped += 1;
      expect(fiveHourWindowFromReport(oneWindow(mutant), 'codex', nowMs)).toEqual(before);
    }
    expect(flipped).toBeGreaterThanOrEqual(CURRENCY_CASE_FLOOR - 1);
  });
});
