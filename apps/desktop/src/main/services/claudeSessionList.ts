/**
 * Discover existing Claude Code sessions for a given working directory.
 * Reads JSONL transcript files from ~/.claude/projects/<encoded-path>/
 */
import * as fs from 'fs';
import * as path from 'path';
import * as os from 'os';

import { trimSuffix } from '../lib/providerParity';

export interface ClaudeSessionSummary {
  sessionId: string;
  timestamp: string;
  /** First user message or session name (truncated) */
  summary: string;
}

/** Encode a directory path the same way the Claude CLI names its per-project
 *  transcript folder: every '/', '\\' and ':' becomes '-', with NO stripping —
 *  so an absolute unix cwd '/foo/bar' encodes to '-foo-bar' (leading dash) and
 *  'C:\\foo' to 'C--foo'. This must match claudemon's `encoded_cwd`
 *  (services/claudemon/src/session/transcript.rs); stripping the leading slash
 *  (as this did before) pointed every lookup at a non-existent folder, so the
 *  resume picker came up empty on unix/macOS. A trailing separator is dropped
 *  first since a real cwd never carries one. */
/** How many CHARACTERS of a transcript summary reach the wire.
 *
 *  Code POINTS, not UTF-16 code units. `String.prototype.slice` counts units, so
 *  every non-BMP character costs two and this side returned half the text the Go
 *  twin did — and an odd boundary left a LONE LEAD SURROGATE, which
 *  JSON.stringify emits as a bare \ud83d and every consumer renders as a
 *  replacement char. claude.sessionsForDir is answered by whichever provider is
 *  registered, so the two must clip identically; the brain's clip()
 *  (cmd/brain/discovery.go) counts runes and has had TestClipDoesNotSplitRune
 *  since it was written. */
const SUMMARY_MAX_CHARS = 100;

/** Twin of clip() in services/hub/cmd/brain/discovery.go. */
export function clip(s: string, n: number): string {
  const points = Array.from(s);
  return points.length > n ? points.slice(0, n).join('') : s;
}

function encodeDirName(dir: string): string {
  return dir.replace(/[/\\]+$/, '').replace(/[/\\:]/g, '-');
}

/**
 * encodeDirName plus the one thing the encoding does not give you on its own:
 * the guarantee that the result is a PLAIN COMPONENT. Returns null to refuse.
 *
 * capspec.unscopedByDecision excuses `claude.sessionsForDir` from bus
 * confinement on the stated grounds that "the caller's string is never opened as
 * a path". That was false: the encoder maps '/', '\' and ':' to '-' and touches
 * nothing else, so '.' and '..' survive verbatim and become a real path
 * component — path.join(~/.claude/projects, '..') is ~/.claude, and the handler
 * then enumerated every *.jsonl one level ABOVE the transcript sandbox
 * (~/.claude/history.jsonl, the user's whole prompt history). '' is the same
 * shape one level down: it names the projects dir itself.
 *
 * No real cwd encodes to any of the three ('/' encodes to '-'), so refusing them
 * costs nothing and makes the exemption's sentence true. Mirrors
 * claudeProjectDirName in services/hub/cmd/brain/discovery.go; the pairs are
 * pinned by the `projectDirNames` block of
 * contracts/path-containment-cases.json.
 */
export function claudeProjectDirName(cwd: string): string | null {
  const name = encodeDirName(cwd);
  if (name === '' || name === '.' || name === '..') return null;
  return name;
}

/** Rows returned to the picker. */
const LIST_LIMIT = 20;
/** Transcripts whose headers we read, newest-by-mtime first. Provably enough:
 *  every row's sort key IS its mtime (see the dead `entry.timestamp` branch
 *  below, kept for twin parity), so the top LIST_LIMIT rows are always inside
 *  the newest SCAN_CAP files — the cap changes cost, never results. */
const SCAN_CAP = 100;
/** Parallel header reads: enough to hide cold-disk latency, few enough not to
 *  monopolize libuv's thread pool (default 4 threads) or the fd table. */
const READ_CONCURRENCY = 8;

/** Per-file header cache, validated by (mtimeMs, size). A transcript's first
 *  8KB never changes once written unless the file itself changes, so a warm
 *  open of a big project re-reads nothing. Bounded so a long-lived process
 *  browsing many projects can't grow it without limit. */
const headerCache = new Map<string, { mtimeMs: number; size: number; row: ClaudeSessionSummary }>();
const HEADER_CACHE_MAX = 2000;

/** Run tasks with at most `limit` in flight. Tiny local pLimit — not worth a
 *  dependency for one call site. */
async function mapLimit<T, R>(
  items: T[],
  limit: number,
  fn: (item: T) => Promise<R>,
): Promise<R[]> {
  const out: R[] = new Array(items.length);
  let next = 0;
  const workers = Array.from({ length: Math.min(limit, items.length) }, async () => {
    for (;;) {
      const i = next++;
      if (i >= items.length) return;
      out[i] = await fn(items[i]);
    }
  });
  await Promise.all(workers);
  return out;
}

