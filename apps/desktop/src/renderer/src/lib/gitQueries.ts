/**
 * Thin client for the host's git surface, used by the review pane: read-only
 * status/diff/numstat plus the staging actions (stage/unstage/commit/push).
 *
 * Git is a host capability exposed on `window.electronAPI` — over preload IPC
 * on the desktop, or the hub bus on the web/remote mirror (and desktop bus
 * mode). The backend (`apps/desktop/src/main/services/gitService.ts`) shells
 * out to `git`. A failed git command rejects the underlying call; we surface
 * its message verbatim.
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
  /** Upstream tracking branch ("origin/master"), null when none/gone.
   *  Optional: an older host over the hub bus may omit it. */
  upstream?: string | null;
  /** Commits ahead of / behind the upstream; both 0 when no upstream. */
  ahead?: number;
  behind?: number;
}

/** Per-file added/deleted line counts. Null counts mean a binary file. */
export interface NumstatEntry {
  path: string;
  added: number | null;
  deleted: number | null;
}

export class GitClient {
  async status(cwd: string): Promise<GitStatus> {
    return window.electronAPI.gitStatus(cwd);
  }

  /**
   * Unified diff text for a single file (or the whole work tree if `path` is
   * omitted). `staged` selects index-vs-HEAD instead of work-tree-vs-index.
   * `untracked` renders an untracked file as an all-added diff.
   */
  async diff(cwd: string, path?: string, staged = false, untracked = false): Promise<string> {
    return window.electronAPI.gitDiff(cwd, path, staged, untracked);
  }

  /** Added/deleted line counts per changed file (`git diff --numstat`). */
  async numstat(cwd: string, staged = false): Promise<NumstatEntry[]> {
    return window.electronAPI.gitNumstat(cwd, staged);
  }

  // ── Mutating actions ──
  //
  // Each rejects with git's stderr on the expected failures (nothing staged,
  // no upstream, conflicts), which the review pane surfaces verbatim.

  /** Stage a single path, or the whole work tree when `path` is omitted. */
  stage(cwd: string, path?: string): Promise<string> {
    return window.electronAPI.gitStage(cwd, path);
  }

  /** Unstage a single path, or everything when `path` is omitted. */
  unstage(cwd: string, path?: string): Promise<string> {
    return window.electronAPI.gitUnstage(cwd, path);
  }

  commit(cwd: string, message: string): Promise<string> {
    return window.electronAPI.gitCommit(cwd, message);
  }

  push(cwd: string): Promise<string> {
    return window.electronAPI.gitPush(cwd);
  }
}
