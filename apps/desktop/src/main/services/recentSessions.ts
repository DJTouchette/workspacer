/**
 * Recent agent sessions — every session claudemon still holds (all providers,
 * including stopped and archived rows), joined with the desktop's own
 * session_history SQLite for the human-facing bits (agent name, model, cost).
 *
 * The daemon is the truth for WHAT is resumable: it never deletes rows, it
 * only stops/archives them. The history DB is the truth for what the user
 * called the agent. Sessions the daemon has forgotten (wiped state dir) are
 * not listed — a row here must be actually resumable.
 */
import { CLAUDEMON_API_URL } from './claudemonDaemon';
import { sessionHistory } from './sessionHistory';
import { titleForSession } from './sessionTitles';
import type { RecentAgentSession } from '../shared/ipcTypes';

/** Wire shape of one `GET /sessions` row (serialized SessionState + extras). */
export interface DaemonSessionRow {
  session_id: string;
  cwd?: string | null;
  mode?: string;
  provider?: string;
  transport?: 'pty' | 'stream';
  updated_at?: string;
  started_at?: string;
  archived?: boolean;
  transcript_path?: string | null;
}

/** How many rows get transcript-derived titles per fetch. The sidebar shows
 *  at most 20 (recentSessionFilter); reading a few extra covers rows the
 *  filter may drop. Reads are mtime-cached, so this is cheap after tick one. */
const TITLE_ENRICH_LIMIT = 40;

interface HistoryName {
  sessionId: string;
  agentName: string;
  model: string | null;
  costUSD: number;
  inputTokens: number;
  outputTokens: number;
}

/**
 * A history figure the DB cannot vouch for, reported as absent.
 *
 * `session_history.cost_usd` / `input_tokens` / `output_tokens` are all
 * `DEFAULT 0` and never NULL, so a stored 0 is indistinguishable from a row
 * that was created but never had usage written to it — and a third of the rows
 * on a real machine are exactly that. Zero is therefore reported as UNKNOWN
 * rather than as a measured $0.00 / 0 tokens: an honest dash beats a confident
 * wrong zero, and no surface can un-invent a number it was handed.
 */
function recorded(n: number | undefined): number | undefined {
  return n ? n : undefined;
}

/** Join daemon rows with history names into the wire summaries, newest first. */
export function mergeRecentSessions(
  rows: DaemonSessionRow[],
  history: HistoryName[],
): Array<RecentAgentSession & { transcriptPath?: string | null }> {
  const byId = new Map(history.map((h) => [h.sessionId, h]));
  return rows
    .filter((r) => r.session_id && !r.session_id.startsWith('agent-'))
    .map((r) => {
      const h = byId.get(r.session_id);
      return {
        sessionId: r.session_id,
        // Legacy daemon rows serialize provider '' — that's a claude session.
        provider: r.provider || 'claude',
        cwd: r.cwd || '',
        mode: r.mode || 'unknown',
        transport: r.transport ?? 'pty',
        archived: r.archived === true,
        updatedAt: Date.parse(r.updated_at ?? '') || 0,
        startedAt: Date.parse(r.started_at ?? '') || 0,
        name: h?.agentName || '',
        title: '',
        model: h?.model || '',
        costUSD: recorded(h?.costUSD),
        billedTokens: h ? recorded(h.inputTokens + h.outputTokens) : undefined,
        // Internal: carried so the title enrichment can find the transcript;
        // stripped before the list crosses IPC.
        transcriptPath: r.transcript_path,
      };
    })
    .sort((a, b) => b.updatedAt - a.updatedAt);
}

/** Session ids the daemon considers alive (any mode but 'stopped'; archived
 *  rows are excluded by the default listing). Returns null — not [] — when the
 *  daemon is unreachable, so boot reconciliation can retry instead of marking
 *  every restored agent dead while claudemon is still coming up. */
export async function listLiveSessionIds(): Promise<string[] | null> {
  try {
    const res = await fetch(`${CLAUDEMON_API_URL}/sessions`);
    if (!res.ok) return null;
    const rows = (await res.json()) as DaemonSessionRow[];
    if (!Array.isArray(rows)) return null;
    return rows.filter((r) => r.session_id && r.mode !== 'stopped').map((r) => r.session_id);
  } catch {
    return null;
  }
}

/** Raw daemon rows for the session roster: every row claudemon still holds,
 *  including stopped, archived and never-used ones, so the roster policy (not
 *  the daemon's default listing filter) decides what is listed. Null — not [] —
 *  when the daemon cannot be read, so callers can tell "no sessions" from
 *  "unknown". */
export async function listDaemonSessionRows(): Promise<DaemonSessionRow[] | null> {
  try {
    const res = await fetch(
      `${CLAUDEMON_API_URL}/sessions?include_archived=true&include_empty=true&state_only=true`,
      {
        signal: AbortSignal.timeout(5000),
      },
    );
    if (!res.ok) return null;
    const rows = (await res.json()) as unknown;
    if (!Array.isArray(rows)) return null;
    return rows.filter(
      (r): r is DaemonSessionRow =>
        !!r && typeof r === 'object' && typeof (r as DaemonSessionRow).session_id === 'string',
    );
  } catch {
    return null;
  }
}

/** One daemon row, by id: null when the daemon has no such session or cannot
 *  be read. */
