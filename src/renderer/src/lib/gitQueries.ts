/**
 * Thin client for claudemon's read-only git API, used by the review pane.
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

const DEFAULT_BASE = 'http://127.0.0.1:7891';

export class GitClient {
  constructor(private readonly baseUrl: string = DEFAULT_BASE) {}

  async status(cwd: string): Promise<GitStatus> {
    const url = `${this.baseUrl}/git/status?cwd=${encodeURIComponent(cwd)}`;
    const res = await fetch(url);
    if (!res.ok) throw new Error(`git status failed: ${res.status} ${await res.text()}`);
    return (await res.json()) as GitStatus;
  }

  /**
   * Unified diff text for a single file (or the whole work tree if `path` is
   * omitted). `staged` selects index-vs-HEAD instead of work-tree-vs-index.
   */
  async diff(cwd: string, path?: string, staged = false): Promise<string> {
    const params = new URLSearchParams({ cwd });
    if (path) params.set('path', path);
    if (staged) params.set('staged', 'true');
    const res = await fetch(`${this.baseUrl}/git/diff?${params.toString()}`);
    if (!res.ok) throw new Error(`git diff failed: ${res.status} ${await res.text()}`);
    const body = (await res.json()) as { diff: string };
    return body.diff;
  }
}
