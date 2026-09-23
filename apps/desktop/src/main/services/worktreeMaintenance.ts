import * as fs from 'fs/promises';
import * as path from 'path';
import { randomUUID } from 'crypto';

export const WORKTREE_MAINTENANCE_LOCK = '.workspacer-maintenance.lock';
export const WORKTREE_ALLOCATION = 'workspacer-allocation.json';

/** Same ancestor/file rule as claudemon's worktree admission guard. */
export async function linkedWorktree(cwd: string): Promise<{ cwd: string; gitDir: string } | null> {
  let dir = await fs.realpath(cwd).catch(() => path.resolve(cwd));
  for (;;) {
    const marker = path.join(dir, '.git');
    const stat = await fs.lstat(marker).catch(() => null);
    if (stat) {
      if (!stat.isFile() || stat.isSymbolicLink()) return null;
      const match = (await fs.readFile(marker, 'utf8')).match(/^gitdir:\s*(.+?)\s*$/);
      if (!match) return null;
      const gitDir = await fs.realpath(path.resolve(dir, match[1]));
      return (await fs.stat(gitDir)).isDirectory() ? { cwd: dir, gitDir } : null;
    }
    const parent = path.dirname(dir);
    if (parent === dir) return null;
    dir = parent;
  }
}

/** Cross-process exclusive lease shared with daemon spawn admission. Never
 * steal by age: a slow cleanup must not race a newly admitted provider. */
export async function withWorktreeMaintenance<T>(
  gitDir: string,
  run: () => Promise<T>,
): Promise<T> {
  const lock = path.join(gitDir, WORKTREE_MAINTENANCE_LOCK);
  const owner = JSON.stringify({ pid: process.pid, token: randomUUID() });
  const file = await fs.open(lock, 'wx', 0o600).catch((error) => {
    if (error.code === 'EEXIST')
      throw new Error(
        'Worktree maintenance or another launch is in progress; retry after it finishes',
      );
    throw error;
  });
  try {
    await file.writeFile(owner);
  } catch (error) {
    await file.close();
    await fs.unlink(lock).catch(() => {});
    throw error;
  }
  await file.close();
  try {
    return await run();
  } finally {
    // Do not remove another owner's lock after external replacement.
    if ((await fs.readFile(lock, 'utf8').catch(() => '')) === owner)
      await fs.unlink(lock).catch(() => {});
  }
}

export async function recordWorktreeAllocation(cwd: string, root: string): Promise<void> {
  const linked = await linkedWorktree(cwd);
  if (!linked) throw new Error('Allocated directory is not a linked worktree');
  const stat = await fs.stat(linked.cwd);
  await fs.writeFile(
    path.join(linked.gitDir, WORKTREE_ALLOCATION),
    JSON.stringify({
      version: 1,
      cwd: linked.cwd,
      root: await fs.realpath(root),
      dev: stat.dev,
      ino: stat.ino,
      createdAt: new Date().toISOString(),
    }),
    { flag: 'wx', mode: 0o600 },
  );
}
