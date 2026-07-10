import { describe, it, expect, beforeAll, afterAll } from 'vitest';
import { execFileSync } from 'child_process';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import { worktreeInfo, createWorktree } from './worktreeService';

// Real git, real temp repo — the service is a thin shell-out and mocking git
// would test nothing.
let tmp: string;
let repo: string;
let wtRoot: string;

function git(args: string[], cwd: string): string {
  return execFileSync('git', args, { cwd, encoding: 'utf8' }).trim();
}

beforeAll(() => {
  tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'wks-worktree-'));
  repo = path.join(tmp, 'myrepo');
  wtRoot = path.join(tmp, 'trees');
  fs.mkdirSync(repo, { recursive: true });
  git(['init', '-q', '-b', 'main'], repo);
  git(['config', 'user.email', 't@t.t'], repo);
  git(['config', 'user.name', 't'], repo);
  fs.writeFileSync(path.join(repo, 'a.txt'), 'hello\n');
  git(['add', '.'], repo);
  git(['commit', '-q', '-m', 'init'], repo);
});

afterAll(() => {
  fs.rmSync(tmp, { recursive: true, force: true });
});

describe('worktreeInfo', () => {
  it('detects a repo with root + branch', async () => {
    const info = await worktreeInfo(repo);
    expect(info.isRepo).toBe(true);
    expect(fs.realpathSync(info.root!)).toBe(fs.realpathSync(repo));
    expect(info.branch).toBe('main');
  });

  it('detects a subdirectory of a repo', async () => {
    const sub = path.join(repo, 'src');
    fs.mkdirSync(sub, { recursive: true });
    const info = await worktreeInfo(sub);
    expect(info.isRepo).toBe(true);
    expect(fs.realpathSync(info.root!)).toBe(fs.realpathSync(repo));
  });

  it('reports non-repos and missing paths', async () => {
    expect((await worktreeInfo(tmp)).isRepo).toBe(false);
    expect((await worktreeInfo(path.join(tmp, 'nope'))).isRepo).toBe(false);
    expect((await worktreeInfo('')).isRepo).toBe(false);
  });
});

describe('createWorktree', () => {
  it('creates a worktree on a fresh wks/<slug> branch', async () => {
    const res = await createWorktree({
      repoCwd: repo,
      name: 'Fix Auth Bug!',
      rootOverride: wtRoot,
    });
    expect(res.ok).toBe(true);
    expect(res.branch).toBe('wks/fix-auth-bug');
    expect(res.path).toBe(path.join(wtRoot, 'myrepo', 'fix-auth-bug'));
    // It's a real checkout of the repo content on the right branch.
    expect(fs.readFileSync(path.join(res.path!, 'a.txt'), 'utf8')).toBe('hello\n');
    expect(git(['rev-parse', '--abbrev-ref', 'HEAD'], res.path!)).toBe('wks/fix-auth-bug');
    // Registered as a worktree of the main repo.
    expect(git(['worktree', 'list'], repo)).toContain(res.path!);
  });

  it('disambiguates on a second spawn with the same name', async () => {
    const res = await createWorktree({
      repoCwd: repo,
      name: 'Fix Auth Bug!',
      rootOverride: wtRoot,
    });
    expect(res.ok).toBe(true);
    expect(res.path).not.toBe(path.join(wtRoot, 'myrepo', 'fix-auth-bug'));
    expect(res.branch).toMatch(/^wks\/fix-auth-bug-[a-z0-9]{4}$/);
  });

  it('fails cleanly outside a repo', async () => {
    const res = await createWorktree({ repoCwd: tmp, name: 'x', rootOverride: wtRoot });
    expect(res.ok).toBe(false);
    expect(res.error).toMatch(/not inside a git repository/);
  });

  it('defaults the slug when no name is given', async () => {
    const res = await createWorktree({ repoCwd: repo, rootOverride: wtRoot });
    expect(res.ok).toBe(true);
    expect(res.branch).toBe('wks/agent');
  });
});
