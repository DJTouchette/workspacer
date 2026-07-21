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
interface DaemonSessionRow {
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
        costUSD: h?.costUSD ?? 0,
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
