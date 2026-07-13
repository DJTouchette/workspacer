import { describe, it, expect, vi } from 'vitest';

// listClaudeModels reads two singletons; stub both so the test exercises only
// the alias/tier shape.
vi.mock('./configService', () => ({
  configService: { getConfig: () => ({}) },
}));
vi.mock('./claudeSessionStore', () => ({
  claudeSessionStore: { getAllSnapshots: () => [] },
}));

import { listClaudeModels } from './claudeModels';

describe('listClaudeModels — picker tiers', () => {
  it('Fable is a single 1M entry (no [1m] split, never 200K)', () => {
    const fable = listClaudeModels().aliases.filter((a) => a.value.startsWith('fable'));
    expect(fable).toHaveLength(1);
    expect(fable[0].value).toBe('fable');
    expect(fable[0].context).toBe('1M');
  });

  it('Opus and Sonnet each expose a standard (200K) and a [1m] (1M) tier', () => {
    const { aliases } = listClaudeModels();
    for (const fam of ['opus', 'sonnet']) {
      expect(aliases.find((a) => a.value === fam)?.context).toBe('200K');
      expect(aliases.find((a) => a.value === `${fam}[1m]`)?.context).toBe('1M');
    }
  });

  it('Haiku is 200K only', () => {
    const haiku = listClaudeModels().aliases.filter((a) => a.value.startsWith('haiku'));
    expect(haiku).toHaveLength(1);
    expect(haiku[0].context).toBe('200K');
  });
});
