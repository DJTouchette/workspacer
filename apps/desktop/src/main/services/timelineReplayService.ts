/**
 * Worktree replay for the transcript timeline: materializes a session's file
 * edits into a disposable git worktree so a replay UI can scrub real files
 * through time without ever touching the agent's working tree.
 *
 * `open` resolves a base commit (the repo as it stood when the session
 * started, best-effort by timestamp) and checks it out detached into a
 * worktree under the OS temp dir. `seek` resets that worktree to the base and
 * re-applies the session's Write/Edit/MultiEdit tool calls up to the scrub
 * position. `close` removes the worktree.
 *
 * Containment: every write lands inside a worktree this service itself
 * created; an op whose path escapes the repository root is skipped, never
 * applied. The agent's real checkout is only read (worktree add/remove).
 */

import { execFile } from 'child_process';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';

/** One file-mutating tool call from the transcript, as the replay UI sends it. */
export interface ReplayOp {
  name: string; // Write | Edit | MultiEdit (others are skipped)
  input: {
    file_path?: string;
    content?: string;
    old_string?: string;
    new_string?: string;
    replace_all?: boolean;
    edits?: { old_string?: string; new_string?: string; replace_all?: boolean }[];
  };
}

export interface SkippedOp {
  path: string;
  op: string;
  reason: string;
}

export interface SeekResult {
  applied: number;
  skipped: SkippedOp[];
  /** Paths changed vs the base commit after this seek (porcelain rows). */
  changedFiles: number;
}

interface ReplayEntry {
  /** The real repository's work-tree root (where `worktree add` runs). */
  root: string;
  /** The disposable worktree this service created. */
  dir: string;
  baseCommit: string;
}

const MAX_BUFFER = 64 * 1024 * 1024;

/** Run `git` in `cwd`, resolving (never rejecting) on a non-zero exit —
 *  callers decide what a failure means. Mirrors gitService's runner. */
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
          reject(new Error('could not run git (is it installed and on PATH?)'));
          return;
        }
        resolve({ ok: !err, stdout: stdout ?? '', stderr: stderr ?? '' });
      },
    );
  });
}

/** Same shape claudemon enforces for session ids — the id becomes a directory
 *  name, so refuse anything traversal-shaped before it touches a path. */
function validSessionId(id: string): boolean {
  return id.length > 0 && id.length <= 128 && !id.includes('..') && /^[A-Za-z0-9._-]+$/.test(id);
}

class TimelineReplayService {
  private entries = new Map<string, ReplayEntry>();

  /** Root under which every replay worktree lives. */
  private replayRoot(): string {
    return path.join(os.tmpdir(), 'workspacer-replay');
  }

  /**
   * Create (or reuse) the replay worktree for a session. `beforeTs` — the
   * session's first event timestamp — picks the last commit at or before that
   * moment as the base; without it (or when no commit predates it) the base
   * is HEAD. Returns the worktree path and resolved base commit.
   */
  async open(
    cwd: string,
    sessionId: string,
    beforeTs?: string,
  ): Promise<{ dir: string; baseCommit: string }> {
    if (!validSessionId(sessionId)) throw new Error('invalid session id');
    const rootRes = await runGit(cwd, ['rev-parse', '--show-toplevel']);
    const root = rootRes.ok ? rootRes.stdout.trim() : '';
    if (!root) throw new Error('not a git repository — worktree replay needs one');

    let base = '';
    if (beforeTs) {
      const r = await runGit(root, ['rev-list', '-1', `--before=${beforeTs}`, 'HEAD']);
      if (r.ok) base = r.stdout.trim();
    }
    if (!base) {
      const r = await runGit(root, ['rev-parse', 'HEAD']);
      if (!r.ok) throw new Error('repository has no commits to replay onto');
      base = r.stdout.trim();
    }

    const dir = path.join(this.replayRoot(), sessionId);
    const existing = this.entries.get(sessionId);
    if (existing && existing.root === root && existing.baseCommit === base && fs.existsSync(dir)) {
      return { dir, baseCommit: base };
    }

    // Stale dir from a previous run (or a base change): tear down and re-add.
    await runGit(root, ['worktree', 'remove', '--force', dir]);
    fs.rmSync(dir, { recursive: true, force: true });
    await runGit(root, ['worktree', 'prune']);
    fs.mkdirSync(this.replayRoot(), { recursive: true });
    const add = await runGit(root, ['worktree', 'add', '--detach', dir, base]);
    if (!add.ok) throw new Error(`worktree add failed: ${add.stderr.trim() || 'unknown error'}`);

    this.entries.set(sessionId, { root, dir, baseCommit: base });
    return { dir, baseCommit: base };
  }

