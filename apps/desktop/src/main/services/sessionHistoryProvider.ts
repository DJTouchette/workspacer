/**
 * Permanent session history for the desktop-as-provider path (claudemon is
 * external, so the hub has no engine and main answers the session methods).
 * The bus contract is docs/session-roster.md §5; an engine hub serves the same
 * methods from hub-rs `live_controls`.
 *
 * - `sessions.history`: one page of claudemon's SQLite history (every session
 *   it ever saw), enriched the way `sessions.recent` is (agent name from the
 *   history DB, provider auto-title), plus the shared-view facts every History
 *   pane needs: `hidden` (shared archive) and `onSidebar` (`sessionOpen`).
 *   History rows carry no liveness, so the live mode comes from claudemon's
 *   in-memory rows.
 * - `sessions.forget`: the only way anything leaves History. claudemon refuses
 *   a live session; on success the id is also dropped from the shared sidebar
 *   and archive so no client keeps a row for a session that no longer exists.
 * - Sidebar sessions idle for more than 7 days: claudemon evicts them from
 *   memory, so the roster fills those ids from `GET /sessions/:id`, which now
 *   answers from history. Cached so the 1s-cadence roster calls stay cheap.
 */
import { callHub } from './hubClient';
import { openSessionIds } from './sessionOpenCache';
import { sessionHistory } from './sessionHistory';
import { titleForSession } from './sessionTitles';
import { claudeSessionStore } from './claudeSessionStore';
import {
  fetchDaemonHistory,
  forgetDaemonSession,
  getDaemonSessionRow,
  listDaemonSessionRows,
  type DaemonSessionRow,
  type HistoryDaemonRow,
} from './recentSessions';
import type {
  HistoryAgentSession,
  SessionHistoryPage,
  SessionHistoryQuery,
} from '../shared/ipcTypes';

const MAX_LIMIT = 200;
const MAX_TEXT = 4096;
/** Rows per page that get transcript-derived titles (mtime-cached reads). */
const TITLE_ENRICH_LIMIT = 40;
/** How far back the history DB is consulted for agent names. */
const NAME_LOOKUP_ROWS = 5000;

function boundedText(value: unknown, what: string): string | undefined {
  if (value === undefined || value === null || value === '') return undefined;
  if (typeof value !== 'string' || value.length > MAX_TEXT || /[\0\n\r]/.test(value))
    throw new Error(`sessions.history ${what} must be a short single-line string`);
  return value;
}

/** Validate caller params. Unknown keys are ignored, like the hub provider. */
export function historyQuery(params: unknown): SessionHistoryQuery {
  const p = (params ?? {}) as Record<string, unknown>;
  if (typeof p !== 'object' || Array.isArray(p))
    throw new Error('sessions.history params must be an object');
  let limit: number | undefined;
  if (p.limit !== undefined && p.limit !== null) {
    if (typeof p.limit !== 'number' || !Number.isInteger(p.limit) || p.limit < 1)
      throw new Error('sessions.history limit must be a positive integer');
    limit = Math.min(p.limit, MAX_LIMIT);
  }
  return {
    cursor: boundedText(p.cursor, 'cursor'),
    limit,
    query: boundedText(p.query, 'query'),
    cwd: boundedText(p.cwd, 'cwd'),
  };
}

const secondsToMs = (s: number | null | undefined): number | undefined =>
  typeof s === 'number' && Number.isFinite(s) && s > 0 ? s * 1000 : undefined;

function hiddenIds(doc: unknown): Set<string> {
  const archived = (doc as { archived?: unknown } | null)?.archived;
  return archived && typeof archived === 'object' && !Array.isArray(archived)
    ? new Set(Object.keys(archived))
    : new Set();
}

/** Join one history page with live modes, names and the shared view state.
 *  Pure; exported for tests. */
export function historyRows(
  rows: HistoryDaemonRow[],
  ctx: {
    hidden: Set<string>;
    open: Set<string>;
    /** session id → live daemon mode, for sessions claudemon holds in memory. */
    modes: Map<string, string>;
    names: Map<string, { agentName: string; model: string | null }>;
  },
): Array<HistoryAgentSession & { transcriptPath?: string | null }> {
  return rows
    .filter((r) => r.session_id && !r.session_id.startsWith('agent-'))
    .map((r) => {
      const named = ctx.names.get(r.session_id);
      const mode = ctx.modes.get(r.session_id) ?? 'stopped';
      const updatedAt = Date.parse(r.updated_at ?? '') || secondsToMs(r.last_event_at) || 0;
      const endedAt = mode === 'stopped' ? secondsToMs(r.ended_at) : undefined;
      const row: HistoryAgentSession & { transcriptPath?: string | null } = {
        sessionId: r.session_id,
        provider: r.provider || 'claude',
        cwd: r.cwd || '',
        mode,
        transport: r.transport === 'stream' ? 'stream' : 'pty',
        // Legacy meaning: stopped and idle past claudemon's 7-day window.
        archived: mode === 'stopped' && Date.now() - updatedAt > 7 * 24 * 60 * 60 * 1000,
        updatedAt,
        startedAt: Date.parse(r.started_at ?? '') || secondsToMs(r.created_at) || 0,
        name: named?.agentName || (typeof r.name === 'string' ? r.name : '') || '',
        title: typeof r.title === 'string' ? r.title : '',
        model: named?.model || (typeof r.model === 'string' ? r.model : '') || '',
        hidden: ctx.hidden.has(r.session_id),
        onSidebar: ctx.open.has(r.session_id),
        transcriptCopy: r.transcript_copy === true,
        transcriptPath: r.transcript_path,
      };
      if (endedAt !== undefined) row.endedAt = endedAt;
      if (typeof r.first_prompt === 'string' && r.first_prompt) row.firstPrompt = r.first_prompt;
      if (typeof r.user_prompts === 'number') row.userPrompts = r.user_prompts;
      if (typeof r.tool_calls === 'number') row.toolCalls = r.tool_calls;
      return row;
    });
}

