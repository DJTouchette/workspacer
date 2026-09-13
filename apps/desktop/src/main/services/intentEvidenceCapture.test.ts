import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import { mkdtempSync, mkdirSync, readFileSync, rmSync, symlinkSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { afterEach, expect, it, vi } from 'vitest';
vi.mock('./configService', () => ({ getConfigDir: () => '/unused-intent-evidence-tests' }));
import { captureIntentGit, INTENT_CAPTURE_LIMITS } from './intentEvidenceCapture';
const roots: string[] = [];
afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllEnvs();
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});
function repo() {
  const root = mkdtempSync(path.join(tmpdir(), 'intent-git-'));
  roots.push(root);
  const git = (...args: string[]) =>
    execFileSync('git', args, {
      cwd: root,
      encoding: 'utf8',
      stdio: ['ignore', 'pipe', 'pipe'],
    }).trim();
  git('init');
  git('config', 'user.name', 'Test');
  git('config', 'user.email', 'test@example.com');
  writeFileSync(path.join(root, 'answer.txt'), 'before\n');
  git('add', '.');
  git('commit', '-m', 'Initial');
  return { root, git };
}
it('captures real staged/unstaged bytes and HEAD while omitting untracked files and credential paths', async () => {
  const { root, git } = repo();
  writeFileSync(path.join(root, '.env'), 'EXAMPLE=before\n');
  git('add', '.env');
  git('commit', '-m', 'Fixture credential');
  writeFileSync(path.join(root, '.env'), 'EXAMPLE=secret-value\n');
  writeFileSync(path.join(root, 'answer.txt'), 'staged\n');
  git('add', 'answer.txt');
  writeFileSync(path.join(root, 'answer.txt'), 'actual working result\n');
  writeFileSync(path.join(root, 'untracked.txt'), 'private untracked bytes');
  const result = await captureIntentGit(root);
  expect(result.headCommit).toBe(git('rev-parse', 'HEAD'));
  expect(result.artifact).toContain('+actual working result');
  expect(result.artifact).not.toContain('secret-value');
  expect(result.artifact).not.toContain('private untracked bytes');
  expect(result.omissions.join('\n')).toContain('Untracked file content omitted');
  expect(result.omissions.join('\n')).toContain('Restricted');
  expect(result.changedFiles).toEqual(['answer.txt']);
  expect(result.scope).toBe('tracked-working-tree-against-head');
});
it('does not expand an execution subdirectory to sibling file contents', async () => {
  const { root, git } = repo();
  mkdirSync(path.join(root, 'frontend'));
  mkdirSync(path.join(root, 'backend'));
  writeFileSync(path.join(root, 'frontend/view.txt'), 'old view\n');
  writeFileSync(path.join(root, 'backend/private.txt'), 'old backend\n');
  git('add', '.');
  git('commit', '-m', 'Subtrees');
  writeFileSync(path.join(root, 'frontend/view.txt'), 'new view\n');
  writeFileSync(path.join(root, 'backend/private.txt'), 'sibling secret\n');
  const result = await captureIntentGit(path.join(root, 'frontend'));
  expect(result.repositoryRoot).toBe(root);
  expect(result.artifact).toContain('+new view');
  expect(result.artifact).not.toContain('sibling secret');
  expect(result.changedFiles).toEqual(['frontend/view.txt']);
});
it('fails closed on absent repositories, unborn HEAD, and oversized artifacts', async () => {
  const { root } = repo();
  await expect(captureIntentGit(path.join(root, 'missing'))).rejects.toThrow();
  await expect(captureIntentGit('relative')).rejects.toThrow('host execution');
  writeFileSync(path.join(root, 'answer.txt'), 'x'.repeat(INTENT_CAPTURE_LIMITS.bytes + 100));
  await expect(captureIntentGit(root)).rejects.toThrow();
  const empty = mkdtempSync(path.join(tmpdir(), 'intent-git-empty-'));
  roots.push(empty);
  execFileSync('git', ['init'], { cwd: empty, stdio: 'ignore' });
  await expect(captureIntentGit(empty)).rejects.toThrow();
});
it('disables repository fsmonitor, external diff and text conversion commands', async () => {
  const { root, git } = repo();
  const marker = path.join(root, 'executed');
  const command = `sh -c 'echo unsafe > ${marker}'`;
  git('config', 'core.fsmonitor', command);
  git('config', 'diff.external', command);
  git('config', 'diff.custom.textconv', command);
  git('config', 'filter.custom.clean', command);
  git('config', 'filter.custom.process', command);
  git('config', 'filter.custom.required', 'true');
  writeFileSync(path.join(root, '.gitattributes'), 'answer.txt diff=custom filter=custom\n');
  writeFileSync(path.join(root, 'answer.txt'), 'new\n');
  const result = await captureIntentGit(root);
  expect(result.artifact).toContain('+new');
  expect(() => readFileSync(marker)).toThrow();
});
it('captures a tracked symlink as link text without reading its target', async () => {
  const { root, git } = repo();
  symlinkSync('/etc/passwd', path.join(root, 'link'));
  git('add', 'link');
  git('commit', '-m', 'Symlink');
  rmSync(path.join(root, 'link'));
  symlinkSync('/etc/shadow', path.join(root, 'link'));
  const result = await captureIntentGit(root);
  expect(result.artifact).toContain('+/etc/shadow');
  expect(result.artifact).not.toContain('root:');
});

