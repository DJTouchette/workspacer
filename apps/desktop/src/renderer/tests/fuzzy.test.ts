import { describe, it, expect } from 'vitest';
import { fuzzyScore, fuzzyScoreAny } from '../src/lib/fuzzy';

describe('fuzzyScore', () => {
  it('matches exact substrings', () => {
    expect(fuzzyScore('theme', 'Theme maker')).toBeGreaterThan(-Infinity);
    expect(fuzzyScore('key', 'Keybindings')).toBeGreaterThan(-Infinity);
  });

  it('matches subsequences (the fuzzy part)', () => {
    // "kbd" is not a substring of "keybindings" but is a subsequence
    expect(fuzzyScore('kbd', 'keybindings')).toBeGreaterThan(-Infinity);
    expect(fuzzyScore('nvim', 'neovim')).toBeGreaterThan(-Infinity);
    expect(fuzzyScore('permode', 'permission mode')).toBeGreaterThan(-Infinity);
  });

  it('rejects non-matches and out-of-order characters', () => {
    expect(fuzzyScore('xyz', 'keybindings')).toBe(-Infinity);
    expect(fuzzyScore('yek', 'key')).toBe(-Infinity);
    expect(fuzzyScore('a', '')).toBe(-Infinity);
  });

  it('is case-insensitive', () => {
    expect(fuzzyScore('THEME', 'theme')).toBeGreaterThan(-Infinity);
    expect(fuzzyScore('theme', 'THEME')).toBeGreaterThan(-Infinity);
  });

  it('empty query matches everything neutrally', () => {
    expect(fuzzyScore('', 'anything')).toBe(0);
  });

  it('ranks exact substring above scattered subsequence', () => {
    const exact = fuzzyScore('term', 'terminal');
    const scattered = fuzzyScore('term', 'the editor remembers marks');
    expect(exact).toBeGreaterThan(scattered);
  });

  it('ranks word-boundary matches above mid-word matches', () => {
    expect(fuzzyScore('mode', 'mode buttons')).toBeGreaterThan(fuzzyScore('mode', 'commodore'));
  });

  it('prefers shorter targets for the same match', () => {
    expect(fuzzyScore('font', 'font')).toBeGreaterThan(fuzzyScore('font', 'font scale factor'));
  });
});

describe('fuzzyScoreAny', () => {
  it('returns the best score across candidates', () => {
    const best = fuzzyScoreAny('shel', ['terminal', 'shell', 'bash']);
    expect(best).toBe(fuzzyScore('shel', 'shell'));
  });

  it('returns -Infinity when nothing matches', () => {
    expect(fuzzyScoreAny('zzz', ['terminal', 'shell'])).toBe(-Infinity);
  });
});