/** `sessions.history`. Rejects (with the daemon's reason) when history cannot
 *  be read, so a client never mistakes "unreadable" for "empty". */
export async function listSessionHistory(params: unknown): Promise<SessionHistoryPage> {
  const query = historyQuery(params);
  const [page, archive, open, resident] = await Promise.all([
    fetchDaemonHistory(query),
    callHub<unknown>('sessionArchive.get', {}).catch(() => null),
    openSessionIds(),
    listDaemonSessionRows(),
  ]);
  const modes = new Map<string, string>();
  for (const row of resident ?? []) {
    if (typeof row.mode === 'string' && row.mode) modes.set(row.session_id, row.mode);
  }
  const names = new Map<string, { agentName: string; model: string | null }>();
  try {
    const wanted = new Set(page.sessions.map((r) => r.session_id));
    for (const rec of sessionHistory.recent(NAME_LOOKUP_ROWS)) {
      if (wanted.has(rec.sessionId) && !names.has(rec.sessionId))
        names.set(rec.sessionId, { agentName: rec.agentName, model: rec.model });
    }
  } catch {
    // History DB unavailable: rows keep claudemon's own name/model.
  }
  const rows = historyRows(page.sessions, { hidden: hiddenIds(archive), open, modes, names });
  await Promise.all(
    rows.slice(0, TITLE_ENRICH_LIMIT).map(async (row) => {
      if (row.title) return;
      try {
        row.title = (await titleForSession(row)) ?? '';
      } catch {
        /* keep '' */
      }
    }),
  );
  const sessions = rows.map(({ transcriptPath: _tp, ...wire }) => wire);
  return page.next ? { sessions, next: page.next } : { sessions };
}

/** `sessions.forget`: delete a stopped session from History for good. */
export async function forgetSession(
  params: unknown,
): Promise<{ ok: true; sessionId: string; transcriptCopyRemoved: boolean }> {
  const { sessionId } = (params ?? {}) as { sessionId?: unknown };
  if (
    typeof sessionId !== 'string' ||
    !sessionId ||
    sessionId.length > 256 ||
    /[/\\\0\s]/.test(sessionId)
  )
    throw new Error('sessions.forget requires { sessionId }');
  const held = claudeSessionStore.getSnapshot(sessionId);
  if (held && held.status !== 'ended')
    throw new Error('session is live; stop it before deleting it from history');
  const { transcriptCopyRemoved } = await forgetDaemonSession(sessionId);
  evicted.delete(sessionId);
  // The session no longer exists anywhere: drop it from the shared sidebar and
  // archive too, so no client keeps listing or hiding a ghost. Best-effort; the
  // history delete itself already succeeded.
  await Promise.all([
    callHub('sessionOpen.set', { sessionId, open: false }).catch(() => undefined),
    callHub('sessionArchive.set', { sessionId, archived: false }).catch(() => undefined),
  ]);
  return { ok: true, sessionId, transcriptCopyRemoved };
}

// ── Sidebar ids claudemon evicted from memory ──────────────────────────────

interface Cached {
  row: DaemonSessionRow | null;
  at: number;
}
const evicted = new Map<string, Cached>();
/** A miss (unknown id, daemon hiccup) is retried after this long. */
const MISS_TTL_MS = 60_000;
/** New by-id reads per roster call; the rest are filled on later calls. */
const FILL_BUDGET = 50;
const FILL_CONCURRENCY = 8;

/**
 * Rows for sidebar (`open`) ids that neither the store nor claudemon's
 * in-memory rows hold: claudemon evicts stopped sessions idle for more than
 * 7 days, and `GET /sessions/:id` answers those from permanent history. A
 * found row is cached until the id leaves the sidebar (history rows of stopped
 * sessions do not change; a resumed session reappears in memory and wins);
 * misses are retried after a minute.
 */
export async function fillEvictedOpenRows(
  open: Set<string>,
  have: Set<string>,
  now = Date.now(),
): Promise<DaemonSessionRow[]> {
  for (const id of evicted.keys()) if (!open.has(id)) evicted.delete(id);
  const wanted = [...open].filter((id) => !have.has(id));
  const toFetch = wanted
    .filter((id) => {
      const hit = evicted.get(id);
      return !hit || (!hit.row && now - hit.at >= MISS_TTL_MS);
    })
    .slice(0, FILL_BUDGET);
  for (let i = 0; i < toFetch.length; i += FILL_CONCURRENCY) {
    await Promise.all(
      toFetch.slice(i, i + FILL_CONCURRENCY).map(async (id) => {
        evicted.set(id, { row: await getDaemonSessionRow(id), at: now });
      }),
    );
  }
  const out: DaemonSessionRow[] = [];
  for (const id of wanted) {
    const row = evicted.get(id)?.row;
    if (row) out.push(row);
  }
  return out;
}

/** Test seam. */
export function resetEvictedOpenRowsForTests(): void {
  evicted.clear();
}
