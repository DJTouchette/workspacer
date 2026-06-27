import { describe, it, expect } from 'vitest';
import { deriveSessionStats } from './sessionStats';
import type { SessionStatusLine, SessionUsage } from '../types/claudeSession';

const usage = (over: Partial<SessionUsage> = {}): SessionUsage =>
  ({
    model: 'claude-sonnet-4-6',
    contextTokens: 0,
    contextLimit: 200_000,
    totalInputTokens: 111,
    totalOutputTokens: 222,
    costUSD: 1,
    ...over,
  }) as SessionUsage;

describe('deriveSessionStats — cumulative tokens', () => {
  it('uses the statusLine when only totalOutputTokens is present', () => {
    const sl = { totalOutputTokens: 500 } as SessionStatusLine;
    // statusLine is authoritative; even with only output tokens it should win
    // over the transcript-derived usage fallback (which would give 111+222=333).
    expect(deriveSessionStats({ statusLine: sl, usage: usage() }).tokens).toBe(500);
  });

  it('uses the statusLine when only totalInputTokens is present', () => {
    const sl = { totalInputTokens: 400 } as SessionStatusLine;
    expect(deriveSessionStats({ statusLine: sl, usage: usage() }).tokens).toBe(400);
  });

  it('sums statusLine input+output when both present', () => {
    const sl = { totalInputTokens: 400, totalOutputTokens: 500 } as SessionStatusLine;
    expect(deriveSessionStats({ statusLine: sl, usage: usage() }).tokens).toBe(900);
  });

  it('falls back to usage when statusLine carries no token counts', () => {
    expect(deriveSessionStats({ statusLine: {} as SessionStatusLine, usage: usage() }).tokens).toBe(333);
  });
});
