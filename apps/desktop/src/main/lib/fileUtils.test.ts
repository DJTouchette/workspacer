/**
 * Characterization tests for fileUtils slug functions.
 * These pin the EXACT byte-for-byte output of each variant so we catch
 * any accidental divergence from the originals (which generate on-disk filenames).
 */

import { describe, it, expect } from 'vitest';
import { SweepTally, itSweptTheWholeCorpus } from '../../../tests/support/sweepTally';
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

// ---------------------------------------------------------------------------
// contracts/filename-slug-cases.json — the cross-language corpus.
//
// Everything above is a characterization test of THIS copy. That is exactly how
// the U+0130 divergence survived: the Go port in cmd/brain/slug.go has its own
// characterization test, neither ever saw the other's answers, and the two
// wrote different filenames into the same config store. Unlike pricing,
// deepMerge and path containment there was no contracts/ fixture pinning the
// slugs; there is now, and this block is one of its two loaders.
// ---------------------------------------------------------------------------

import * as fs from 'fs';
import * as path from 'path';

interface SlugCase {
  name: string;
  input: string;
  expect: { library: string; layout: string; session: string };
  why?: string;
}

interface SlugFixture {
  owners: Record<string, string[]>;
  cases: SlugCase[];
}

const SLUG_OWNER = 'apps/desktop/src/main/lib/fileUtils.ts';

// apps/desktop/src/main/lib/ → five levels below the repo root, where contracts/ sits.
const slugFixture: SlugFixture = JSON.parse(
  fs.readFileSync(
    path.join(__dirname, '../../../../../contracts/filename-slug-cases.json'),
    'utf-8',
  ),
);

describe('filename slugs — cross-language contract', () => {
  it('the fixture loads and names this owner', () => {
    // Renaming this file without updating the fixture must FAIL, not silently
    // stop testing anything.
    expect(slugFixture.owners[SLUG_OWNER], `the fixture must name ${SLUG_OWNER}`).toBeDefined();
    expect([...slugFixture.owners[SLUG_OWNER]].sort()).toEqual(['layout', 'library', 'session']);
    expect(slugFixture.cases.length).toBeGreaterThan(0);
  });

  const tally = new SweepTally();
  for (const c of slugFixture.cases) {
    it(c.name, () => {
      tally.ran('other');
      expect(slugLibrary(c.input), `slugLibrary — ${c.why ?? ''}`).toBe(c.expect.library);
      expect(slugLayout(c.input), `slugLayout — ${c.why ?? ''}`).toBe(c.expect.layout);
      expect(slugSession(c.input), `slugSession — ${c.why ?? ''}`).toBe(c.expect.session);
    });

    it(`${c.name} — idempotent where the variant claims to be`, () => {
      // remove()/delete re-slug a STORED id, so a non-idempotent variant unlinks
      // a filename save() never wrote. library and layout both trim and both
      // re-trim after truncation for this reason; session deliberately does not
      // trim, so it is only required to be stable on its own output's shape.
      expect(slugLibrary(slugLibrary(c.input))).toBe(slugLibrary(c.input));
      expect(slugLayout(slugLayout(c.input))).toBe(slugLayout(c.input));
    });
  }
  // The twin of cmd/brain/slug_test.go's slugCorpusFloor. Both loaders had only
  // a `> 0` check, which a corpus down to one case passes.
  itSweptTheWholeCorpus(tally, 'the filename-slug corpus', 17, { allow: 0, deny: 0 });
});