it('never loads a late-added executable filter from mutable repository metadata', async () => {
  const { root, git } = repo();
  const marker = path.join(root, 'executed-late');
  const write = fs.writeFileSync.bind(fs);
  let changed = false;
  write(path.join(root, 'answer.txt'), 'changed\n');
  vi.spyOn(fs, 'writeFileSync').mockImplementation(((
    filename: Parameters<typeof fs.writeFileSync>[0],
    ...args: unknown[]
  ) => {
    if (
      !changed &&
      String(filename).includes('workspacer-intent-git-') &&
      String(filename).endsWith('/HEAD')
    ) {
      changed = true;
      git('config', 'filter.late.clean', `sh -c 'echo executed > ${marker}; cat'`);
      write(path.join(root, '.gitattributes'), 'answer.txt filter=late\n');
    }
    return (write as (...values: unknown[]) => void)(filename, ...args);
  }) as typeof fs.writeFileSync);
  const result = await captureIntentGit(root);
  expect(changed).toBe(true);
  expect(result.artifact).toContain('+changed');
  expect(() => readFileSync(marker)).toThrow();
});

it('ignores inherited repository and config override variables', async () => {
  const { root } = repo();
  writeFileSync(path.join(root, 'answer.txt'), 'host bytes\n');
  vi.stubEnv('GIT_DIR', '/wrong');
  vi.stubEnv('GIT_WORK_TREE', '/wrong');
  vi.stubEnv('GIT_INDEX_FILE', '/wrong');
  vi.stubEnv('GIT_CONFIG_COUNT', '1');
  vi.stubEnv('GIT_CONFIG_KEY_0', 'core.bare');
  vi.stubEnv('GIT_CONFIG_VALUE_0', 'true');
  expect((await captureIntentGit(root)).artifact).toContain('+host bytes');
});

it('accepts a differently spelled execution scope only when its directory identity matches, preserving subtree confinement', async () => {
  const { root, git } = repo();
  mkdirSync(path.join(root, 'frontend'));
  writeFileSync(path.join(root, 'frontend/view.txt'), 'before\n');
  git('add', '.');
  git('commit', '-m', 'Subdirectory');
  writeFileSync(path.join(root, 'frontend/view.txt'), 'after\n');
  writeFileSync(path.join(root, 'answer.txt'), 'outside execution\n');
  const alias = path.join(root, 'execution-alias');
  symlinkSync(
    path.join(root, 'frontend'),
    alias,
    process.platform === 'win32' ? 'junction' : 'dir',
  );
  const realpath = fs.realpathSync.bind(fs);
  // Reproduce the runner's Node/Git spelling disagreement without changing
  // platform path semantics: Node retains the alias; Git returns the real root.
  vi.spyOn(fs, 'realpathSync').mockImplementation(((value: fs.PathLike, options?: unknown) =>
    String(value) === alias
      ? alias
      : (realpath as (...args: unknown[]) => unknown)(value, options)) as typeof fs.realpathSync);
  const captured = await captureIntentGit(alias);
  expect(captured.cwd).toBe(alias);
  expect(captured.repositoryRoot).toBe(root);
  expect(captured.changedFiles).toEqual(['frontend/view.txt']);
  expect(captured.artifact).toContain('+after');
  expect(captured.artifact).not.toContain('outside execution');
});

it('refuses an alias whose physical identity differs from Git scope or changes during capture', async () => {
  const { root } = repo();
  writeFileSync(path.join(root, 'answer.txt'), 'after\n');
  const alias = path.join(root, 'execution-alias');
  const outside = path.join(root, 'other-directory');
  mkdirSync(outside);
  symlinkSync(root, alias, process.platform === 'win32' ? 'junction' : 'dir');
  const realpath = fs.realpathSync.bind(fs),
    stat = fs.statSync.bind(fs);
  const foreign = stat(outside, { bigint: true });
  vi.spyOn(fs, 'realpathSync').mockImplementation(((value: fs.PathLike, options?: unknown) =>
    String(value) === alias
      ? alias
      : (realpath as (...args: unknown[]) => unknown)(value, options)) as typeof fs.realpathSync);
  let scopeReads = 0;
  let mismatchAt = 1;
  vi.spyOn(fs, 'statSync').mockImplementation(((
    value: fs.PathLike,
    options?: { bigint?: boolean },
  ) => {
    if (String(value) === root && options?.bigint && ++scopeReads >= mismatchAt) return foreign;
    return (stat as (...args: unknown[]) => unknown)(value, options);
  }) as typeof fs.statSync);
  await expect(captureIntentGit(alias)).rejects.toThrow('identity does not match');
  scopeReads = 0;
  mismatchAt = 2;
  await expect(captureIntentGit(alias)).rejects.toThrow('Execution changed');
});