  /**
   * Reset the session's worktree to its base commit and re-apply `ops` in
   * order. Idempotent per (ops) — each seek starts from the base, so scrubbing
   * left simply re-applies a shorter prefix. Ops that can't apply (path outside
   * the repo, missing file, old_string not found) are skipped and reported,
   * never partially applied.
   */
  async seek(sessionId: string, ops: ReplayOp[]): Promise<SeekResult> {
    const entry = this.entries.get(sessionId);
    if (!entry) throw new Error('replay not open for this session — call replay.open first');
    const { root, dir } = entry;

    await runGit(dir, ['checkout', '-q', '--', '.']);
    await runGit(dir, ['clean', '-fdq']);

    let applied = 0;
    const skipped: SkippedOp[] = [];
    for (const op of ops ?? []) {
      const input = op.input ?? {};
      const src = input.file_path ?? '';
      const skip = (reason: string) => skipped.push({ path: src, op: op.name, reason });

      if (op.name !== 'Write' && op.name !== 'Edit' && op.name !== 'MultiEdit') {
        skip('unsupported op');
        continue;
      }
      if (!src) {
        skip('no file path');
        continue;
      }
      // Transcript paths are absolute within the agent's checkout; remap them
      // into the worktree via the repo root, refusing anything that escapes.
      const rel = path.relative(root, src);
      if (!rel || rel.startsWith('..') || path.isAbsolute(rel)) {
        skip('outside the repository');
        continue;
      }
      const target = path.join(dir, rel);

      try {
        if (op.name === 'Write') {
          fs.mkdirSync(path.dirname(target), { recursive: true });
          fs.writeFileSync(target, String(input.content ?? ''));
          applied++;
          continue;
        }
        if (!fs.existsSync(target)) {
          skip('file not present at this point in the timeline');
          continue;
        }
        let content = fs.readFileSync(target, 'utf8');
        const edits =
          op.name === 'MultiEdit'
            ? (input.edits ?? [])
            : [
                {
                  old_string: input.old_string,
                  new_string: input.new_string,
                  replace_all: input.replace_all,
                },
              ];
        let ok = true;
        for (const e of edits) {
          const oldS = String(e.old_string ?? '');
          const newS = String(e.new_string ?? '');
          if (!oldS || !content.includes(oldS)) {
            skip('old_string not found (file diverged from the transcript)');
            ok = false;
            break;
          }
          content = e.replace_all ? content.split(oldS).join(newS) : content.replace(oldS, newS);
        }
        if (!ok) continue;
        fs.writeFileSync(target, content);
        applied++;
      } catch (err) {
        skip(err instanceof Error ? err.message : 'write failed');
      }
    }

    const status = await runGit(dir, ['status', '--porcelain']);
    const changedFiles = status.ok ? status.stdout.split('\n').filter((l) => l.trim()).length : 0;
    return { applied, skipped, changedFiles };
  }

  /** Remove the session's replay worktree. Safe to call when already gone. */
  async close(sessionId: string): Promise<{ ok: true }> {
    const entry = this.entries.get(sessionId);
    if (entry) {
      await runGit(entry.root, ['worktree', 'remove', '--force', entry.dir]);
      fs.rmSync(entry.dir, { recursive: true, force: true });
      await runGit(entry.root, ['worktree', 'prune']);
      this.entries.delete(sessionId);
    }
    return { ok: true };
  }
}

export const timelineReplay = new TimelineReplayService();
