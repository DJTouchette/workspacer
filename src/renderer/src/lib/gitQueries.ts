/**
 * Thin client for claudemon's git API, used by the review pane: read-only
 * status/diff plus the staging actions (stage/unstage/commit/push).
 * Endpoints implemented at claudemon/src/daemon/git.rs.
 */

export interface FileStatus {
  path: string;
  /** Set only for renames/copies: the original path. */
  orig_path?: string;
  /** Porcelain X code (staged / index status): "M" "A" "D" "R" "?" " " ... */
  staged: string;
  /** Porcelain Y code (unstaged / work-tree status). */
  unstaged: string;
}

export interface GitStatus {
  branch: string | null;
  files: FileStatus[];
}

/** Per-file added/deleted line counts. Null counts mean a binary file. */
export interface NumstatEntry {
  path: string;
  added: number | null;
  deleted: number | null;
}

import { CLAUDEMON_API_BASE } from './claudemonBase';

export class GitClient {
  constructor(private readonly baseUrl: string = CLAUDEMON_API_BASE) {}

  async status(cwd: string): Promise<GitStatus> {
    const url = `${this.baseUrl}/git/status?cwd=${encodeURIComponent(cwd)}`;
    const res = await fetch(url);
    if (!res.ok) throw new Error(`git status failed: ${res.status} ${await res.text()}`);
    return (await res.json()) as GitStatus;
  }

  /**
   * Unified diff text for a single file (or the whole work tree if `path` is
   * omitted). `staged` selects index-vs-HEAD instead of work-tree-vs-index.
   * `untracked` renders an untracked file as an all-added diff.
   */
  async diff(cwd: string, path?: string, staged = false, untracked = false): Promise<string> {
    const params = new URLSearchParams({ cwd });
    if (path) params.set('path', path);
    if (staged) params.set('staged', 'true');
    if (untracked) params.set('untracked', 'true');
    const res = await fetch(`${this.baseUrl}/git/diff?${params.toString()}`);
    if (!res.ok) throw new Error(`git diff failed: ${res.status} ${await res.text()}`);
    const body = (await res.json()) as { diff: string };
    return body.diff;
  }

  /** Added/deleted line counts per changed file (`git diff --numstat`). */
  async numstat(cwd: string, staged = false): Promise<NumstatEntry[]> {
    const params = new URLSearchParams({ cwd });
    if (staged) params.set('staged', 'true');
    const res = await fetch(`${this.baseUrl}/git/numstat?${params.toString()}`);
    if (!res.ok) throw new Error(`git numstat failed: ${res.status} ${await res.text()}`);
    const body = (await res.json()) as { files: NumstatEntry[] };
    return body.files;
  }

  // ── Mutating actions ──
  //
  // Each posts a JSON body and expects `{ ok, output?, error? }`. The daemon
  // returns 422 with git's stderr in `error` for the expected failures
  // (nothing staged, no upstream, conflicts), which we surface verbatim.

  private async post(path: string, body: Record<string, unknown>): Promise<string> {
    const res = await fetch(`${this.baseUrl}${path}`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body),
    });
    const data = (await res.json().catch(() => null)) as
      | { ok?: boolean; output?: string; error?: string }
      | null;
    if (!res.ok || !data?.ok) {
      throw new Error(data?.error?.trim() || `${path} failed: ${res.status}`);
    }
    return data.output ?? '';
  }

  /** Stage a single path, or the whole work tree when `path` is omitted. */
  stage(cwd: string, path?: string): Promise<string> {
    return this.post('/git/stage', { cwd, path });
  }

  /** Unstage a single path, or everything when `path` is omitted. */
  unstage(cwd: string, path?: string): Promise<string> {
    return this.post('/git/unstage', { cwd, path });
  }

  commit(cwd: string, message: string): Promise<string> {
    return this.post('/git/commit', { cwd, message });
  }

  push(cwd: string): Promise<string> {
    return this.post('/git/push', { cwd });
  }
}
