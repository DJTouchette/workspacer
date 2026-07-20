/**
 * Auto-generated conversation titles for resumable sessions, read from the
 * provider's own on-disk record:
 *
 *  - claude: the transcript's `{"type":"ai-title","aiTitle":"…"}` line (what
 *    Claude Code's /resume picker shows) when present — it's a newer feature,
 *    so most transcripts don't have one — else the first genuine user message.
 *  - codex: the rollout's first `user_message` event. Rollout files are found
 *    by name: `~/.codex/sessions/<y>/<m>/<d>/rollout-…-<sessionId>.jsonl`.
 *
 * Reads are capped (head of file only) and cached by mtime, so the sidebar's
 * 60s poll doesn't re-parse unchanged transcripts.
 */
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';

const READ_CAP_BYTES = 256 * 1024;
const MAX_TITLE_CHARS = 120;

interface CacheEntry {
  mtimeMs: number;
  title: string | undefined;
}
const titleCache = new Map<string, CacheEntry>();
/** sessionId → resolved rollout path (codex); misses are re-tried each call. */
const rolloutPathCache = new Map<string, string>();

/** Claude Code's project-dir munging: every '/' and '.' becomes '-'. */
export function claudeProjectDir(cwd: string): string {
  return cwd.replace(/[/.]/g, '-');
}

/** First line of a message, cleaned up for display as a title. */
function cleanTitle(text: string): string | undefined {
  const line = text
    .trim()
    .split('\n')[0]
    .replace(/^[#>*\-\s`]+/, '')
    .replace(/\s+/g, ' ')
    .trim();
  if (!line) return undefined;
  return line.length > MAX_TITLE_CHARS ? `${line.slice(0, MAX_TITLE_CHARS)}…` : line;
}

/** True for transcript "user" content that isn't the human talking: command
 *  wrappers, system reminders, pasted tool results, caveat banners. */
function isSyntheticUserText(text: string): boolean {
  const t = text.trimStart();
  return t.startsWith('<') || t.startsWith('Caveat:') || t === '';
}

function userTextFromClaudeLine(obj: any): string | undefined {
  if (obj?.type !== 'user' || obj?.isMeta) return undefined;
  const content = obj?.message?.content;
  if (typeof content === 'string') {
    return isSyntheticUserText(content) ? undefined : content;
  }
  if (Array.isArray(content)) {
    for (const block of content) {
      if (block?.type === 'tool_result') return undefined; // tool feedback turn
      if (block?.type === 'text' && typeof block.text === 'string') {
        return isSyntheticUserText(block.text) ? undefined : block.text;
      }
    }
  }
  return undefined;
}

/** Parse the head of a transcript/rollout into a display title. Exported for
 *  tests; callers go through titleForSession. */
export async function extractTitle(
  filePath: string,
  kind: 'claude' | 'codex',
): Promise<string | undefined> {
  let head: string;
  try {
    const fh = await fs.promises.open(filePath, 'r');
    try {
      const buf = Buffer.alloc(READ_CAP_BYTES);
      const { bytesRead } = await fh.read(buf, 0, READ_CAP_BYTES, 0);
      head = buf.subarray(0, bytesRead).toString('utf8');
    } finally {
      await fh.close();
    }
  } catch {
    return undefined;
  }

  const lines = head.split('\n');
  // Drop a trailing partial line (cap landed mid-record).
  if (!head.endsWith('\n')) lines.pop();

  let aiTitle: string | undefined;
  let firstUserMsg: string | undefined;
  for (const line of lines) {
    if (!line.trim()) continue;
    let obj: any;
    try {
      obj = JSON.parse(line);
    } catch {
      continue;
    }
    if (kind === 'claude') {
      // Last ai-title wins — Claude re-titles as the conversation evolves.
      if (obj?.type === 'ai-title' && typeof obj.aiTitle === 'string') {
        aiTitle = obj.aiTitle;
        continue;
      }
      if (!firstUserMsg) firstUserMsg = userTextFromClaudeLine(obj);
    } else {
      const p = obj?.payload;
      if (
        obj?.type === 'event_msg' &&
        p?.type === 'user_message' &&
        typeof p.message === 'string'
      ) {
        if (!isSyntheticUserText(p.message)) {
          firstUserMsg = p.message;
          break;
        }
      }
    }
  }
  const raw = aiTitle ?? firstUserMsg;
  return raw ? cleanTitle(raw) : undefined;
}

/** Locate a codex rollout by its session-id filename suffix, newest day first. */
async function findCodexRollout(sessionId: string): Promise<string | undefined> {
  const cached = rolloutPathCache.get(sessionId);
  if (cached) return cached;
  const root = path.join(os.homedir(), '.codex', 'sessions');
  const suffix = `-${sessionId}.jsonl`;
  const listDesc = async (dir: string): Promise<string[]> => {
    try {
      return (await fs.promises.readdir(dir)).sort().reverse();
    } catch {
      return [];
    }
  };
  for (const y of await listDesc(root)) {
    for (const m of await listDesc(path.join(root, y))) {
      for (const d of await listDesc(path.join(root, y, m))) {
        for (const f of await listDesc(path.join(root, y, m, d))) {
          if (f.endsWith(suffix)) {
            const full = path.join(root, y, m, d, f);
            rolloutPathCache.set(sessionId, full);
            return full;
          }
        }
      }
    }
  }
  return undefined;
}

/**
 * Best-effort auto title for a resumable session; undefined when the provider
 * has no on-disk record we know how to read (opencode/pi) or the file is gone.
 */
export async function titleForSession(row: {
  sessionId: string;
  provider: string;
  cwd: string;
  transcriptPath?: string | null;
}): Promise<string | undefined> {
  const provider = row.provider || 'claude';
  let filePath: string | undefined;
  let kind: 'claude' | 'codex';
  if (provider === 'claude') {
    kind = 'claude';
    filePath =
      row.transcriptPath ||
      path.join(
        os.homedir(),
        '.claude',
        'projects',
        claudeProjectDir(row.cwd),
        `${row.sessionId}.jsonl`,
      );
  } else if (provider === 'codex') {
    kind = 'codex';
    filePath = await findCodexRollout(row.sessionId);
  } else {
    return undefined;
  }
  if (!filePath) return undefined;

  let mtimeMs: number;
  try {
    mtimeMs = (await fs.promises.stat(filePath)).mtimeMs;
  } catch {
    titleCache.delete(row.sessionId);
    return undefined;
  }
  const hit = titleCache.get(row.sessionId);
  if (hit && hit.mtimeMs === mtimeMs) return hit.title;

  const title = await extractTitle(filePath, kind);
  titleCache.set(row.sessionId, { mtimeMs, title });
  return title;
}

/** Test hook: drop all caches. */
export function resetSessionTitleCaches(): void {
  titleCache.clear();
  rolloutPathCache.clear();
}
