import { afterEach, describe, expect, it, vi } from 'vitest';
import fs from 'fs';
import os from 'os';
import path from 'path';
import { execFileSync } from 'child_process';
import {
  cleanupWorktreeArtifacts,
  loadWorktreeCleanupState,
  type CleanupState,
} from './worktreeArtifactCleanup';
import {
  linkedWorktree,
  recordWorktreeAllocation,
  withWorktreeMaintenance,
  WORKTREE_ALLOCATION,
  WORKTREE_MAINTENANCE_LOCK,
} from './worktreeMaintenance';

const dirs: string[] = [];
const ready: CleanupState = { maintenanceSupported: true, sessions: [] };
function git(cwd: string, args: string[]) {
  return execFileSync('git', args, {
    cwd,
    encoding: 'utf8',
    stdio: ['ignore', 'pipe', 'pipe'],
  }).trim();
}
function write(file: string, body = 'generated output') {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, body);
}
async function fixture() {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'artifact-cleanup-'));
  dirs.push(dir);
  const repo = path.join(dir, 'source'),
    root = path.join(dir, 'worktrees'),
    cwd = path.join(root, 'source', 'worker');
  fs.mkdirSync(repo);
  git(repo, ['init', '-q', '-b', 'main']);
  git(repo, ['config', 'user.email', 'test@example.com']);
  git(repo, ['config', 'user.name', 'test']);
  write(
    path.join(repo, '.gitignore'),
    'node_modules/\nbin/\nobj/\ntarget/\ndist/\nprivate-cache/\n',
  );
  write(path.join(repo, 'package.json'), '{}');
  write(path.join(repo, 'App.csproj'), '<Project />');
  write(path.join(repo, 'Cargo.toml'), '[package]\nname="fixture"\nversion="0.1.0"');
  write(path.join(repo, 'source.ts'), 'original source');
  git(repo, ['add', '.']);
  git(repo, ['commit', '-qm', 'fixture']);
  fs.mkdirSync(path.dirname(cwd), { recursive: true });
  git(repo, ['worktree', 'add', '-q', '-b', 'wks/worker', cwd]);
  await recordWorktreeAllocation(cwd, root);
  const gitDir = (await linkedWorktree(cwd))!.gitDir;
  const run = (state = ready, apply = true, minAgeMs = 0) =>
    cleanupWorktreeArtifacts({ root, apply, minAgeMs, loadState: async () => state });
  return { dir, repo, root, cwd, gitDir, run };
}
afterEach(() => {
  vi.unstubAllGlobals();
  for (const dir of dirs.splice(0)) fs.rmSync(dir, { recursive: true, force: true });
});

