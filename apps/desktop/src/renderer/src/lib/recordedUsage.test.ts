import { describe, it, expect } from 'vitest';
import { recordedUsageBySession } from './recordedUsage';
import { withRecordedUsage } from './sessionStats';
import type { RecentAgentSession } from '../../../main/shared/ipcTypes';

const sess = (over: Partial<RecentAgentSession> = {}): RecentAgentSession => ({
  sessionId: 's1',
  provider: 'claude',
  cwd: '/home/u/proj',
  mode: 'stopped',
  transport: 'pty',
  archived: false,
  updatedAt: 1,
  startedAt: 0,
  name: '',
  title: '',
  model: '',
  ...over,
});

describe('recordedUsageBySession — unknown, unavailable and zero are three things', () => {
  it('indexes the figures the history DB actually recorded', () => {
    const map = recordedUsageBySession([sess({ costUSD: 12.5, billedTokens: 4_000_000 })]);
    expect(map.s1).toEqual({ costUSD: 12.5, billedTokens: 4_000_000 });
  });

  it('OMITS a session with nothing recorded, rather than storing an empty record', () => {
    // The distinction that matters downstream: `map[id]` undefined means the
    // surface keeps its dash. An empty object would still be truthy and invite
    // callers to treat "we hold a record" as "we hold a number".
    const map = recordedUsageBySession([sess()]);
    expect(map.s1).toBeUndefined();
    expect(Object.keys(map)).toHaveLength(0);
  });

  it('keeps a half-known row half-known (tokens without a cost)', () => {
    const map = recordedUsageBySession([sess({ billedTokens: 900 })]);
    expect(map.s1).toEqual({ billedTokens: 900 });
    expect('costUSD' in map.s1).toBe(false);
  });

  it('never invents a zero for the missing half', () => {
    const map = recordedUsageBySession([sess({ costUSD: 3 })]);
    expect(map.s1.billedTokens).toBeUndefined();
    expect(map.s1.costUSD).toBe(3);
  });
});

describe('withRecordedUsage — live wins, absence stays absence', () => {
  it('fills cost and tokens when the live snapshot has neither (the cold start)', () => {
    const out = withRecordedUsage({}, { costUSD: 5, billedTokens: 100 });
    expect(out.costUSD).toBe(5);
    expect(out.billedTokens).toBe(100);
    expect(out.recorded).toBe(true);
  });

  it('never overwrites a live reading with a stale record', () => {
    const out = withRecordedUsage(
      { costUSD: 0.03, billedTokens: 7 },
      { costUSD: 5, billedTokens: 100 },
    );
    expect(out.costUSD).toBe(0.03);
    expect(out.billedTokens).toBe(7);
    // Nothing was filled, so nothing is flagged as recorded.
    expect(out.recorded).toBeUndefined();
  });

  it('a LIVE zero is a measurement and survives the merge', () => {
    const out = withRecordedUsage({ costUSD: 0 }, { costUSD: 5 });
    expect(out.costUSD).toBe(0);
  });

  it('flags a partial fill, and only fills the missing half', () => {
    const out = withRecordedUsage({ costUSD: 0.03 }, { costUSD: 5, billedTokens: 100 });
    expect(out.costUSD).toBe(0.03);
    expect(out.billedTokens).toBe(100);
    expect(out.recorded).toBe(true);
  });

  it('leaves the stats untouched — same object — when there is nothing to fill', () => {
    const stats = { costUSD: 1, billedTokens: 2 };
    expect(withRecordedUsage(stats, undefined)).toBe(stats);
    expect(withRecordedUsage(stats, { costUSD: 9 })).toBe(stats);
    // An empty record can't fill anything either.
    expect(withRecordedUsage({}, {})).toEqual({});
  });

  it('does not manufacture a figure when neither side has one', () => {
    const out = withRecordedUsage({}, { billedTokens: 100 });
    expect(out.costUSD).toBeUndefined();
    expect(out.billedTokens).toBe(100);
  });
});
