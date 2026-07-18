import { describe, it, expect, vi, beforeEach } from 'vitest';

// configService is pulled in transitively by rememberModel(); stub it so the
// accumulator never touches disk or real config.
vi.mock('../configService', () => ({
  configService: {
    getConfig: vi.fn(() => ({ claude: { seenModels: [] } })),
    saveConfig: vi.fn(),
  },
}));

import { SessionUsageAccumulator } from './usageAccumulator';
import { configService } from '../configService';
import type { ClaudeSessionState } from '../claudeSessionStore';

function mkSession(): ClaudeSessionState {
  // Only the fields applyUsage touches matter here.
  return { sessionId: 's1', peakContext: 0, usage: null } as unknown as ClaudeSessionState;
}

describe('SessionUsageAccumulator.applyUsage — context limit', () => {
  let acc: SessionUsageAccumulator;
  beforeEach(() => {
    acc = new SessionUsageAccumulator();
  });

  it('promotes the context limit to 1M once a turn exceeds 200k', () => {
    const s = mkSession();
    acc.applyUsage(s, 'claude-sonnet-4-5', { input_tokens: 300_000 }, 'm1');
    expect(s.usage!.contextLimit).toBe(1_000_000);
  });

  it('keeps the 1M limit sticky on a later smaller turn (session stays in 1M mode)', () => {
    const s = mkSession();
    acc.applyUsage(s, 'claude-sonnet-4-5', { input_tokens: 300_000 }, 'm1');
    expect(s.usage!.contextLimit).toBe(1_000_000);
    // A subsequent smaller turn must NOT revert the window to 200k — the
    // session is still running in 1M mode (peakContext remembers the high mark).
    acc.applyUsage(s, 'claude-sonnet-4-5', { input_tokens: 50_000 }, 'm2');
    expect(s.peakContext).toBe(300_000);
    expect(s.usage!.contextLimit).toBe(1_000_000);
  });

  it('leaves the limit at 200k for a session that never exceeds the standard window', () => {
    const s = mkSession();
    acc.applyUsage(s, 'claude-sonnet-4-5', { input_tokens: 50_000 }, 'm1');
    expect(s.usage!.contextLimit).toBe(200_000);
  });
});

describe('SessionUsageAccumulator.applyUsage — replay must not double-count', () => {
  let acc: SessionUsageAccumulator;
  beforeEach(() => {
    acc = new SessionUsageAccumulator();
  });

  it('is idempotent when the same messages are replayed (conversation resync/reset)', () => {
    const s = mkSession();
    acc.applyUsage(s, 'claude-sonnet-4-5', { input_tokens: 200, output_tokens: 30 }, 'm1');
    acc.applyUsage(s, 'claude-sonnet-4-5', { input_tokens: 100, output_tokens: 10 }, 'm2');
    expect(s.usage!.totalInputTokens).toBe(300);
    expect(s.usage!.totalOutputTokens).toBe(40);
    const costAfterFirstPass = s.usage!.costUSD;
    expect(costAfterFirstPass).toBeGreaterThan(0);

    // A resync rebuilds the conversation by replaying the SAME usage items.
    // Totals and cost must stay put rather than doubling.
    acc.applyUsage(s, 'claude-sonnet-4-5', { input_tokens: 200, output_tokens: 30 }, 'm1');
    acc.applyUsage(s, 'claude-sonnet-4-5', { input_tokens: 100, output_tokens: 10 }, 'm2');
    expect(s.usage!.totalInputTokens).toBe(300);
    expect(s.usage!.totalOutputTokens).toBe(40);
    expect(s.usage!.costUSD).toBeCloseTo(costAfterFirstPass, 10);
  });

  it('still dedups non-consecutive repeats interleaved with other messages', () => {
    const s = mkSession();
    acc.applyUsage(s, 'claude-sonnet-4-5', { input_tokens: 100, output_tokens: 10 }, 'm1');
    acc.applyUsage(s, 'claude-sonnet-4-5', { input_tokens: 100, output_tokens: 10 }, 'm2');
    acc.applyUsage(s, 'claude-sonnet-4-5', { input_tokens: 100, output_tokens: 10 }, 'm1'); // repeat of m1
    expect(s.usage!.totalInputTokens).toBe(200);
    expect(s.usage!.totalOutputTokens).toBe(20);
  });

  it('forget() lets a fresh session recount from zero', () => {
    const s = mkSession();
    acc.applyUsage(s, 'claude-sonnet-4-5', { input_tokens: 100, output_tokens: 10 }, 'm1');
    acc.forget(s.sessionId);
    s.usage = null;
    acc.applyUsage(s, 'claude-sonnet-4-5', { input_tokens: 100, output_tokens: 10 }, 'm1');
    expect(s.usage!.totalInputTokens).toBe(100);
  });
});