export async function getDaemonSessionRow(sessionId: string): Promise<DaemonSessionRow | null> {
  try {
    const res = await fetch(`${CLAUDEMON_API_URL}/sessions/${encodeURIComponent(sessionId)}`, {
      signal: AbortSignal.timeout(5000),
    });
    if (!res.ok) return null;
    const row = (await res.json()) as DaemonSessionRow | null;
    return row && typeof row === 'object' && row.session_id === sessionId ? row : null;
  } catch {
    return null;
  }
}

/** Fetch the full session list from the daemon and enrich it. Errors (daemon
 *  down, mid-restart) resolve to an empty list — the sidebar just shows
 *  nothing rather than breaking the renderer. */
export async function listRecentSessions(): Promise<RecentAgentSession[]> {
  try {
    const res = await fetch(`${CLAUDEMON_API_URL}/sessions?include_archived=true`);
    if (!res.ok) return [];
    const rows = (await res.json()) as DaemonSessionRow[];
    if (!Array.isArray(rows)) return [];
    const names = sessionHistory.recent(500).map((rec) => ({
      sessionId: rec.sessionId,
      agentName: rec.agentName,
      model: rec.model,
      costUSD: rec.costUSD,
      inputTokens: rec.inputTokens,
      outputTokens: rec.outputTokens,
    }));
    const merged = mergeRecentSessions(rows, names);
    // Provider auto-titles (claude ai-title / first user message) for the rows
    // the sidebar can actually show. Best-effort per row — a missing or
    // unreadable transcript just leaves title ''.
    await Promise.all(
      merged.slice(0, TITLE_ENRICH_LIMIT).map(async (row) => {
        try {
          row.title = (await titleForSession(row)) ?? '';
        } catch {
          /* keep '' */
        }
      }),
    );
    return merged.map(({ transcriptPath: _tp, ...wire }) => wire);
  } catch {
    return [];
  }
}

/** One `GET /history` row: the permanent SQLite history claudemon keeps for
 *  every session it ever saw (docs/session-roster.md §5). Always
 *  `mode: "stopped"`: history carries no liveness, so callers merge live state.
 *  `created_at`/`last_event_at`/`ended_at` are epoch SECONDS. */
export interface HistoryDaemonRow extends DaemonSessionRow {
  name?: string | null;
  title?: string | null;
  model?: string | null;
  requested_model?: string | null;
  created_at?: number | null;
  last_event_at?: number | null;
  ended_at?: number | null;
  tool_calls?: number | null;
  user_prompts?: number | null;
  first_prompt?: string | null;
  transcript_copy?: boolean;
  history?: boolean;
}

export interface DaemonHistoryQuery {
  cursor?: string;
  limit?: number;
  query?: string;
  cwd?: string;
}

/** One page of claudemon's permanent history, newest first. Throws (with the
 *  daemon's reason) instead of answering an empty page: "cannot read history"
 *  and "no history" must stay distinguishable to the caller. */
export async function fetchDaemonHistory(
  q: DaemonHistoryQuery,
): Promise<{ sessions: HistoryDaemonRow[]; next?: string }> {
  const params = new URLSearchParams();
  if (q.cursor) params.set('before', q.cursor);
  if (q.limit !== undefined) params.set('limit', String(q.limit));
  if (q.query) params.set('q', q.query);
  if (q.cwd) params.set('cwd', q.cwd);
  const qs = params.toString();
  const res = await fetch(`${CLAUDEMON_API_URL}/history${qs ? `?${qs}` : ''}`, {
    signal: AbortSignal.timeout(10_000),
  });
  if (!res.ok) {
    const reason = (await res.text().catch(() => '')).trim();
    throw new Error(`session history unavailable (${res.status}${reason ? `: ${reason}` : ''})`);
  }
  const page = (await res.json()) as { sessions?: unknown; next?: unknown } | null;
  if (!page || !Array.isArray(page.sessions)) throw new Error('session history: malformed page');
  const sessions = page.sessions.filter(
    (r): r is HistoryDaemonRow =>
      !!r && typeof r === 'object' && typeof (r as HistoryDaemonRow).session_id === 'string',
  );
  return typeof page.next === 'string' && page.next ? { sessions, next: page.next } : { sessions };
}

/** Delete one STOPPED session from claudemon's history for good (row, events
 *  and transcript copy; never the provider's own transcript). Throws with a
 *  user-facing reason: live (409), unknown (404) or anything else. */
export async function forgetDaemonSession(
  sessionId: string,
): Promise<{ transcriptCopyRemoved: boolean }> {
  const res = await fetch(`${CLAUDEMON_API_URL}/sessions/${encodeURIComponent(sessionId)}/forget`, {
    method: 'POST',
    signal: AbortSignal.timeout(10_000),
  });
  if (res.status === 409)
    throw new Error('session is live; stop it before deleting it from history');
  if (res.status === 404) throw new Error('session not found in history');
  if (!res.ok) {
    const reason = (await res.text().catch(() => '')).trim();
    throw new Error(`could not delete from history (${res.status}${reason ? `: ${reason}` : ''})`);
  }
  const body = (await res.json().catch(() => null)) as { transcript_copy_removed?: unknown } | null;
  return { transcriptCopyRemoved: body?.transcript_copy_removed === true };
}
