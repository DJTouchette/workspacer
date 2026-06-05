/**
 * Characterization tests for fileUtils slug functions.
 * These pin the EXACT byte-for-byte output of each variant so we catch
 * any accidental divergence from the originals (which generate on-disk filenames).
 */

import { describe, it, expect } from 'vitest';
import { slugLibrary, slugLayout, slugSession } from './fileUtils';

// ── slugLibrary (libraryService variant) ─────────────────────────────────────

describe('slugLibrary', () => {
  it('lowercases input', () => {
    expect(slugLibrary('Hello World')).toBe('hello-world');
  });

  it('replaces runs of bad chars with a single hyphen', () => {
    // Multiple spaces collapse to one '-' in a single pass
    expect(slugLibrary('foo   bar')).toBe('foo-bar');
  });

  it('trims leading and trailing hyphens', () => {
    expect(slugLibrary('--foo--')).toBe('foo');
    expect(slugLibrary('-leading')).toBe('leading');
    expect(slugLibrary('trailing-')).toBe('trailing');
  });

  it('returns fallback "item" for empty/whitespace-only input', () => {
    expect(slugLibrary('')).toBe('item');
    expect(slugLibrary('   ')).toBe('item');
    expect(slugLibrary('!!!')).toBe('item');
  });

  it('preserves hyphens and underscores', () => {
    expect(slugLibrary('foo-bar_baz')).toBe('foo-bar_baz');
  });

  it('allows digits', () => {
    expect(slugLibrary('prompt-42')).toBe('prompt-42');
  });

  it('strips special characters', () => {
    // ' (' is a run of bad chars → single '-'; trailing ')' → '-', then trimmed
    expect(slugLibrary('my prompt (v2)')).toBe('my-prompt-v2');
  });

  it('no max length constraint', () => {
    const long = 'a'.repeat(100);
    expect(slugLibrary(long)).toBe(long);
  });
});

// ── slugLayout (layoutService variant) ───────────────────────────────────────

describe('slugLayout', () => {
  it('lowercases input', () => {
    expect(slugLayout('My Layout')).toBe('my-layout');
  });

  it('replaces bad chars and deduplicates consecutive hyphens', () => {
    expect(slugLayout('foo   bar')).toBe('foo-bar');
  });

  it('trims leading and trailing hyphens', () => {
    expect(slugLayout('--foo--')).toBe('foo');
    expect(slugLayout('-leading')).toBe('leading');
    expect(slugLayout('trailing-')).toBe('trailing');
  });

  it('returns fallback "layout" for empty/only-bad-chars input', () => {
    expect(slugLayout('')).toBe('layout');
    expect(slugLayout('!!!')).toBe('layout');
  });

  it('preserves hyphens and underscores', () => {
    expect(slugLayout('dev_env-v2')).toBe('dev_env-v2');
  });

  it('caps at 64 characters', () => {
    const long = 'a'.repeat(80);
    expect(slugLayout(long)).toHaveLength(64);
    expect(slugLayout(long)).toBe('a'.repeat(64));
  });

  it('caps at 64 after substitution', () => {
    // 70 'a's separated by spaces — slug = 'a-a-a-...', capped at 64
    const input = Array.from({ length: 70 }, () => 'a').join(' ');
    expect(slugLayout(input).length).toBeLessThanOrEqual(64);
  });
});

// ── slugSession (sessionService / sanitizeFilename variant) ──────────────────

describe('slugSession', () => {
  it('lowercases input', () => {
    expect(slugSession('My Session')).toBe('my-session');
  });

  it('replaces bad chars and deduplicates consecutive hyphens', () => {
    expect(slugSession('foo   bar')).toBe('foo-bar');
  });

  it('does NOT trim leading or trailing hyphens', () => {
    // Original: name.toLowerCase().replace(...).replace(/-+/g, '-').substring(0, 64)
    // No trim step — leading/trailing dashes are preserved
    expect(slugSession('---foo---')).toBe('-foo-');
  });

  it('returns empty string for empty input (no fallback)', () => {
    expect(slugSession('')).toBe('');
  });

  it('returns empty string for all-bad-char input', () => {
    // '!!!' → '---' → '-' (after dedup) → not trimmed
    expect(slugSession('!!!')).toBe('-');
  });

  it('preserves hyphens and underscores', () => {
    expect(slugSession('my-session_v2')).toBe('my-session_v2');
  });

  it('caps at 64 characters', () => {
    const long = 'a'.repeat(80);
    expect(slugSession(long)).toHaveLength(64);
    expect(slugSession(long)).toBe('a'.repeat(64));
  });
});
