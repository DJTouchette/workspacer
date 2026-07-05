/**
 * Per-turn "changed files" snapshots behind the transcript's ChangedFilesCard.
 *
 * When an agent turn finishes, ClaudePane collects the files that turn's tool
 * calls edited and freezes a snapshot of their git line counts at that moment
 * (a later turn touching the same file must not mutate an older card). Counts
 * come from `git diff --numstat` against the pane's cwd, intersected with the
 * tool-reported paths; when git isn't available (non-repo cwd, deleted
 * worktree) the card falls back to line estimates parsed from the tool inputs.
 *
 * Snapshots are renderer-memory only, cached per (sessionId, turn-group index)
 * so they survive pane remounts and GUI↔Term toggles within an app run. After
 * an app restart the conversation itself is restored from claudemon, but these
 * snapshots are gone — restored turns render the estimate fallback instead of
 * stale git numbers.
 */

import { GitClient, type FileStatus, type NumstatEntry } from './gitQueries';
import type { ToolCall } from '../types/claudeSession';

const git = new GitClient();

/** Tool names (lowercased) that edit files, across providers: claude
 *  Edit/MultiEdit/Write/NotebookEdit · codex apply_patch · opencode/pi
 *  edit/write/patch. Shell-driven edits are invisible by design. */
const EDIT_TOOL_NAMES = new Set([
  'edit',
  'multiedit',
  'write',
  'notebookedit',
  'apply_patch',
  'patch',
]);

export interface EditEstimate {
  added: number;
  removed: number;
}

/** Files a run of tool calls edited, keyed by the tool-reported path (usually
 *  absolute), with line-count estimates from the tool inputs. Codex's
 *  apply_patch carries only a path, so its estimates stay 0/0. */
export function collectEditedFiles(calls: ToolCall[]): Map<string, EditEstimate> {
  const out = new Map<string, EditEstimate>();
  for (const tc of calls) {
    if (!EDIT_TOOL_NAMES.has(tc.name?.toLowerCase() ?? '')) continue;
    const path = tc.input?.file_path ?? tc.input?.path ?? tc.input?.filePath;
    if (typeof path !== 'string' || !path) continue;
    const est = out.get(path) ?? { added: 0, removed: 0 };
    // MultiEdit carries its changes in an `edits` array rather than top-level
    // old_string/new_string — same shape summarizeWork (WorkCard) counts.
    const edits = Array.isArray(tc.input?.edits)
      ? tc.input.edits
      : [{ old_string: tc.input?.old_string, new_string: tc.input?.new_string }];
    for (const e of edits) {
      const old = typeof e?.old_string === 'string' ? e.old_string : '';
      const nw = typeof e?.new_string === 'string' ? e.new_string : '';
      if (old) est.removed += old.split('\n').length;
      if (nw) est.added += nw.split('\n').length;
    }
    if (typeof tc.input?.content === 'string' && tc.input.content) {
      est.added += tc.input.content.split('\n').length;
    }
    out.set(path, est);
  }
  return out;
}

export interface TurnChangeFile {
  /** Path as the tool reported it (usually absolute) — what reviewBus and the
   *  editor bus expect. */
  path: string;
  /** Repo-relative path when git matched the file, else the tool path with
   *  the cwd prefix stripped. Drives the tree layout. */
  relPath: string;
  /** Porcelain-style status code to badge ('M', 'A', 'D', …). */
  code: string;
  /** Line counts. Null on both sides means a binary file. */
  added: number | null;
  removed: number | null;
  untracked: boolean;
}

export interface TurnChangeSnapshot {
  files: TurnChangeFile[];
  totalAdded: number;
  totalRemoved: number;
  /** False when git wasn't reachable — counts are tool-input estimates. */
  gitAvailable: boolean;
  capturedAt: number;
}

const norm = (p: string) => p.replace(/\\/g, '/');

