/**
 * Git-worktree support for agent spawns: give each agent its own working tree
 * so parallel agents in one repo can't trample each other, and everything
 * scoped to the agent's cwd (plugins, watchers, checks) is confined to that
 * tree.
 *
 * Worktrees are created under `agents.worktreeRoot` (default
 * `~/.workspacer/worktrees`) as `<repoName>/<slug>` on a fresh `wks/<slug>`
 * branch cut from the repo's current HEAD. They are NEVER deleted
 * automatically — a worktree may hold uncommitted work; cleanup is a
 * deliberate `git worktree remove` (or `git worktree prune` after deleting
 * the folder).
 */

import { execFile } from 'child_process';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';

export interface WorktreeInfo {
  /** True when the directory is inside a git work tree. */
  isRepo: boolean;
  /** Repo top-level (the main working tree root), when isRepo. */
  root?: string;
  /** Current branch name (or short HEAD sha when detached), when isRepo. */
  branch?: string;
}

export interface WorktreeCreateResult {
  ok: boolean;
  /** Absolute path of the new worktree (the agent's cwd), when ok. */
  path?: string;
  /** The branch the worktree was created on, when ok. */
  branch?: string;
  error?: string;
}

function git(
  args: string[],
  cwd: string,
): Promise<{ ok: boolean; stdout: string; stderr: string }> {
  return new Promise((resolve) => {
    execFile('git', args, { cwd, timeout: 15_000 }, (err, stdout, stderr) => {
      resolve({ ok: !err, stdout: (stdout ?? '').trim(), stderr: (stderr ?? '').trim() });
    });
  });
}

/** Default parent directory for agent worktrees. */
export function defaultWorktreeRoot(): string {
  return path.join(os.homedir(), '.workspacer', 'worktrees');
}

/** Is `cwd` a git repo, and if so where/what branch? Never throws. */
export async function worktreeInfo(cwd: string): Promise<WorktreeInfo> {
  if (!cwd || !fs.existsSync(cwd)) return { isRepo: false };
  const top = await git(['rev-parse', '--show-toplevel'], cwd);
  if (!top.ok || !top.stdout) return { isRepo: false };
  const branch = await git(['rev-parse', '--abbrev-ref', 'HEAD'], cwd);
  let name = branch.ok ? branch.stdout : '';
  if (name === 'HEAD') {
    const sha = await git(['rev-parse', '--short', 'HEAD'], cwd);
    name = sha.ok ? sha.stdout : '';
  }
  return { isRepo: true, root: top.stdout, branch: name || undefined };
}

/** Filesystem/branch-safe slug from an agent name. */
function slugify(name: string): string {
  const s = name
    .toLowerCase()
    .replace(/[^a-z0-9._-]+/g, '-')
    .replace(/^[-.]+|[-.]+$/g, '')
    .slice(0, 40);
  return s || 'agent';
}

/** A short random suffix so repeated spawns with the same name never collide. */
function shortId(): string {
  return Math.random().toString(36).slice(2, 6);
}

/**
 * Create a worktree for an agent spawn. `name` seeds the folder + branch slug
 * (typically the agent's display name); `rootOverride` is the configured
 * `agents.worktreeRoot` ('' = default).
 */
export async function createWorktree(opts: {
  repoCwd: string;
  name?: string;
  rootOverride?: string;
}): Promise<WorktreeCreateResult> {
  const info = await worktreeInfo(opts.repoCwd);
  if (!info.isRepo || !info.root) {
    return { ok: false, error: `${opts.repoCwd} is not inside a git repository` };
  }

  const parent = path.join(
    opts.rootOverride?.trim() ? path.resolve(opts.rootOverride.trim()) : defaultWorktreeRoot(),
    path.basename(info.root),
  );

  const slug = slugify(opts.name ?? '');
  // Prefer the clean slug; disambiguate with a short suffix on collision (an
  // existing dir OR an existing branch — git refuses both).
  for (let attempt = 0; attempt < 3; attempt++) {
    const candidate = attempt === 0 ? slug : `${slug}-${shortId()}`;
    const wtPath = path.join(parent, candidate);
    const branch = `wks/${candidate}`;
    if (fs.existsSync(wtPath)) continue;
    try {
      await fs.promises.mkdir(parent, { recursive: true });
    } catch (err) {
      return { ok: false, error: `cannot create ${parent}: ${(err as Error).message}` };
    }
    const res = await git(['worktree', 'add', '-b', branch, wtPath], info.root);
    if (res.ok) {
      console.log(`[worktree] created ${wtPath} (${branch}) from ${info.root}`);
      return { ok: true, path: wtPath, branch };
    }
    // Branch collision → retry with a suffix; anything else is terminal.
    if (!/already exists/i.test(res.stderr)) {
      return { ok: false, error: res.stderr || 'git worktree add failed' };
    }
  }
  return { ok: false, error: 'could not find a free worktree name (tried 3 candidates)' };
}
