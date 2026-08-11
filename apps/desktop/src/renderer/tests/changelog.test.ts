import { describe, it, expect } from 'vitest';
import { readFileSync } from 'fs';
import { join } from 'path';
import { CHANGELOG } from '../src/lib/changelog.generated';
import { parseChangelog } from '../../../scripts/gen-changelog.mjs';

/**
 * CHANGELOG.md is the single source for release notes; this module is generated
 * from it and is what the app, the packaged build and the web renderer actually
 * read. So the two must agree, and the generated file is committed — a drift
 * test is the only thing that catches "edited the markdown, forgot to
 * regenerate" on a checkout nobody has rebuilt.
 *
 * Same shape as configService.test.ts's drift guard over
 * configDefaults.generated.ts.
 */

const REPO_ROOT = join(__dirname, '..', '..', '..', '..', '..');
const SOURCE = join(REPO_ROOT, 'CHANGELOG.md');

describe('changelog.generated.ts', () => {
  it('matches CHANGELOG.md — regenerate with `npm run gen:changelog`', () => {
    const fresh = parseChangelog(readFileSync(SOURCE, 'utf8'));
    expect(JSON.parse(JSON.stringify(CHANGELOG))).toEqual(fresh);
  });

  it('parses the real file into something with entries', () => {
    // A parser that silently matched nothing would satisfy the equality above
    // (both sides empty) while shipping an empty "what's new" screen.
    expect(CHANGELOG.length).toBeGreaterThan(0);
    const entries = CHANGELOG.reduce(
      (n, r) => n + r.sections.reduce((m, s) => m + s.items.length, 0),
      0,
    );
    expect(entries).toBeGreaterThan(0);
  });

  it('gives every release a version and every non-Unreleased one a date', () => {
    for (const r of CHANGELOG) {
      expect(r.version, 'a release heading with no version').toBeTruthy();
      if (!r.unreleased) {
        expect(r.date, `${r.version} has no date`).toMatch(/^\d{4}-\d{2}-\d{2}$/);
      }
    }
  });

  it('joins an entry that wraps onto continuation lines into one paragraph', () => {
    // The file hard-wraps for readable diffs; the UI must not show the wrap.
    // Pinned because losing it degrades silently — the text still renders, just
    // truncated at the first line break, which reads like a terse entry.
    const wrapped = CHANGELOG.flatMap((r) => r.sections.flatMap((s) => s.items)).filter(
      (t) => t.length > 120,
    );
    expect(wrapped.length, 'no multi-line entry survived the parse').toBeGreaterThan(0);
    for (const t of wrapped) expect(t).not.toContain('\n');
  });

  it('refuses a release heading with no entries under it', () => {
    expect(() => parseChangelog('## [9.9.9] - 2026-01-01\n\n### Added\n')).toThrow(/no entries/);
  });
});
