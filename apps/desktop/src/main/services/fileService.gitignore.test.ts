/**
 * Regression test: listDir must hide gitignored files regardless of whether
 * their names are ASCII. git's `check-ignore` C-quotes non-ASCII paths under
 * the default core.quotePath=true (e.g. "\303\251.log"), while fs.readdir
 * returns the decoded name ('é.log'); comparing those never matches, so the
 * ignored unicode file leaked into the editor's file tree.
 */
import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import { execFileSync } from 'child_process';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import { listDir } from './fileService';

let dir: string;
beforeEach(() => {
  dir = fs.mkdtempSync(path.join(os.tmpdir(), 'wks-ls-'));
  execFileSync('git', ['init', '-q'], { cwd: dir });
  fs.writeFileSync(path.join(dir, '.gitignore'), '*.log\n');
  fs.writeFileSync(path.join(dir, 'ascii.log'), '');
  fs.writeFileSync(path.join(dir, 'é.log'), '');
  fs.writeFileSync(path.join(dir, 'keep.txt'), '');
});
afterEach(() => { fs.rmSync(dir, { recursive: true, force: true }); });

describe('listDir — gitignore filtering', () => {
  it('hides gitignored files with non-ASCII names', () => {
    const names = listDir(dir).entries.map((e) => e.name);
    expect(names).toContain('keep.txt');
    expect(names).not.toContain('ascii.log'); // sanity: ASCII ignore works
    expect(names).not.toContain('é.log'); // the bug: unicode ignore leaked
  });
});