/**
 * ASYNC on purpose — this runs in the Electron MAIN process, and the sync
 * version froze the whole app: the first open of a project with hundreds of
 * transcripts did hundreds of COLD `openSync`/`readSync` calls on the main
 * thread (page-cache-warm re-runs are why measuring it after a repro showed
 * nothing — the freeze only reproduces cold). All IO now rides fs.promises
 * with bounded concurrency, and per-file results are cached by (mtime, size).
 */
export async function listClaudeSessionsForDir(cwd: string): Promise<ClaudeSessionSummary[]> {
  const claudeDir = path.join(os.homedir(), '.claude', 'projects');
  const encoded = claudeProjectDirName(cwd);
  if (encoded === null) return [];
  const projectDir = path.join(claudeDir, encoded);

  let files: string[];
  try {
    files = (await fs.promises.readdir(projectDir)).filter((f) => f.endsWith('.jsonl'));
  } catch {
    return []; // missing project dir = no sessions (same as the old existsSync gate)
  }

  // TrimSuffix, not replace(): replace removes the FIRST occurrence anywhere,
  // so 'a.jsonl.b.jsonl' became 'a.b.jsonl' here and 'a.jsonl.b' in the Go twin
  // (discovery.go strings.TrimSuffix) — two different resume ids for one
  // transcript. Worse, '.jsonlagent-x.jsonl' became 'agent-x.jsonl', which then
  // matched the subagent filter below and dropped a row the brain listed.
  const candidates = files
    .map((file) => ({ file, sessionId: trimSuffix(file, '.jsonl') }))
    // Skip subagent sessions
    .filter((c) => !c.sessionId.startsWith('agent-'));

  // Stat everything (cheap even in the hundreds), then read headers from only
  // the newest SCAN_CAP files.
  const stats = await mapLimit(candidates, 16, async (c) => {
    try {
      return { ...c, stat: await fs.promises.stat(path.join(projectDir, c.file)) };
    } catch {
      return null; // unreadable — skip, as before
    }
  });
  const readable = stats
    .filter((s): s is NonNullable<(typeof stats)[number]> => s !== null)
    .sort((a, b) => b.stat.mtimeMs - a.stat.mtimeMs)
    .slice(0, SCAN_CAP);

  const rows = await mapLimit(readable, READ_CONCURRENCY, async ({ file, sessionId, stat }) => {
    const filePath = path.join(projectDir, file);
    const cached = headerCache.get(filePath);
    if (cached && cached.mtimeMs === stat.mtimeMs && cached.size === stat.size) {
      return cached.row;
    }
    try {
      // Read first ~8KB to extract metadata without loading the whole file
      const fh = await fs.promises.open(filePath, 'r');
      const buf = Buffer.alloc(8192);
      let bytesRead = 0;
      try {
        bytesRead = (await fh.read(buf, 0, 8192, 0)).bytesRead;
      } finally {
        await fh.close();
      }

      const chunk = buf.toString('utf-8', 0, bytesRead);
      const lines = chunk.split('\n').filter((l) => l.trim());

      let timestamp = stat.mtime.toISOString();
      let summary = '';

      for (const line of lines) {
        try {
          const entry = JSON.parse(line);
          if (!timestamp && entry.timestamp) {
            timestamp = entry.timestamp;
          }
          // Look for a session name (set via --name flag)
          if (entry.type === 'summary' && entry.summary) {
            summary = clip(entry.summary, SUMMARY_MAX_CHARS);
            break;
          }
          // Look for first user message
          if (!summary && entry.type === 'user' && entry.message) {
            const msg = entry.message;
            const content =
              typeof msg.content === 'string'
                ? msg.content
                : Array.isArray(msg.content)
                  ? msg.content
                      .filter((b: any) => b.type === 'text')
                      .map((b: any) => b.text)
                      .join('\n')
                  : '';
            if (content) {
              summary = clip(content, SUMMARY_MAX_CHARS).replace(/\n/g, ' ');
            }
          }
        } catch {
          // Skip unparseable lines
        }
      }

      if (!summary) summary = sessionId;

      const row: ClaudeSessionSummary = { sessionId, timestamp, summary };
      if (headerCache.size >= HEADER_CACHE_MAX) {
        // Bounded, coarse eviction: drop the oldest-inserted half. Insertion
        // order approximates access recency well enough for a header cache.
        let toDrop = HEADER_CACHE_MAX / 2;
        for (const key of headerCache.keys()) {
          if (toDrop-- <= 0) break;
          headerCache.delete(key);
        }
      }
      headerCache.set(filePath, { mtimeMs: stat.mtimeMs, size: stat.size, row });
      return row;
    } catch {
      return null; // Skip unreadable files
    }
  });

  const sessions = rows.filter((r): r is ClaudeSessionSummary => r !== null);

  // Sort by timestamp descending (most recent first)
  sessions.sort((a, b) => new Date(b.timestamp).getTime() - new Date(a.timestamp).getTime());

  // Return top 20
  return sessions.slice(0, LIST_LIMIT);
}
