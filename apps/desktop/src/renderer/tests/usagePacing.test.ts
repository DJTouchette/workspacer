import { describe, it, expect } from 'vitest';
import {
  usageAccountIdentity,
  usagePacingRows,
  usageReportAttribution,
  usageResetPhrase,
  usageStaleNote,
} from '../src/lib/usagePacing';
import type { UsageReportAccount, UsageReportWire } from '../../main/shared/usageReport';
const now = 1_800_000_000;
const report: UsageReportWire = { evaluated_at: now, valid_until: now + 60 };
function account(used = 50, expected = 52): UsageReportAccount {
  return {
    account: '/a/work',
    source: 'disk',
    observed_at: now - 300,
    fresh: null,
    windows: {
      five_hour: {
        used_percent: { state: 'ok', value: used },
        resets_at: now + 9000,
        window_minutes: 300,
        pace: {
          known: true,
          state: 'ahead',
          usedPct: used,
          expectedPct: expected,
          curve: 'calendar',
        },
      },
    },
  };
}
const rows = (a = account(), r = report, at = now) => usagePacingRows(r, a, at * 1000).rows;
describe('whole usage observations', () => {
  it.each([
    [0, 'under'],
    [49.999, 'under'],
    [50, 'on pace'],
    [52, 'on pace'],
    [54, 'on pace'],
    [54.001, 'ahead'],
  ])('compares %s before rounding', (pct, verdict) => {
    expect(rows(account(pct as number))[0].verdict).toBe(verdict);
  });
  it('preserves zero and the actual expected offset', () => {
    expect(rows(account(0))[0]).toMatchObject({ pct: 0, expected: 52 });
    expect(usagePacingRows(report, account(0), now * 1000).fields.fiveHourPct).toBe(0);
  });
  it.each([-1, 0, 1])('rechecks reset at now %+i', (offset) => {
    const a = account();
    a.windows!.five_hour!.resets_at = now + offset;
    expect(rows(a).length).toBe(offset > 0 ? 1 : 0);
  });
  it.each([undefined, null, 0, -1, 527041, NaN, Infinity])(
    'refuses malformed length %s',
    (length) => {
      const a = account();
      a.windows!.five_hour!.window_minutes = length;
      expect(rows(a)[0]).toMatchObject({ pct: 50, expected: undefined, verdict: undefined });
    },
  );
  it.each([undefined, null, NaN, Infinity, -1, 101])('does not invent percent %s', (value) => {
    const a = account();
    a.windows!.five_hour!.used_percent!.value = value;
    expect(rows(a)[0].pct).toBeUndefined();
    expect(rows(a)[0].verdict).toBeUndefined();
  });
  it('requires matching measured usage, known pace, valid sample and provider freshness', () => {
    for (const change of [
      (a: UsageReportAccount) => {
        a.fresh = false;
      },
      (a: UsageReportAccount) => {
        a.windows!.five_hour!.pace!.known = false;
      },
      (a: UsageReportAccount) => {
        a.windows!.five_hour!.pace!.state = 'disabled';
      },
      (a: UsageReportAccount) => {
        a.windows!.five_hour!.pace!.usedPct = 67;
      },
      (a: UsageReportAccount) => {
        a.windows!.five_hour!.resets_at = null;
      },
    ]) {
      const a = account();
      change(a);
      expect(rows(a)[0].verdict).toBeUndefined();
    }
    expect(rows(account(), report, now + 60)[0].expected).toBeUndefined();
    expect(rows(account(), { ...report, transport_stale: true })[0].expected).toBeUndefined();
    expect(rows(account(), { ...report, evaluated_at: now + 1 })[0].expected).toBeUndefined();
    expect(rows(account(), {}, now)[0].expected).toBeUndefined();
  });
  it('does not resurrect missing, removed, or unavailable windows', () => {
    const a = account();
    a.windows!.five_hour!.used_percent = { state: 'unavailable' };
    expect(rows(a)).toEqual([]);
    a.windows = { seven_day: { used_percent: { state: 'ok', value: 0 }, resets_at: now + 1000 } };
    expect(rows(a).map((r) => r.key)).toEqual(['seven_day']);
  });
  /**
   * WHAT IS LEFT is the number a reader actually wants, and it is the one that
   * can be invented. `100 - used` is only meaningful when `used` is a real
   * measurement, so an absent, malformed or unavailable reading has to leave
   * the remainder absent too: "100% left" over an account nobody could read is
   * a lie with no tell.
   */
  it('derives the remainder only from a valid measurement', () => {
    expect(rows(account(70))[0]).toMatchObject({ usedPct: 70, left: 30 });
    // Rounded once, so the pair always sums to 100 rather than to 101.
    expect(rows(account(70.5))[0]).toMatchObject({ usedPct: 71, left: 29 });
    expect(rows(account(0))[0]).toMatchObject({ usedPct: 0, left: 100 });
    for (const value of [undefined, null, NaN, -1, 101]) {
      const a = account();
      a.windows!.five_hour!.used_percent!.value = value;
      expect(rows(a)[0]).toMatchObject({ usedPct: undefined, left: undefined });
    }
    // …including a window that is only a reset time, with no measurement at all.
    const resetOnly = account();
    resetOnly.windows!.five_hour!.used_percent = undefined;
    expect(rows(resetOnly)[0].left).toBeUndefined();
  });

  // Near resets read as a countdown; anything past a day reads as a wall clock,
  // because "3d" is not a figure to plan a weekly allowance against.
  it('speaks a near reset as a countdown and a far one as a local time', () => {
    const at = now * 1000;
    expect(usageResetPhrase(now + 9000, at)).toBe('resets in 3h');
    expect(usageResetPhrase(now + 40 * 60, at)).toBe('resets in 40m');
    const far = usageResetPhrase(now + 3 * 86400, at);
    expect(far).toMatch(/^resets /);
    expect(far).not.toMatch(/^resets in /);
    // A reset already past gets no phrase; the window is dropped anyway, and a
    // leftover "resets in 0m" would contradict that.
    expect(usageResetPhrase(now - 1, at)).toBeUndefined();
    expect(usageResetPhrase(undefined, at)).toBeUndefined();
    expect(rows()[0].resetPhrase).toBe('resets in 3h');
  });

  /** A kept percentage from an unconfirmed observation is history, and the row
   *  has to be able to say which of the two reasons produced it. */
  it('marks a reading historical, naming the transport and the provider apart', () => {
    expect(rows()[0].stale).toBe(false);
    expect(usageStaleNote(report, account())).toBeUndefined();

    const aged = account();
    aged.fresh = false;
    expect(rows(aged)[0]).toMatchObject({ stale: true, pct: 50, expected: undefined });
    expect(usageStaleNote(report, aged)).toMatch(/provider last observed/);

    const offline = { ...report, transport_stale: true };
    expect(rows(account(), offline)[0]).toMatchObject({ stale: true, expected: undefined });
    expect(usageStaleNote(offline, account())).toMatch(/refresh failed/);
  });

  it('keeps canonical, default, unattributed and Windows identities distinct', () => {
    const keys = ['/a/work', '/b/work', '', null, 'C:\\accounts\\work'];
    expect(new Set(keys.map((account) => usageAccountIdentity({ account }))).size).toBe(
      keys.length,
    );
  });
});