describe('agent artifact reclamation', () => {
  it('previews then reclaims ignored generated output while keeping dirty source, untracked code and branches', async () => {
    const f = await fixture();
    for (const name of ['node_modules', 'bin', 'obj', 'target', 'dist'])
      write(path.join(f.cwd, name, 'large.bin'));
    write(path.join(f.cwd, 'source.ts'), 'uncommitted edits');
    write(path.join(f.cwd, 'new-source.ts'), 'valuable untracked code');
    write(path.join(f.cwd, 'private-cache', 'notes.txt'), 'valuable ignored notes');
    const preview = await f.run(ready, false);
    expect(preview.artifacts.filter((item) => item.action === 'planned')).toHaveLength(5);
    expect(preview.removedBytes).toBe(0);
    expect(fs.existsSync(path.join(f.cwd, 'node_modules'))).toBe(true);
    const result = await f.run();
    expect(result.artifacts.filter((item) => item.action === 'removed')).toHaveLength(5);
    expect(result.removedBytes).toBeGreaterThan(0);
    expect(fs.readFileSync(path.join(f.cwd, 'source.ts'), 'utf8')).toBe('uncommitted edits');
    expect(fs.existsSync(path.join(f.cwd, 'new-source.ts'))).toBe(true);
    expect(fs.existsSync(path.join(f.cwd, 'private-cache', 'notes.txt'))).toBe(true);
    expect(git(f.cwd, ['branch', '--show-current'])).toBe('wks/worker');
    expect(git(f.repo, ['worktree', 'list'])).toContain(f.cwd);
  });

  it('protects tracked files, nonignored directories and nested repositories', async () => {
    const f = await fixture();
    write(path.join(f.cwd, 'bin', 'important.cs'));
    git(f.cwd, ['add', '-f', 'bin/important.cs']);
    write(path.join(f.cwd, 'build', 'not-ignored.txt'));
    write(path.join(f.cwd, 'node_modules', 'local-package', '.git', 'config'));
    const result = await f.run();
    expect(result.removedBytes).toBe(0);
    expect(result.artifacts.every((item) => item.action === 'skipped')).toBe(true);
    expect(fs.existsSync(path.join(f.cwd, 'bin', 'important.cs'))).toBe(true);
    expect(result.artifacts.map((item) => item.reason).join('\n')).toMatch(/tracked files/);
    expect(result.artifacts.map((item) => item.reason).join('\n')).toMatch(/nested Git/);
  });

  it('never follows dependency or nested artifact symlinks', async () => {
    const f = await fixture();
    write(path.join(f.repo, 'node_modules', 'shared.txt'), 'keep shared deps');
    fs.symlinkSync(path.join(f.repo, 'node_modules'), path.join(f.cwd, 'node_modules'), 'junction');
    write(path.join(f.cwd, 'bin', 'output.dll'));
    fs.symlinkSync(
      path.join(f.repo, 'node_modules'),
      path.join(f.cwd, 'bin', 'linked-deps'),
      'junction',
    );
    await f.run();
    expect(fs.existsSync(path.join(f.cwd, 'bin'))).toBe(false);
    expect(fs.lstatSync(path.join(f.cwd, 'node_modules')).isSymbolicLink()).toBe(true);
    expect(fs.readFileSync(path.join(f.repo, 'node_modules', 'shared.txt'), 'utf8')).toBe(
      'keep shared deps',
    );
  });

  it('protects dependencies referenced by another worktree, including one added during the sweep', async () => {
    const f = await fixture();
    write(path.join(f.cwd, 'node_modules', 'shared.txt'));
    const other = path.join(f.root, 'source', 'child');
    const result = await cleanupWorktreeArtifacts({
      root: f.root,
      apply: true,
      minAgeMs: 0,
      loadState: async () => {
        git(f.repo, ['worktree', 'add', '-q', '-b', 'wks/child', other]);
        fs.symlinkSync(
          path.join(f.cwd, 'node_modules'),
          path.join(other, 'node_modules'),
          'junction',
        );
        return ready;
      },
    });
    expect(result.artifacts[0].reason).toMatch(/Referenced/);
    expect(fs.existsSync(path.join(f.cwd, 'node_modules', 'shared.txt'))).toBe(true);
  });

  it('finds nested and hidden shared dependency links in worktrees under a different root', async () => {
    const f = await fixture();
    write(path.join(f.cwd, 'node_modules', 'pkg', 'index.js'));
    const other = path.join(f.dir, 'different-root', 'child');
    fs.mkdirSync(path.dirname(other));
    git(f.repo, ['worktree', 'add', '-q', '-b', 'wks/cross-root', other]);
    fs.mkdirSync(path.join(other, 'node_modules'));
    fs.symlinkSync(
      path.join(f.cwd, 'node_modules', 'pkg'),
      path.join(other, 'node_modules', 'pkg'),
      'junction',
    );
    const nested = await f.run({ ...ready, sessions: [{ mode: 'input', cwd: other }] });
    expect(nested.artifacts[0].reason).toMatch(/Referenced/);
    fs.unlinkSync(path.join(other, 'node_modules', 'pkg'));
    fs.mkdirSync(path.join(other, '.dependencies'));
    fs.symlinkSync(
      path.join(f.cwd, 'node_modules'),
      path.join(other, '.dependencies', 'deps'),
      'junction',
    );
    expect((await f.run()).artifacts[0].reason).toMatch(/Referenced/);
    expect(fs.existsSync(path.join(f.cwd, 'node_modules', 'pkg', 'index.js'))).toBe(true);
  });

  it('honors git worktree locks and never cleans a primary checkout placed under the managed root', async () => {
    const f = await fixture();
    write(path.join(f.cwd, 'bin', 'output.dll'));
    git(f.repo, ['worktree', 'lock', f.cwd]);
    expect((await f.run()).skipped[0].reason).toMatch(/worktree is locked/);
    git(f.repo, ['worktree', 'unlock', f.cwd]);
    const primary = path.join(f.root, 'source', 'primary');
    fs.mkdirSync(primary);
    git(primary, ['init', '-q']);
    write(path.join(primary, 'package.json'), '{}');
    write(path.join(primary, 'node_modules', 'keep.txt'));
    await f.run();
    expect(fs.existsSync(path.join(primary, 'node_modules', 'keep.txt'))).toBe(true);
  });

  it.each(['input', 'responding', 'unknown', 'approval', 'question'])(
    'does not clean a live %s session even when its cwd is nested',
    async (mode) => {
      const f = await fixture();
      write(path.join(f.cwd, 'bin', 'output.dll'));
      fs.mkdirSync(path.join(f.cwd, 'src'));
      const report = await f.run({ ...ready, sessions: [{ mode, cwd: path.join(f.cwd, 'src') }] });
      expect(report.skipped[0].reason).toMatch(/live session/);
      expect(fs.existsSync(path.join(f.cwd, 'bin'))).toBe(true);
    },
  );

  it('protects original and live directories, unknown live cwd, recent stops and recent artifacts', async () => {
    const f = await fixture();
    write(path.join(f.cwd, 'bin', 'output.dll'));
    const marker = path.join(f.gitDir, WORKTREE_ALLOCATION);
    const record = JSON.parse(fs.readFileSync(marker, 'utf8'));
    record.createdAt = new Date(Date.now() - 7200_000).toISOString();
    fs.writeFileSync(marker, JSON.stringify(record));
    for (const sessions of [
      [{ mode: 'input', cwd: f.repo, liveCwd: f.cwd }],
      [{ mode: 'input', cwd: null }],
      [{ mode: 'stopped', cwd: f.cwd, updated_at: new Date().toISOString() }],
    ])
      expect((await f.run({ ...ready, sessions }, true, 3600_000)).skipped).toHaveLength(1);
    const recent = await f.run(ready, true, 3600_000);
    expect(recent.artifacts[0].reason).toMatch(/recently written/);
    expect(fs.existsSync(path.join(f.cwd, 'bin'))).toBe(true);
  });

  it('requires compatible daemon state and releases the lease after errors', async () => {
    const f = await fixture();
    write(path.join(f.cwd, 'bin', 'output.dll'));
    expect((await f.run({ ...ready, maintenanceSupported: false })).skipped[0].reason).toMatch(
      /lacks worktree-maintenance/,
    );
    const failed = await cleanupWorktreeArtifacts({
      root: f.root,
      apply: true,
      minAgeMs: 0,
      loadState: async () => {
        expect(fs.existsSync(path.join(f.gitDir, WORKTREE_MAINTENANCE_LOCK))).toBe(true);
        throw new Error('daemon unreachable');
      },
    });
    expect(failed.skipped[0].reason).toMatch(/daemon unreachable/);
    expect(fs.existsSync(path.join(f.gitDir, WORKTREE_MAINTENANCE_LOCK))).toBe(false);
    expect(fs.existsSync(path.join(f.cwd, 'bin'))).toBe(true);
  });

  it('recognizes legacy wks branches and refuses changed allocation identity or occupied leases', async () => {
    const f = await fixture();
    write(path.join(f.cwd, 'bin', 'output.dll'));
    const marker = path.join(f.gitDir, WORKTREE_ALLOCATION);
    const allocation = JSON.parse(fs.readFileSync(marker, 'utf8'));
    fs.writeFileSync(marker, JSON.stringify({ ...allocation, ino: allocation.ino + 1 }));
    expect((await f.run()).skipped[0].reason).toMatch(/identity changed/);
    fs.unlinkSync(marker);
    await withWorktreeMaintenance(f.gitDir, async () => {
      expect((await f.run()).skipped[0].reason).toMatch(/maintenance/);
      expect(fs.existsSync(path.join(f.cwd, 'bin'))).toBe(true);
    });
    expect((await f.run()).artifacts[0].action).toBe('removed');
  });

  it('negotiates daemon protection without changing the health payload contract', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async (url: string) =>
        url.endsWith('/health')
          ? new Response('ok', { headers: { 'X-Workspacer-Maintenance': '1' } })
          : new Response(JSON.stringify([{ mode: 'stopped', cwd: '/project' }])),
      ),
    );
    expect((await loadWorktreeCleanupState('http://localhost:1')).maintenanceSupported).toBe(true);
  });
});
