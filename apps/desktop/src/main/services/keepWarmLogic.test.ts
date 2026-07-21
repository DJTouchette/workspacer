import { describe, it, expect } from 'vitest';
import {
  dayKey,
  dueForCheck,
  emptyKeepWarmState,
  parsePingResetsAtMs,
  windowActive,
  type KeepWarmConfig,
} from './keepWarmLogic';

const cfg = (over: Partial<KeepWarmConfig> = {}): KeepWarmConfig => ({
  enabled: true,
  mode: 'auto',
  intervalHours: 5,
  dailyAt: '08:00',
  ...over,
});

// Local-time constructor so daily-mode tests are TZ-independent.
const at = (hh: number, mm: number) => new Date(2026, 6, 20, hh, mm, 0);

describe('dueForCheck', () => {
  it('auto: due whenever nothing is known to be running', () => {
    expect(dueForCheck(cfg(), emptyKeepWarmState(), at(3, 0))).toBe(true);
  });

  it('every mode goes quiet while a window is assumed running', () => {
    const now = at(9, 0);
    const state = { ...emptyKeepWarmState(), assumedResetsAtMs: now.getTime() + 60_000 };
    for (const mode of ['auto', 'interval', 'daily'] as const) {
      expect(dueForCheck(cfg({ mode }), state, now)).toBe(false);
    }
  });

  it('auto: due again once the assumed window has lapsed', () => {
    const now = at(9, 0);
    const state = { ...emptyKeepWarmState(), assumedResetsAtMs: now.getTime() - 1 };
    expect(dueForCheck(cfg(), state, now)).toBe(true);
  });

  it('failure backoff suppresses checks until it expires', () => {
    const now = at(9, 0);
    const state = { ...emptyKeepWarmState(), notBeforeMs: now.getTime() + 1 };
    expect(dueForCheck(cfg(), state, now)).toBe(false);
    state.notBeforeMs = now.getTime();
    expect(dueForCheck(cfg(), state, now)).toBe(true);
  });

  it('interval: first check is immediate, then respects the cadence', () => {
    const c = cfg({ mode: 'interval', intervalHours: 2 });
    const state = emptyKeepWarmState();
    expect(dueForCheck(c, state, at(9, 0))).toBe(true);
    state.lastIntervalCheckMs = at(9, 0).getTime();
    expect(dueForCheck(c, state, at(10, 59))).toBe(false);
    expect(dueForCheck(c, state, at(11, 0))).toBe(true);
  });

  it('interval: a non-positive cadence falls back to 5h instead of spinning', () => {
    const c = cfg({ mode: 'interval', intervalHours: 0 });
    const state = { ...emptyKeepWarmState(), lastIntervalCheckMs: at(9, 0).getTime() };
    expect(dueForCheck(c, state, at(9, 1))).toBe(false);
    expect(dueForCheck(c, state, at(14, 0))).toBe(true);
  });

  it('daily: fires at/after the configured time, once per day', () => {
    const c = cfg({ mode: 'daily', dailyAt: '08:30' });
    const state = emptyKeepWarmState();
    expect(dueForCheck(c, state, at(8, 29))).toBe(false);
    expect(dueForCheck(c, state, at(8, 30))).toBe(true);
    state.lastDailyKey = dayKey(at(8, 30));
    expect(dueForCheck(c, state, at(9, 0))).toBe(false);
    // Next day it's due again.
    expect(dueForCheck(c, state, new Date(2026, 6, 21, 8, 30))).toBe(true);
  });

  it('daily: an unparseable time does nothing rather than guessing', () => {
    const c = cfg({ mode: 'daily', dailyAt: 'morning' });
    expect(dueForCheck(c, emptyKeepWarmState(), at(12, 0))).toBe(false);
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

describe('parsePingResetsAtMs', () => {
  it('reads the five-hour rate_limit_event (camelCase wire shape)', () => {
    const stdout = [
      '{"type":"system","subtype":"init"}',
      '{"type":"rate_limit_event","rate_limit_info":{"rateLimitType":"five_hour","utilization":1.0,"resetsAt":1789000000,"status":"allowed"}}',
      '{"type":"result","subtype":"success"}',
    ].join('\n');
    expect(parsePingResetsAtMs(stdout)).toBe(1789000000 * 1000);
  });

  it('buckets an untyped event as five-hour but skips 7d/overage windows', () => {
    const sevenDay =
      '{"type":"rate_limit_event","rate_limit_info":{"rateLimitType":"seven_day","resetsAt":111}}';
    const overage =
      '{"type":"rate_limit_event","rate_limit_info":{"rateLimitType":"overage","resetsAt":222}}';
    const untyped = '{"type":"rate_limit_event","rate_limit_info":{"resetsAt":333}}';
    expect(parsePingResetsAtMs([sevenDay, overage, untyped].join('\n'))).toBe(333 * 1000);
    expect(parsePingResetsAtMs([sevenDay, overage].join('\n'))).toBeNull();
  });

  it('survives non-JSON noise and reports null when absent', () => {
    expect(parsePingResetsAtMs('warning: something\nok\n')).toBeNull();
    expect(parsePingResetsAtMs('')).toBeNull();
  });
});
