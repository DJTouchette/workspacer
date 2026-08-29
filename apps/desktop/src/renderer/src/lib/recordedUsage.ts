/**
 * The recorded-usage lookup every agent surface reads at a cold start.
 *
 * `listRecentAgentSessions` already returns the daemon's full session list
 * joined with the desktop history DB's cost and token figures. Nothing rendered
 * them: the per-agent surfaces derive from LIVE snapshots, and a restored
 * agent's session is a stopped row with no snapshot, so cost and tokens showed
 * a dash on a machine holding five figures of recorded spend.
 *
 * This turns that list into a sessionId → figures map, which
 * `withRecordedUsage` merges under the live stats.
 */
import type { RecentAgentSession } from '../../../main/shared/ipcTypes';
import type { RecordedSessionUsage } from './sessionStats';

export type RecordedUsageBySession = Record<string, RecordedSessionUsage>;

/**
 * Index the recent-sessions list by session id, keeping only rows that actually
 * recorded something.
 *
 * A row with neither figure is OMITTED rather than stored as an empty object:
 * "the history DB has nothing for this session" and "this session cost nothing"
 * are different answers, and only the absence is ours to state. Downstream that
 * means `map[sessionId]` is undefined and `withRecordedUsage` is a no-op, so
 * the surface keeps rendering its honest dash.
 */
export function recordedUsageBySession(sessions: RecentAgentSession[]): RecordedUsageBySession {
  const out: RecordedUsageBySession = {};
  for (const s of sessions) {
    if (!s.sessionId) continue;
    if (s.costUSD === undefined && s.billedTokens === undefined) continue;
    out[s.sessionId] = {
      ...(s.costUSD !== undefined && { costUSD: s.costUSD }),
      ...(s.billedTokens !== undefined && { billedTokens: s.billedTokens }),
    };
  }
  return out;
}