/**
 * WHOSE allowance is this? The Overview never has to answer — it draws every
 * account the report knows. A session surface does, and picking the wrong row
 * puts another login's 91% on this session's card with nothing to give it away.
 */
describe('attributing a report row to one session', () => {
  const acct = (path: string | null, label?: string): UsageReportAccount => ({
    account: path,
    label,
    windows: { five_hour: { used_percent: { state: 'ok', value: 1 }, resets_at: now + 900 } },
  });
  const withAccounts = (accounts: UsageReportAccount[], provider = 'claude'): UsageReportWire => ({
    ...report,
    providers: [{ provider, accounts }],
  });
  const DEFAULT = acct('', 'default');
  const WORK = acct('/home/u/.claude/accounts/work', 'work');

  it('picks the row named by the session’s own config root', () => {
    const r = withAccounts([DEFAULT, WORK]);
    expect(
      usageReportAttribution(r, {
        provider: 'claude',
        transcriptPath: '/home/u/.claude/accounts/work/projects/p/t.jsonl',
      }),
    ).toMatchObject({ state: 'match', account: WORK });
    expect(
      usageReportAttribution(r, {
        provider: 'claude',
        transcriptPath: '/home/u/.claude/projects/p/t.jsonl',
      }),
    ).toMatchObject({ state: 'match', account: DEFAULT });
  });

  // Two logins, and nothing in the session says which. The report HAS the
  // numbers, which is exactly why the temptation to show one of them exists.
  it('refuses to guess between two accounts of one provider', () => {
    expect(usageReportAttribution(withAccounts([DEFAULT, WORK]), { provider: 'claude' })).toEqual({
      state: 'ambiguous',
      provider: 'claude',
      count: 2,
    });
    // Two config roots whose basenames collide resolve to the same key, so the
    // session's own path does not separate them either.
    expect(
      usageReportAttribution(withAccounts([acct('/a/work'), acct('/b/work')]), {
        provider: 'claude',
        transcriptPath: '/a/work/projects/p/t.jsonl',
      }),
    ).toMatchObject({ state: 'ambiguous', count: 2 });
  });

  // A provider with no per-session account marker: ONE row is an
  // identification, more than one is a guess.
  it('identifies a single-account provider and no more', () => {
    const one = withAccounts([acct('codex-account')], 'codex');
    expect(usageReportAttribution(one, { provider: 'codex' })).toMatchObject({ state: 'match' });
    expect(
      usageReportAttribution(withAccounts([acct('a'), acct('b')], 'codex'), { provider: 'codex' }),
    ).toMatchObject({ state: 'ambiguous', count: 2 });
    // …and never another provider's row.
    expect(usageReportAttribution(one, { provider: 'claude' })).toEqual({
      state: 'none',
      provider: 'claude',
    });
  });

  // The report's unattributed bucket is the daemon saying it could not name
  // those sessions. It is not a fallback for a session that CAN name itself.
  it('never folds a named session into the unattributed bucket', () => {
    const r = withAccounts([acct(null), WORK]);
    expect(
      usageReportAttribution(r, {
        provider: 'claude',
        transcriptPath: '/home/u/.claude/projects/p/t.jsonl',
      }),
    ).toEqual({ state: 'none', provider: 'claude' });
  });

  // Federation blanks transcriptPath, which collapses to the DEFAULT account
  // key — so without the hub check a peer's session would silently borrow the
  // local default login's numbers.
  it('withholds a local reading from a session on a peer hub', () => {
    expect(
      usageReportAttribution(withAccounts([DEFAULT]), {
        provider: 'claude',
        transcriptPath: '',
        hub: 'studio',
      }),
    ).toEqual({ state: 'remote', provider: 'claude', hub: 'studio' });
  });

  it('reports no report at all as unavailable, defaulting the provider', () => {
    expect(usageReportAttribution(null, { transcriptPath: '/x' })).toEqual({
      state: 'unavailable',
      provider: 'claude',
    });
    expect(usageReportAttribution(undefined, null)).toEqual({
      state: 'unavailable',
      provider: 'claude',
    });
    expect(usageReportAttribution({ ...report, providers: [] }, { provider: 'codex' })).toEqual({
      state: 'none',
      provider: 'codex',
    });
  });
});
