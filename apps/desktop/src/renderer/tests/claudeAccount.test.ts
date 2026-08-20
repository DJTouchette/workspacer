/**
 * claudeAccountOf — the Claude LOGIN a session belongs to, read off its
 * transcript path. This is what keys the per-account usage cards: a wrong
 * grouping either merges two logins' gauges (the bug this shipped to fix) or
 * splits one login into phantom cards.
 */

import { describe, it, expect } from 'vitest';
import { claudeAccountOf } from '../src/lib/claudeAccount';

describe('claudeAccountOf', () => {
  it('maps the default config root to the default group', () => {
    expect(claudeAccountOf('/home/u/.claude/projects/-home-u-work/abc.jsonl')).toBe('');
  });

  it('maps an Add-Claude-Account root to its slug', () => {
    expect(claudeAccountOf('/home/u/.claude/accounts/work/projects/-home-u-work/abc.jsonl')).toBe(
      'work',
    );
  });

  it('handles Windows separators', () => {
    expect(claudeAccountOf('C:\\Users\\u\\.claude\\accounts\\work\\projects\\p\\a.jsonl')).toBe(
      'work',
    );
    expect(claudeAccountOf('C:\\Users\\u\\.claude\\projects\\p\\a.jsonl')).toBe('');
  });

  it('labels a hand-made config dir by its basename', () => {
    expect(claudeAccountOf('/home/u/.claude-work/projects/p/a.jsonl')).toBe('.claude-work');
  });

  it('uses the LAST projects component, so a root containing one stays intact', () => {
    expect(claudeAccountOf('/home/u/projects/.claude/projects/p/a.jsonl')).toBe('');
  });

  it('missing/blank paths (fresh spawns, remote sessions) fall to default', () => {
    expect(claudeAccountOf(undefined)).toBe('');
    expect(claudeAccountOf('')).toBe('');
    expect(claudeAccountOf('/tmp/not-a-transcript.jsonl')).toBe('');
  });
});
