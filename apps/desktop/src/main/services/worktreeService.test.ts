import { describe, it, expect, beforeAll, afterAll } from 'vitest';
import { execFileSync } from 'child_process';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import {
  worktreeInfo,
  createWorktree,
  removeAgentWorktree,
  discoverNodeModules,
  linkNodeModules,
} from './worktreeService';

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

describe('node_modules linking', () => {
  // Its own repo: these tests need node_modules dirs + specific .gitignore
  // content without disturbing the shared fixture above.
  let nmRepo: string;
  function makeRepo(name: string, gitignore: string): string {
    const r = path.join(tmp, name);
    fs.mkdirSync(path.join(r, 'apps', 'desktop'), { recursive: true });
    git(['init', '-q', '-b', 'main'], r);
    git(['config', 'user.email', 't@t.t'], r);
    git(['config', 'user.name', 't'], r);
    fs.writeFileSync(path.join(r, '.gitignore'), gitignore);
    fs.writeFileSync(path.join(r, 'apps', 'desktop', 'pkg.txt'), 'x\n');
    git(['add', '.'], r);
    git(['commit', '-q', '-m', 'init'], r);
    // Deps exist only on disk, never committed.
    fs.mkdirSync(path.join(r, 'node_modules', 'dep'), { recursive: true });
    fs.writeFileSync(path.join(r, 'node_modules', 'dep', 'index.js'), '1\n');
    fs.mkdirSync(path.join(r, 'apps', 'desktop', 'node_modules'), { recursive: true });
    return r;
  }

  beforeAll(() => {
    nmRepo = makeRepo('nmrepo', 'node_modules\n');
  });

  it('discovers node_modules at depth ≤ 2 only, without descending into them', async () => {
    // Too deep (parent depth 3) and nested-inside-node_modules must not match.
    fs.mkdirSync(path.join(nmRepo, 'a', 'b', 'c', 'node_modules'), { recursive: true });
    fs.mkdirSync(path.join(nmRepo, 'node_modules', 'dep', 'node_modules'), { recursive: true });
    const found = await discoverNodeModules(nmRepo);
    expect(found.sort()).toEqual([path.join('apps', 'desktop', 'node_modules'), 'node_modules']);
  });

  it('symlinks discovered node_modules into a fresh worktree, and the tree stays clean', async () => {
    const wt = await createWorktree({ repoCwd: nmRepo, name: 'deps', rootOverride: wtRoot });
    expect(wt.ok).toBe(true);
    for (const rel of ['node_modules', path.join('apps', 'desktop', 'node_modules')]) {
      const link = path.join(wt.path!, rel);
      expect(fs.lstatSync(link).isSymbolicLink()).toBe(true);
      expect(fs.realpathSync(link)).toBe(fs.realpathSync(path.join(nmRepo, rel)));
    }
    // Deps are actually reachable through the link.
    expect(fs.existsSync(path.join(wt.path!, 'node_modules', 'dep', 'index.js'))).toBe(true);
    expect(git(['status', '--porcelain'], wt.path!)).toBe('');
    // Teardown is unaffected by the ignored symlinks.
    const res = await removeAgentWorktree({ cwd: wt.path!, rootOverride: wtRoot });
    expect(res.ok).toBe(true);
    // Removing the worktree never reaches through the link into real deps.
    expect(fs.existsSync(path.join(nmRepo, 'node_modules', 'dep', 'index.js'))).toBe(true);
  });

  it('rolls back links that git would NOT ignore (dir-only `node_modules/` pattern)', async () => {
    // The classic gotcha: `node_modules/` matches directories but not symlinks.
    const r = makeRepo('nmrepo-dironly', 'node_modules/\n');
    const wt = await createWorktree({ repoCwd: r, name: 'deps', rootOverride: wtRoot });
    expect(wt.ok).toBe(true);
    expect(fs.existsSync(path.join(wt.path!, 'node_modules'))).toBe(false);
    expect(git(['status', '--porcelain'], wt.path!)).toBe('');
  });

  it('leaves an existing path alone and skips repos with no node_modules', async () => {
    const wt = await createWorktree({ repoCwd: nmRepo, name: 'occupied', rootOverride: wtRoot });
    expect(wt.ok).toBe(true);
    // Simulate something already at the path, then re-run the linker directly.
    fs.rmSync(path.join(wt.path!, 'node_modules'));
    fs.mkdirSync(path.join(wt.path!, 'node_modules', 'mine'), { recursive: true });
    const linked = await linkNodeModules(nmRepo, wt.path!);
    expect(linked).toEqual([]); // root occupied, apps/desktop already linked
    expect(fs.lstatSync(path.join(wt.path!, 'node_modules')).isSymbolicLink()).toBe(false);
    expect(fs.existsSync(path.join(wt.path!, 'node_modules', 'mine'))).toBe(true);
    // A source with nothing to link is a silent no-op.
    expect(await linkNodeModules(repo, wt.path!)).toEqual([]);
  });
});

describe('removeAgentWorktree', () => {
  it('removes a CLEAN agent worktree but keeps its branch (committed work survives)', async () => {
    const wt = await createWorktree({
      repoCwd: repo,
      name: 'teardown-clean',
      rootOverride: wtRoot,
    });
    expect(wt.ok).toBe(true);
    const res = await removeAgentWorktree({ cwd: wt.path!, rootOverride: wtRoot });
    expect(res.ok).toBe(true);
    expect(fs.existsSync(wt.path!)).toBe(false);
    // The branch (where a worker's commits / PR live) is NOT deleted.
    expect(git(['branch', '--list', wt.branch!], repo)).toContain(wt.branch!.replace('wks/', ''));
  });

  it('REFUSES a dirty worktree (uncommitted work) and leaves it in place', async () => {
    const wt = await createWorktree({
      repoCwd: repo,
      name: 'teardown-dirty',
      rootOverride: wtRoot,
    });
    fs.writeFileSync(path.join(wt.path!, 'scratch.txt'), 'unsaved work\n');
    const res = await removeAgentWorktree({ cwd: wt.path!, rootOverride: wtRoot });
    expect(res.ok).toBe(false);
    expect(res.skipped).toBe(true);
    expect(fs.existsSync(wt.path!)).toBe(true); // work preserved
  });

  it('NEVER touches the primary checkout (not a linked worktree)', async () => {
    const res = await removeAgentWorktree({ cwd: repo, rootOverride: wtRoot });
    expect(res.skipped).toBe(true);
    expect(fs.existsSync(repo)).toBe(true);
  });

  it('skips a path outside the agent worktree root', async () => {
    const res = await removeAgentWorktree({ cwd: repo, rootOverride: path.join(tmp, 'other') });
    expect(res.skipped).toBe(true);
  });
});
