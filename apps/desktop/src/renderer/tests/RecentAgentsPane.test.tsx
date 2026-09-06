import { describe, expect, it } from 'vitest';
import { total } from '../src/panes/RecentAgentsPane';
import { recentAgentsFixture } from '../src/harness/recentAgentsFixture';
describe('history metric coverage', () => {
  it('counts explicit zero as reported and never counts a duplicate dispatch twice', () => {
    const attempt = recentAgentsFixture()[0].attempts[0];
    const one = { ...attempt, metrics: { costUSD: 0 } };
    const unknown = { ...attempt, dispatchId: 'unknown', metrics: {} };
    expect(total([one, one, unknown], 'costUSD')).toBe('$0.0000 · partial 1/2 reported');
    expect(total([unknown], 'inputTokens')).toBe('Not reported (0/1)');
  });
  it('counts each real retry attempt once within its task', () => {
    const task = recentAgentsFixture()[1];
    expect(total(task.attempts, 'inputTokens')).toContain('partial 9/18 reported');
    expect(total([...task.attempts, ...task.attempts], 'inputTokens')).toBe(
      total(task.attempts, 'inputTokens'),
    );
  });
});
