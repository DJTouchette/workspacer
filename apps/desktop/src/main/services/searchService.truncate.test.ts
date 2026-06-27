/**
 * Regression test: `truncated` must reflect a genuinely dropped match, not just
 * the running total reaching maxResults. ripgrep's --json stream emits trailing
 * `end`/`summary` messages after the last match; the cap check must not treat
 * those as evidence of truncation when the count lands exactly on the cap.
 */
import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import { searchProject } from './searchService';

let dir: string;
beforeEach(() => { dir = fs.mkdtempSync(path.join(os.tmpdir(), 'wks-search-')); });
afterEach(() => { fs.rmSync(dir, { recursive: true, force: true }); });

describe('searchProject — truncated flag accuracy', () => {
  it('does not flag truncated when match count equals maxResults exactly', async () => {
    fs.writeFileSync(path.join(dir, 'a.txt'), 'x\nx\nx\n');
    const r = await searchProject({ query: 'x', cwd: dir, caseSensitive: true, maxResults: 3 });
    const total = r.results.reduce((n, f) => n + f.matches.length, 0);
    expect(total).toBe(3);
    expect(r.truncated).toBe(false);
  });

  it('flags truncated when there are genuinely more matches than maxResults', async () => {
    fs.writeFileSync(path.join(dir, 'a.txt'), 'x\nx\nx\nx\n');
    const r = await searchProject({ query: 'x', cwd: dir, caseSensitive: true, maxResults: 3 });
    expect(r.truncated).toBe(true);
  });
});
