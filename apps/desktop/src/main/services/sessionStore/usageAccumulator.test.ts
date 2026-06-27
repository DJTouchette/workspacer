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
