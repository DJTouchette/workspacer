/**
 * Git inspection and staging for the review pane — a host-side capability,
 * exposed on the hub bus (`git.*`) and over IPC (`GIT_*`). This replaces the
 * old `/git/*` HTTP surface that lived in claudemon (services/claudemon/src/
 * daemon/git.rs); the agent daemon no longer touches git.
 *
 * Reads (keyed off a `cwd` — the renderer passes the active agent's working
 * directory): status (branch + changed files), diff (raw unified text), numstat
 * (per-file line counts). Writes: stage / unstage / commit / push.
 *
 * Everything shells out to the `git` binary. Before running anything we resolve
 * `cwd` to its work-tree root with `rev-parse --show-toplevel`, so an arbitrary
 * path can't turn these into a generic "run git anywhere" surface, and so the
 * status/diff/add path conventions all agree (see workRoot below).
 */

import { execFile } from 'child_process';

/** One changed file as reported by `git status --porcelain`. `staged` and
 *  `unstaged` are the porcelain XY codes (e.g. "M", "A", "D", "?", " "). */
export interface FileStatus {
  path: string;
  /** Set only for renames/copies: the original path. */
  orig_path?: string;
  staged: string;
  unstaged: string;
}

export interface GitStatus {
  branch: string | null;
  files: FileStatus[];
}

/** One row of `git diff --numstat`: lines added/deleted per file. `null` counts
 *  mean a binary file (numstat prints `-` for those). */
export interface NumstatEntry {
  path: string;
  added: number | null;
  deleted: number | null;
}

/** Cap on a single git command's stdout. Diffs of a whole work tree can be
 *  large; the renderer gates rendering past ~1.5 MB but still receives the full
 *  text, so allow generous headroom before truncation would corrupt a diff. */
const MAX_BUFFER = 256 * 1024 * 1024;

/** Run `git` in `cwd` with `args`, returning (ok, stdout, stderr). Never
 *  rejects on a non-zero git exit — callers decide what a failure means
 *  (a read fails the request; a mutating action surfaces git's stderr). */
function runGit(
  cwd: string,
  args: string[],
): Promise<{ ok: boolean; stdout: string; stderr: string }> {
  return new Promise((resolve, reject) => {
    execFile(
      'git',
      args,
      { cwd, maxBuffer: MAX_BUFFER, windowsHide: true },
      (err, stdout, stderr) => {
        if (err && (err as NodeJS.ErrnoException).code === 'ENOENT') {
          // git not on PATH — distinct from a non-zero git exit.
          reject(new Error('could not run git (is it installed and on PATH?)'));
          return;
        }
        resolve({ ok: !err, stdout: stdout ?? '', stderr: stderr ?? '' });
      },
    );
  });
}

/** Resolve `cwd` to its git work-tree root, or `null` when `cwd` isn't inside
 *  one. Every git command below runs from this root rather than `cwd` itself,
 *  because `git status`/`diff --numstat` emit *repo-root-relative* paths while
 *  `git diff`/`add` interpret a pathspec relative to the *current directory*.
 *  Run from a subdirectory those two conventions disagree, so a root-relative
 *  path silently matches nothing and the diff comes back empty. Anchoring at
 *  the root keeps both ends speaking the same path language. */
async function workRoot(cwd: string): Promise<string | null> {
  const { ok, stdout } = await runGit(cwd, ['rev-parse', '--show-toplevel']);
  if (!ok) return null;
  const root = stdout.trim();
  return root ? root : null;
}

/** Parse `git status --porcelain -z` output into structured rows.
 *
 *  Each entry is `XY <path>` terminated by NUL, where X is the staged (index)
 *  status and Y the unstaged (work tree) status. The `-z` format never quotes
 *  or escapes paths (unlike the default, which wraps unusual paths in `"…"`).
 *  For a rename/copy the destination path is this entry and the original path
 *  is the *next* NUL-terminated token. */
export function parsePorcelain(stdout: string): FileStatus[] {
  const files: FileStatus[] = [];
  const tokens = stdout.split('\0');
  for (let i = 0; i < tokens.length; i++) {
    const entry = tokens[i];
    // Need at least "XY <path>" — two status chars, a space, then a path.
    if (entry.length < 4) continue;
    const staged = entry[0];
    const unstaged = entry[1];
    const path = entry.slice(3);

    // Rename/copy: the source path follows as a separate token (no " -> ").
    const isMove = staged === 'R' || staged === 'C' || unstaged === 'R' || unstaged === 'C';
    const orig_path = isMove ? tokens[++i] : undefined;

    files.push({ path, orig_path, staged, unstaged });
  }
  return files;
}

/** Resolve a numstat path to the *new* name. Renames appear either as
 *  `old => new` or in brace form `prefix/{old => new}/suffix`. */
export function parseNumstatPath(raw: string): string {
  const open = raw.indexOf('{');
  const close = raw.indexOf('}');
  if (open !== -1 && close !== -1 && open < close) {
    const inner = raw.slice(open + 1, close);
    const arrow = inner.indexOf(' => ');
    if (arrow !== -1) {
      const next = inner.slice(arrow + 4);
      const joined = raw.slice(0, open) + next + raw.slice(close + 1);
      // An empty side ("{ => sub}") leaves a doubled separator behind.
      return joined.replace('//', '/');
    }
  }
  const arrow = raw.indexOf(' => ');
  return arrow !== -1 ? raw.slice(arrow + 4) : raw;
}

