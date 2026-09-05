import { describe, it, expect } from 'vitest';
import {
  usageAccountIdentity,
  usagePacingRows,
  usageReportAttribution,
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
  const withAccounts = (
    accounts: UsageReportAccount[],
    provider = 'claude',
  ): UsageReportWire => ({
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
