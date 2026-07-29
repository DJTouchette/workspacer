/**
 * These bounds are load-bearing for main-process responsiveness, not just for
 * memory: every coalesced flush structured-clones the whole session to the
 * renderer, up to ~60 times a second while an agent streams. Anything that can
 * grow here becomes an unbounded per-frame cost on the thread that also
 * forwards PTY bytes, so a regression shows up as the whole app stuttering
 * rather than as anything resembling a memory bug.
 */
import { describe, expect, it } from 'vitest';
import {
  capInPlace,
  MAX_FILE_CHANGES,
  MAX_TOOL_RESPONSE_CHARS,
  truncateToolResponse,
} from './bounds';

describe('capInPlace', () => {
  it('keeps the most recent entries and mutates in place', () => {
    const arr = [1, 2, 3, 4, 5];
    capInPlace(arr, 3);
    expect(arr).toEqual([3, 4, 5]);
  });

  it('leaves a short array alone', () => {
    const arr = [1, 2];
    capInPlace(arr, 5);
    expect(arr).toEqual([1, 2]);
  });
});

describe('truncateToolResponse', () => {
  it('passes a response through untouched when it fits', () => {
    const small = 'x'.repeat(100);
    expect(truncateToolResponse(small)).toBe(small);
  });

  it('truncates an oversized response and says how much it dropped', () => {
    const huge = 'x'.repeat(MAX_TOOL_RESPONSE_CHARS + 5_000);
    const out = truncateToolResponse(huge) as string;
    expect(out.length).toBeLessThan(huge.length);
    expect(out).toContain('[truncated 5000 chars]');
    expect(out.startsWith('x'.repeat(1_000))).toBe(true);
  });

  it('keeps far more than anything the UI renders', () => {
    // ToolTraceCard's excerptJson shows at most 4000 chars.
    expect(MAX_TOOL_RESPONSE_CHARS).toBeGreaterThan(4_000);
  });

  it('leaves non-string payloads alone', () => {
    // Structured results are small, and rewriting them would break the shape
    // consumers switch on.
    const obj = { is_error: true, detail: 'nope' };
    expect(truncateToolResponse(obj)).toBe(obj);
    expect(truncateToolResponse(undefined)).toBeUndefined();
    expect(truncateToolResponse(null)).toBeNull();
  });

  it('bounds a pathological Read result to a fixed size', () => {
    // The measured case: a session that Read a few large files reached 6.3MB
    // and 3.1ms per clone, paid ~60 times a second for the rest of its life.
    const fiveMb = 'y'.repeat(5 * 1024 * 1024);
    const out = truncateToolResponse(fiveMb) as string;
    expect(out.length).toBeLessThan(MAX_TOOL_RESPONSE_CHARS + 100);
  });
});

describe('MAX_FILE_CHANGES', () => {
  it('is above what the renderer keeps for background snapshots', () => {
    // compactClaudeSnapshotForBackground tails fileChanges to 80; the store
    // must not be the thing that truncates first for ordinary sessions.
    expect(MAX_FILE_CHANGES).toBeGreaterThan(80);
  });
});