export function parseNumstat(stdout: string): NumstatEntry[] {
  const out: NumstatEntry[] = [];
  for (const line of stdout.split('\n')) {
    if (!line) continue;
    const parts = line.split('\t');
    if (parts.length < 3) continue;
    const [added, deleted, ...rest] = parts;
    const rawPath = rest.join('\t').replace(/\r$/, '');
    const a = parseInt(added, 10);
    const d = parseInt(deleted, 10);
    out.push({
      path: parseNumstatPath(rawPath),
      added: Number.isNaN(a) ? null : a,
      deleted: Number.isNaN(d) ? null : d,
    });
  }
  return out;
}

/** Resolve the work root or throw the same message the daemon used to 400 with. */
async function rootOrThrow(cwd: string): Promise<string> {
  const root = await workRoot(cwd);
  if (!root) throw new Error('cwd is not inside a git work tree');
  return root;
}

export async function status(cwd: string): Promise<GitStatus> {
  const root = await rootOrThrow(cwd);

  // `--untracked-files=all` lists every untracked file individually. Without
  // it, git collapses a fully-untracked directory into one `dir/` entry, and
  // the review pane would then ask for an untracked diff of a directory.
  const res = await runGit(root, ['status', '--porcelain', '-z', '--untracked-files=all']);
  if (!res.ok) throw new Error(res.stderr.trim() || 'git status failed');
  const files = parsePorcelain(res.stdout);

  // Branch name is best-effort: a detached HEAD or fresh repo may not have one.
  let branch: string | null = null;
  const b = await runGit(root, ['rev-parse', '--abbrev-ref', 'HEAD']);
  if (b.ok) {
    const name = b.stdout.trim();
    if (name && name !== 'HEAD') branch = name;
  }

  return { branch, files };
}

/** Unified diff text for a single file (or the whole work tree if `path` is
 *  omitted). `staged` selects index-vs-HEAD; `untracked` renders an untracked
 *  file as an all-added diff via `--no-index`. */
export async function diff(
  cwd: string,
  path?: string,
  staged = false,
  untracked = false,
): Promise<string> {
  const root = await rootOrThrow(cwd);

  if (untracked) {
    if (!path) throw new Error('untracked diff requires a path');
    // A directory has no single-file diff: `git diff --no-index /dev/null dir/`
    // makes git hunt for `dir/null` and fail. Status uses --untracked-files=all
    // so this shouldn't happen, but guard rather than emit a confusing error.
    if (path.endsWith('/') || path.endsWith('\\')) {
      throw new Error('untracked diff path is a directory');
    }
    // `--no-index` exits 1 when the files differ — the expected case here — so
    // success is "produced output", not "exit 0". git special-cases the literal
    // "/dev/null" on every platform, including Windows.
    const res = await runGit(root, ['diff', '--no-index', '--', '/dev/null', path]);
    if (res.ok || res.stdout) return res.stdout;
    throw new Error(res.stderr.trim() || 'git diff failed');
  }

  const args = ['diff'];
  if (staged) args.push('--staged');
  // `--` separates pathspecs from revisions so a file named like a flag can't
  // be misread as one.
  if (path) args.push('--', path);

  const res = await runGit(root, args);
  if (!res.ok) throw new Error(res.stderr.trim() || 'git diff failed');
  return res.stdout;
}

/** Added/deleted line counts per changed file (`git diff --numstat`). */
export async function numstat(cwd: string, staged = false): Promise<NumstatEntry[]> {
  const root = await rootOrThrow(cwd);
  // `core.quotepath=false` keeps unicode paths unquoted so they match the
  // (NUL-unquoted) paths from `git status`.
  const args = ['-c', 'core.quotepath=false', 'diff', '--numstat'];
  if (staged) args.push('--staged');
  const res = await runGit(root, args);
  if (!res.ok) throw new Error(res.stderr.trim() || 'git diff --numstat failed');
  return parseNumstat(res.stdout);
}

/** Run a mutating git command from the work root, returning git's stdout. On a
 *  non-zero exit we throw git's stderr so the renderer can surface the real
 *  reason (nothing staged, no upstream, merge conflict, …). */
async function action(cwd: string, args: string[]): Promise<string> {
  const root = await rootOrThrow(cwd);
  const res = await runGit(root, args);
  if (!res.ok) throw new Error(res.stderr.trim() || 'git command failed');
  return res.stdout;
}

/** Stage a single path, or the whole work tree (`-A`) when `path` is omitted. */
export function stage(cwd: string, path?: string): Promise<string> {
  // `--` keeps a path that looks like a flag from being parsed as one.
  return action(cwd, path ? ['add', '--', path] : ['add', '-A']);
}

/** Unstage a single path, or everything, leaving the work tree untouched. */
export function unstage(cwd: string, path?: string): Promise<string> {
  const args = ['reset', '-q', 'HEAD'];
  if (path) args.push('--', path);
  return action(cwd, args);
}

export function commit(cwd: string, message: string): Promise<string> {
  if (!message.trim()) throw new Error('empty commit message');
  return action(cwd, ['commit', '-m', message]);
}

export function push(cwd: string): Promise<string> {
  return action(cwd, ['push']);
}
