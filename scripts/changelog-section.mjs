#!/usr/bin/env node
// Print one release's section of CHANGELOG.md, for the GitHub Release body.
//
//   node scripts/changelog-section.mjs 1.2.3      (or v1.2.3)
//
// Exits 1 with a message when that version has no section, which is deliberate:
// release.yml runs this on a version tag, so tagging a release nobody wrote
// notes for FAILS THE BUILD instead of publishing an empty release page. The
// notes and the installers ship together or not at all.
//
// The app reads the same file through apps/desktop/scripts/gen-changelog.mjs, so
// the release page and Settings → Updates cannot disagree.

import { readFileSync } from 'fs';
import { fileURLToPath } from 'url';
import { dirname, join } from 'path';

const repoRoot = join(dirname(fileURLToPath(import.meta.url)), '..');

/** The lines under `## [<version>]`, up to the next `## ` heading. */
export function sectionFor(markdown, version) {
  const want = version.replace(/^v/, '');
  const lines = markdown.split('\n');
  const start = lines.findIndex((l) => new RegExp(`^##\\s+\\[${escapeRe(want)}\\]`).test(l));
  if (start < 0) return null;
  const rest = lines.slice(start + 1);
  const end = rest.findIndex((l) => /^##\s/.test(l));
  const body = (end < 0 ? rest : rest.slice(0, end)).join('\n').trim();
  return body || null;
}

function escapeRe(s) {
  return s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

if (process.argv[1] && import.meta.url === new URL(`file://${process.argv[1]}`).href) {
  const version = process.argv[2];
  if (!version) {
    console.error('usage: changelog-section.mjs <version>');
    process.exit(2);
  }
  const md = readFileSync(join(repoRoot, 'CHANGELOG.md'), 'utf8');
  const body = sectionFor(md, version);
  if (!body) {
    console.error(
      `CHANGELOG.md has no entries for ${version}. Add a "## [${version.replace(/^v/, '')}] - YYYY-MM-DD" ` +
        `section before tagging — a release with no notes is a release nobody can read.`,
    );
    process.exit(1);
  }
  process.stdout.write(body + '\n');
}
