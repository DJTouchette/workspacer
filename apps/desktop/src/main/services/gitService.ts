/**
 * Git inspection and staging for the review pane — a host-side capability,
 * exposed on the hub bus (`git.*`) and over IPC (`GIT_*`). This replaces the
 * old `/git/*` HTTP surface that lived in claudemon (services/claudemon/src/
 * daemon/git.rs); the agent daemon no longer touches git.
 *
 * Reads (keyed off a `cwd` — the renderer passes the active agent's working
 * directory): status (branch + changed files), diff (raw unified text), numstat
 * (per-file line counts), log (recent commits). Writes: stage / unstage /
 * commit / push.
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
  /** Upstream tracking branch (e.g. "origin/master"), or null when none is
   *  configured (or it's gone). */
  upstream: string | null;
  /** Commits ahead of / behind the upstream. Both 0 when no upstream. */
  ahead: number;
  behind: number;
}

/** One row of `git diff --numstat`: lines added/deleted per file. `null` counts
 *  mean a binary file (numstat prints `-` for those). */
export interface NumstatEntry {
  path: string;
  added: number | null;
  deleted: number | null;
}

/** One commit from `git log` — enough for a "what was I doing here?" peek. */
export interface LogEntry {
  hash: string;
  subject: string;
  /** Author time, unix seconds. */
  authoredAt: number;
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

/** Parse the `--branch` header of porcelain status (`## master...origin/master
 *  [ahead 1, behind 2]`). Variants: no upstream (`## master`), a gone upstream
 *  (`[gone]` — treated as none, plain `git push` can't reach it), detached
 *  (`## HEAD (no branch)`), and an unborn branch (`## No commits yet on x`). */
export function parseBranchHeader(header: string): {
  upstream: string | null;
  ahead: number;
  behind: number;
} {
  const none = { upstream: null, ahead: 0, behind: 0 };
  if (!header.startsWith('## ')) return none;
  const body = header.slice(3);
  const sep = body.indexOf('...');
  if (sep === -1) return none;
  let rest = body.slice(sep + 3);
  let ahead = 0;
  let behind = 0;
  const bracket = rest.indexOf(' [');
  if (bracket !== -1) {
    const inside = rest.slice(bracket + 2).replace(/\]$/, '');
    rest = rest.slice(0, bracket);
    if (inside === 'gone') return none;
    const a = /ahead (\d+)/.exec(inside);
    const b = /behind (\d+)/.exec(inside);
    if (a) ahead = parseInt(a[1], 10);
    if (b) behind = parseInt(b[1], 10);
  }
  return { upstream: rest || null, ahead, behind };
}

/** Parse `git log --pretty=format:%h%x00%s%x00%at` output (one commit per
 *  line, NUL-separated fields — subjects never contain newlines or NULs). */
export function parseLog(stdout: string): LogEntry[] {
  const out: LogEntry[] = [];
  for (const line of stdout.split('\n')) {
    if (!line) continue;
    const [hash, subject, at] = line.split('\0');
    if (!hash || subject === undefined || at === undefined) continue;
    const authoredAt = parseInt(at, 10);
    if (Number.isNaN(authoredAt)) continue;
    out.push({ hash, subject, authoredAt });
  }
  return out;
}

