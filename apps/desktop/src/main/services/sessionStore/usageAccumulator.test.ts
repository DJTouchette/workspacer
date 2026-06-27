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