describe('SessionUsageAccumulator.applyUsage — subagent (sidechain) turns', () => {
  let acc: SessionUsageAccumulator;
  beforeEach(() => {
    acc = new SessionUsageAccumulator();
  });

  it('counts sidechain tokens/cost into the totals at the subagent model rates', () => {
    const s = mkSession();
    acc.applyUsage(s, 'claude-fable-5', { input_tokens: 1_000, output_tokens: 500 }, 'm1');
    const mainCost = s.usage!.costUSD;
    // haiku subagent: 1M in + 1M out at $1/$5 = $6
    acc.applyUsage(
      s,
      'claude-haiku-4-5',
      { input_tokens: 1_000_000, output_tokens: 1_000_000 },
      'sub1',
      true,
    );
    expect(s.usage!.totalInputTokens).toBe(1_001_000);
    expect(s.usage!.totalOutputTokens).toBe(1_000_500);
    expect(s.usage!.costUSD).toBeCloseTo(mainCost + 6, 9);
  });

  it('never moves the context gauge, peak, or reported model', () => {
    const s = mkSession();
    acc.applyUsage(s, 'claude-fable-5', { input_tokens: 1_000 }, 'm1');
    acc.applyUsage(s, 'claude-haiku-4-5', { input_tokens: 500_000 }, 'sub1', true);
    expect(s.usage!.model).toBe('claude-fable-5');
    expect(s.usage!.contextTokens).toBe(1_000);
    expect(s.peakContext).toBe(1_000);
    // Fable is 1M-native.
    expect(s.usage!.contextLimit).toBe(1_000_000);
  });

  it('splits tokens/cost per model across main and sidechain turns', () => {
    const s = mkSession();
    acc.applyUsage(s, 'claude-fable-5', { input_tokens: 1_000, output_tokens: 500 }, 'm1');
    acc.applyUsage(s, 'claude-haiku-4-5', { input_tokens: 200, output_tokens: 100 }, 'sub1', true);
    acc.applyUsage(s, 'claude-haiku-4-5', { input_tokens: 300, output_tokens: 50 }, 'sub2', true);
    const models = s.usage!.models;
    expect(models['claude-fable-5']).toMatchObject({ inputTokens: 1_000, outputTokens: 500 });
    expect(models['claude-haiku-4-5']).toMatchObject({ inputTokens: 500, outputTokens: 150 });
    // fable: (1000*10 + 500*50)/1e6 ; haiku: (500*1 + 150*5)/1e6
    expect(models['claude-fable-5'].costUSD).toBeCloseTo(0.035, 9);
    expect(models['claude-haiku-4-5'].costUSD).toBeCloseTo(0.00125, 9);
  });

  it('dedups replayed sidechain usage like main-thread usage', () => {
    const s = mkSession();
    acc.applyUsage(s, 'claude-haiku-4-5', { input_tokens: 100, output_tokens: 10 }, 'sub1', true);
    acc.applyUsage(s, 'claude-haiku-4-5', { input_tokens: 100, output_tokens: 10 }, 'sub1', true);
    expect(s.usage!.totalInputTokens).toBe(100);
    expect(s.usage!.models['claude-haiku-4-5'].inputTokens).toBe(100);
  });
});

describe('SessionUsageAccumulator.rememberModel — external seenModels not clobbered', () => {
  beforeEach(() => {
    vi.mocked(configService.saveConfig).mockClear();
  });

  it('preserves models another writer added to seenModels after launch', () => {
    const acc = new SessionUsageAccumulator();

    // Desktop launches; on-disk seenModels = ['sonnet']. rememberModel seeds its
    // cache from this on first use. 'sonnet' is already known → no save.
    vi.mocked(configService.getConfig).mockReturnValue({
      claude: { seenModels: ['sonnet'] },
    } as any);
    acc.applyUsage(mkSession(), 'sonnet', { input_tokens: 100 }, 'm1');
    expect(vi.mocked(configService.saveConfig)).not.toHaveBeenCalled();

    // A web/brain/mobile client records 'opus' → config.yaml now holds BOTH.
    // (The mtime gate in configService never invalidates the desktop's cache.)
    vi.mocked(configService.getConfig).mockReturnValue({
      claude: { seenModels: ['opus', 'sonnet'] },
    } as any);

    // Desktop then observes a first-ever 'haiku' turn and persists.
    acc.applyUsage(mkSession(), 'haiku', { input_tokens: 100 }, 'm2');

    const calls = vi.mocked(configService.saveConfig).mock.calls;
    expect(calls.length).toBeGreaterThan(0);
    const saved = (calls[calls.length - 1][0] as any).claude.seenModels as string[];

    // deepMerge replaces the seenModels array wholesale, so the persisted array
    // must already contain every model — including the externally-added 'opus'.
    expect(saved).toContain('haiku');
    expect(saved).toContain('sonnet');
    expect(saved).toContain('opus'); // FAILS on current code ('opus' dropped)
  });
});