function stripCwd(toolPath: string, cwd?: string): string {
  const t = norm(toolPath).replace(/\/+$/, '');
  if (cwd) {
    const c = norm(cwd).replace(/\/+$/, '');
    if (t.startsWith(c + '/')) return t.slice(c.length + 1);
  }
  return t.replace(/^\//, '');
}

/** Claude emits absolute paths while git status is repo-relative — match by
 *  exact, suffix, or basename (same resolution ReviewPane uses). */
function matchStatusFile(toolPath: string, files: FileStatus[]): FileStatus | undefined {
  const t = norm(toolPath);
  const base = t.split('/').pop();
  return files.find((f) => {
    const fp = norm(f.path);
    return t === fp || t.endsWith('/' + fp) || fp.split('/').pop() === base;
  });
}

/** Estimate-only snapshot from tool inputs — the non-git fallback. */
export function estimateSnapshot(
  edited: ReadonlyMap<string, EditEstimate>,
  cwd?: string,
): TurnChangeSnapshot {
  const files: TurnChangeFile[] = [];
  let totalAdded = 0;
  let totalRemoved = 0;
  for (const [path, est] of edited) {
    files.push({
      path,
      relPath: stripCwd(path, cwd),
      code: 'M',
      added: est.added,
      removed: est.removed,
      untracked: false,
    });
    totalAdded += est.added;
    totalRemoved += est.removed;
  }
  return { files, totalAdded, totalRemoved, gitAvailable: false, capturedAt: Date.now() };
}

/**
 * Freeze the turn's change set: git status + numstat (staged and unstaged,
 * summed per path) intersected with the tool-reported files. Known v1 limits:
 * per-file counts reflect the file's whole uncommitted delta at capture time
 * (two agents editing the same file in one cwd blend), and a file the agent
 * committed during the turn falls back to its tool-input estimate.
 */
export async function captureTurnSnapshot(
  cwd: string,
  edited: ReadonlyMap<string, EditEstimate>,
): Promise<TurnChangeSnapshot> {
  let status;
  try {
    status = await git.status(cwd);
  } catch {
    return estimateSnapshot(edited, cwd);
  }
  // Counts are decoration — don't let a numstat hiccup take down the card.
  const [unstagedStats, stagedStats] = await Promise.all([
    git.numstat(cwd, false).catch(() => [] as NumstatEntry[]),
    git.numstat(cwd, true).catch(() => [] as NumstatEntry[]),
  ]);

  // Fold staged + unstaged counts per path; binary (null) on either side wins.
  const counts = new Map<string, { added: number | null; deleted: number | null }>();
  for (const e of [...stagedStats, ...unstagedStats]) {
    const cur = counts.get(e.path);
    if (!cur) {
      counts.set(e.path, { added: e.added, deleted: e.deleted });
    } else if (cur.added == null || cur.deleted == null || e.added == null || e.deleted == null) {
      counts.set(e.path, { added: null, deleted: null });
    } else {
      counts.set(e.path, { added: cur.added + e.added, deleted: cur.deleted + e.deleted });
    }
  }

  const files: TurnChangeFile[] = [];
  let totalAdded = 0;
  let totalRemoved = 0;
  for (const [path, est] of edited) {
    const match = matchStatusFile(path, status.files);
    if (!match) {
      // Committed during the turn (or matched nothing) — the tool-input
      // estimate is the only honest number left.
      files.push({
        path,
        relPath: stripCwd(path, cwd),
        code: 'M',
        added: est.added,
        removed: est.removed,
        untracked: false,
      });
      totalAdded += est.added;
      totalRemoved += est.removed;
      continue;
    }
    const untracked = match.staged === '?';
    const stat = counts.get(match.path);
    // Numstat omits untracked files — a freshly written file estimates as
    // all-added from its tool input, matching how the diff would read. A
    // present entry is authoritative even when null (null = binary).
    const added = untracked ? est.added : stat ? stat.added : est.added;
    const removed = untracked ? 0 : stat ? stat.deleted : est.removed;
    files.push({
      path,
      relPath: match.path,
      code: untracked ? 'A' : match.unstaged !== ' ' ? match.unstaged : match.staged,
      added,
      removed,
      untracked,
    });
    totalAdded += added ?? 0;
    totalRemoved += removed ?? 0;
  }
  return { files, totalAdded, totalRemoved, gitAvailable: true, capturedAt: Date.now() };
}

// ── Per-session snapshot cache ──
//
// Module-level so snapshots survive pane remounts. Keyed by the global
// conversation index of the turn-group's first assistant turn — stable because
// the conversation only ever appends.

const PER_SESSION_CAP = 50;

const cache = new Map<string, Map<number, TurnChangeSnapshot>>();
const inFlight = new Map<string, Promise<TurnChangeSnapshot>>();

export function getTurnSnapshot(
  sessionId: string,
  groupKey: number,
): TurnChangeSnapshot | undefined {
  return cache.get(sessionId)?.get(groupKey);
}

/** Capture (once) and cache the snapshot for a completed turn-group. Repeated
 *  calls for the same key share the in-flight capture. */
export function ensureTurnSnapshot(
  sessionId: string,
  groupKey: number,
  cwd: string | undefined,
  edited: ReadonlyMap<string, EditEstimate>,
): Promise<TurnChangeSnapshot> {
  const existing = getTurnSnapshot(sessionId, groupKey);
  if (existing) return Promise.resolve(existing);
  const flightKey = `${sessionId}:${groupKey}`;
  const pending = inFlight.get(flightKey);
  if (pending) return pending;

  const promise = (
    cwd ? captureTurnSnapshot(cwd, edited) : Promise.resolve(estimateSnapshot(edited, cwd))
  )
    .then((snap) => {
      let bySession = cache.get(sessionId);
      if (!bySession) {
        bySession = new Map();
        cache.set(sessionId, bySession);
      }
      bySession.set(groupKey, snap);
      // Insertion-ordered eviction keeps long sessions bounded; evicted turns
      // degrade to the estimate fallback, same as after an app restart.
      while (bySession.size > PER_SESSION_CAP) {
        const oldest = bySession.keys().next().value;
        if (oldest === undefined) break;
        bySession.delete(oldest);
      }
      return snap;
    })
    .finally(() => inFlight.delete(flightKey));
  inFlight.set(flightKey, promise);
  return promise;
}
