/**
 * THE READ-SET INVARIANT: the set of files `search.project` may return CONTENT
 * from cannot exceed `fs.read`'s.
 *
 * The composition this closes takes two calls that are each correctly confined.
 * `search.project` applies assertPathAllowed to its CWD and to nothing else, and
 * delegates per-file exclusion to ripgrep's hidden/ignore walker — whose policy
 * is A FILE INSIDE THE SEARCHED DIRECTORY. `<cwd>/.ignore` holding `!*` is an
 * ordinary dotfile to every guard in the repo: not a credential basename, no
 * `.git` component, inside the root. So:
 *
 *     fs.write       <root>/.ignore   "!*\n!**\/*\n"     -> allowed
 *     search.project { cwd: <root> }                     -> allowed
 *
 * and the second returned matching lines out of `<root>/.git/config` and
 * `<root>/.settings.json`, the two files the secret gate exists to refuse.
 * Bytes written as DATA by the first call became the READ POLICY of the second.
 *
 * This runs the REAL ripgrep with the real `.ignore` in place, so the un-hiding
 * actually happens and the gate is what stops it. The fix is deliberately not
 * "make ripgrep ignore `.ignore`" — its walker has several such files and their
 * precedence is its business — which is why the assertion is about the RESULT
 * SET rather than about the argv.
 *
 * TWIN: cmd/brain/search_test.go TestSearchDropsFilesFsReadWouldRefuse.
 */
import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import { searchProject } from './searchService';

const TOKEN = 'CHAIN_SECRET_TOKEN';

let dir: string;
beforeEach(() => {
  dir = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'wks-search-readset-')));
  const write = (rel: string, body: string): void => {
    const p = path.join(dir, rel);
    fs.mkdirSync(path.dirname(p), { recursive: true });
    fs.writeFileSync(p, body);
  };
  write('readme.md', `ordinary ${TOKEN} placeholder\n`);
  write('.git/config', `\turl = https://x-access-token:${TOKEN}@github.com/acme/p.git\n`);
  write('.settings.json', `{"apiKey":"${TOKEN}"}\n`);
  write('.claude/settings.json', `{"note":"${TOKEN}"}\n`);
  write('.mcp.json', `{"note":"${TOKEN}"}\n`);
});
afterEach(() => {
  fs.rmSync(dir, { recursive: true, force: true });
});

/** Result file paths, relative to the searched directory. */
async function hits(): Promise<string[]> {
  const r = await searchProject({ query: TOKEN, cwd: dir, caseSensitive: true });
  return r.results.map((f) => path.relative(dir, f.file).split(path.sep).join('/')).sort();
}

describe('search.project — a planted ignore file cannot move its read set', () => {
  it('returns only the ordinary file before anything is planted', async () => {
    // The BASELINE, and it is load-bearing twice over: it shows the hidden files
    // start out excluded (so the assertion below is about the gate and not about
    // ripgrep's defaults), and it shows the search returns anything at all.
    expect(await hits()).toEqual(['readme.md']);
  });

  it('still returns only the ordinary file once an ignore file un-hides everything', async () => {
    // Exactly what `fs.write` accepts today: an ordinary dotfile inside a root.
    fs.writeFileSync(path.join(dir, '.ignore'), '!*\n!**/*\n');

    const got = await hits();
    for (const denied of ['.git/config', '.settings.json', '.claude/settings.json', '.mcp.json']) {
      expect(
        got,
        `search.project returned bytes from ${denied}, which fs.read refuses — one fs.write of an ordinary dotfile moved this capability's read set`,
      ).not.toContain(denied);
    }
    // THE FLOOR. Without it a gate that drops every result passes the loop above
    // while silently breaking the capability.
    expect(got, 'the gate is dropping ordinary files too').toContain('readme.md');
  });
});
