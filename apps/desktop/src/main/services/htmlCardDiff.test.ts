import { it, expect, beforeEach, afterEach, vi } from 'vitest';
import fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import { execFileSync } from 'child_process';
import { readHtmlCardDiff } from './gitService';
import { resolveHtmlCardPath, readHtmlCardFile } from './htmlCardPaths';
let scratch: string;
let cwd: string;
beforeEach(() => {
  scratch = fs.mkdtempSync(path.join(os.tmpdir(), 'wks-card-read-'));
  cwd = path.join(scratch, 'project');
  fs.mkdirSync(cwd);
  const git = (...args: string[]) => execFileSync('git', args, { cwd, stdio: 'ignore' });
  git('init');
  git('config', 'user.name', 'Fixture');
  git('config', 'user.email', 'fixture@example.invalid');
  fs.writeFileSync(path.join(cwd, 'file.txt'), 'before\n');
  git('add', '.');
  git('-c', 'commit.gpgsign=false', 'commit', '-m', 'fixture');
  fs.writeFileSync(path.join(cwd, 'file.txt'), 'after\n');
  fs.writeFileSync(path.join(scratch, 'outside.txt'), 'outside secret');
});
afterEach(() => {
  vi.restoreAllMocks();
  fs.rmSync(scratch, { recursive: true, force: true });
});
it('returns exact tracked and untracked snapshots without basename fallback', async () => {
  expect(await readHtmlCardDiff('file.txt', cwd)).toEqual({
    ok: true,
    path: path.join(cwd, 'file.txt'),
    before: 'before\n',
    after: 'after\n',
  });
  fs.mkdirSync(path.join(cwd, 'nested'));
  fs.writeFileSync(path.join(cwd, 'nested/file.txt'), 'new nested');
  expect(await readHtmlCardDiff('nested/file.txt', cwd)).toEqual({
    ok: true,
    path: path.join(cwd, 'nested/file.txt'),
    before: '',
    after: 'new nested',
  });
  expect((await readHtmlCardDiff('missing/file.txt', cwd)).ok).toBe(false);
});
it('rejects a symlink replaced after initial validation, including untracked files', async () => {
  for (const name of ['file.txt', 'untracked.txt']) {
    const target = path.join(cwd, name);
    fs.writeFileSync(target, 'safe');
    expect(resolveHtmlCardPath(target, cwd).ok).toBe(true);
    fs.unlinkSync(target);
    fs.symlinkSync(path.join(scratch, 'outside.txt'), target);
    expect((await readHtmlCardDiff(target, cwd)).ok).toBe(false);
  }
});
it('refuses a parent symlink swap during descriptor open before reading any bytes', () => {
  fs.mkdirSync(path.join(cwd, 'dir'));
  fs.writeFileSync(path.join(cwd, 'dir/outside.txt'), 'safe');
  const original = fs.openSync;
  vi.spyOn(fs, 'openSync').mockImplementation(((target: string, flags: number) => {
    fs.renameSync(path.join(cwd, 'dir'), path.join(cwd, 'saved'));
    fs.symlinkSync(scratch, path.join(cwd, 'dir'));
    return original(target, flags);
  }) as typeof fs.openSync);
  const read = vi.spyOn(fs, 'readSync');
  expect(() => readHtmlCardFile('dir/outside.txt', cwd)).toThrow();
  expect(read).not.toHaveBeenCalled();
});
it('rejects escapes, configuration, binary and oversized working files', async () => {
  fs.writeFileSync(path.join(cwd, '.mcp.json'), '{}');
  fs.writeFileSync(path.join(cwd, 'binary'), Buffer.from([1, 0, 2]));
  fs.writeFileSync(path.join(cwd, 'large'), Buffer.alloc(256 * 1024 + 1, 65));
  for (const target of ['../outside.txt', '.mcp.json', 'binary', 'large'])
    expect((await readHtmlCardDiff(target, cwd)).ok, target).toBe(false);
});