export function formatGitActionError(raw: string, fallback = 'git command failed'): string {
  const msg = raw.trim() || fallback;
  const lower = msg.toLowerCase();
  const withDetail = (summary: string) => `${summary}\n\n${msg}`;
  if (
    lower.includes('you have unmerged paths') ||
    lower.includes('fix conflicts') ||
    lower.includes('conflict (') ||
    lower.includes('merge conflict')
  ) {
    return withDetail(
      'Merge conflicts need resolution before this git action can continue. Resolve the conflicted files, stage them, then retry.',
    );
  }
  if (lower.includes('no changes added to commit') || lower.includes('nothing to commit')) {
    return withDetail('Nothing is staged to commit. Stage files in Review, then commit again.');
  }
  if (lower.includes('has no upstream branch') || lower.includes('no upstream branch')) {
    return withDetail(
      'No upstream branch is configured. Set an upstream with git push --set-upstream, then retry.',
    );
  }
  if (
    lower.includes('non-fast-forward') ||
    lower.includes('fetch first') ||
    lower.includes('updates were rejected') ||
    lower.includes('rejected')
  ) {
    return withDetail(
      'Push was rejected because the remote has changes this branch does not have. Pull or rebase, resolve anything needed, then retry.',
    );
  }
  return msg;
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
  // `--branch` prepends a `## …` header carrying upstream + ahead/behind,
  // which the review pane uses to grey out Push when there's nothing to push.
  const res = await runGit(root, [
    'status',
    '--porcelain',
    '-z',
    '--branch',
    '--untracked-files=all',
  ]);
  if (!res.ok) throw new Error(res.stderr.trim() || 'git status failed');

  // Split the branch header off before the file parser sees it.
  const nul = res.stdout.indexOf('\0');
  const header = res.stdout.startsWith('## ')
    ? res.stdout.slice(0, nul === -1 ? undefined : nul)
    : '';
  const body = header ? (nul === -1 ? '' : res.stdout.slice(nul + 1)) : res.stdout;
  const { upstream, ahead, behind } = parseBranchHeader(header);
  const files = parsePorcelain(body);

  // Branch name is best-effort: a detached HEAD or fresh repo may not have one.
  let branch: string | null = null;
  const b = await runGit(root, ['rev-parse', '--abbrev-ref', 'HEAD']);
  if (b.ok) {
    const name = b.stdout.trim();
    if (name && name !== 'HEAD') branch = name;
  }

  return { branch, files, upstream, ahead, behind };
}

/** Most recent commits, newest first (capped at 20). An empty repo — where
 *  `git log` exits non-zero because HEAD has no commits — yields []. */
export async function log(cwd: string, limit = 5): Promise<LogEntry[]> {
  const root = await rootOrThrow(cwd);
  const n = Math.max(1, Math.min(50, Math.floor(limit)));
  const res = await runGit(root, ['log', '-n', String(n), '--pretty=format:%h%x00%s%x00%at']);
  if (!res.ok) return [];
  return parseLog(res.stdout);
}

/** A commit ref we accept from callers: an abbreviated or full hex hash.
 *  Anything else is rejected up front — these reach `git` argv (and the hub
 *  capabilities expose them to remote/token clients), so no refnames, ranges,
 *  or option-shaped strings. */
function assertCommitHash(hash: string): string {
  const h = String(hash || '').trim();
  if (!/^[0-9a-f]{4,40}$/i.test(h)) throw new Error(`not a commit hash: ${hash}`);
  return h;
}

/** Unified diff of one commit against its parent (root commits diff against
 *  the empty tree — `git show` handles both). `path` narrows to one file. */
export async function commitDiff(cwd: string, hash: string, path?: string): Promise<string> {
  const root = await rootOrThrow(cwd);
  const h = assertCommitHash(hash);
  const args = ['show', '--format=', '--patch', h];
  if (path) args.push('--', path);
  const res = await runGit(root, args);
  if (!res.ok) throw new Error(res.stderr.trim() || 'git show failed');
  return res.stdout;
}

/** Per-file +/- counts for one commit (same parent baseline as commitDiff). */
export async function commitNumstat(cwd: string, hash: string): Promise<NumstatEntry[]> {
  const root = await rootOrThrow(cwd);
  const h = assertCommitHash(hash);
  const res = await runGit(root, ['show', '--format=', '--numstat', h]);
  if (!res.ok) throw new Error(res.stderr.trim() || 'git show failed');
  return parseNumstat(res.stdout);
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
  if (!res.ok) {
    const raw = [res.stderr, res.stdout]
      .map((s) => s.trim())
      .filter(Boolean)
      .join('\n');
    throw new Error(formatGitActionError(raw, `git ${args[0] ?? 'command'} failed`));
  }
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
