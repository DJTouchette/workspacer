import { describe, it, expect } from 'vitest';
import { deriveSessionStats, isSnapshotStale, summarizeFileChanges, STALE_AFTER_MS } from './sessionStats';
import type { FileChange, SessionStatusLine, SessionUsage } from '../types/claudeSession';

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

describe('isSnapshotStale', () => {
  const NOW = 1_000_000_000;
  const OLD = NOW - STALE_AFTER_MS - 1;
  const FRESH = NOW - 1_000;

  it('flags a working agent whose snapshot has gone quiet', () => {
    expect(isSnapshotStale('streaming', OLD, NOW)).toBe(true);
    expect(isSnapshotStale('thinking', OLD, NOW)).toBe(true);
  });

  it('does not flag a working agent with recent activity', () => {
    expect(isSnapshotStale('streaming', FRESH, NOW)).toBe(false);
  });

  it('never flags idle/waiting/stopped agents — silence is normal for them', () => {
    expect(isSnapshotStale('idle', OLD, NOW)).toBe(false);
    expect(isSnapshotStale('waiting_approval', OLD, NOW)).toBe(false);
    expect(isSnapshotStale(undefined, OLD, NOW)).toBe(false);
  });

  it('does not flag when lastActivity is unknown', () => {
    expect(isSnapshotStale('streaming', undefined, NOW)).toBe(false);
  });
});

describe('summarizeFileChanges', () => {
  const fc = (toolName: string, path: string, input: any = {}): FileChange => ({
    path,
    toolName,
    input,
    timestamp: 0,
  });

  it('counts unique files with estimated +/- from tool inputs', () => {
    const out = summarizeFileChanges([
      fc('Edit', '/r/a.ts', { file_path: '/r/a.ts', old_string: 'x\ny', new_string: 'z' }),
      fc('Edit', '/r/a.ts', { file_path: '/r/a.ts', old_string: 'q', new_string: 'r\ns' }),
      fc('Write', '/r/b.md', { file_path: '/r/b.md', content: 'one\ntwo' }),
    ]);
    expect(out).toEqual({ files: 2, added: 5, removed: 3 });
  });

  it('falls back to the change path when the input lacks one (codex apply_patch)', () => {
    const out = summarizeFileChanges([fc('apply_patch', 'src/main.rs')]);
    expect(out).toEqual({ files: 1, added: 0, removed: 0 });
  });

  it('is empty for no changes', () => {
    expect(summarizeFileChanges([])).toEqual({ files: 0, added: 0, removed: 0 });
  });
});
