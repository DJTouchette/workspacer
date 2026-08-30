import { describe, it, expect } from 'vitest';
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
