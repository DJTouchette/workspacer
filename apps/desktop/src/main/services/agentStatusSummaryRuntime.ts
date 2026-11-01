import { configService } from './configService';
import { CLAUDEMON_API_URL } from './claudemonDaemon';
import { callHub } from './hubClient';
import { AgentStatusSummaryService, SummaryUnavailable } from './agentStatusSummary';
import { listed, layoutSessionIds } from './sessionRoster';
import { openSessionIds } from './sessionOpenCache';

const LIVE_MODES = ['input', 'responding', 'approval', 'question'];

// The session roster boundary (sessionRoster.ts, docs/session-roster.md): a
// local live row, or a stopped row that is listed because it is in the shared
// open set or curated in this hub's layout. The access projection carries no
// `updated_at`, so the roster's 24h window never widens this gate. Remote ids
// are never resolved via desktop lookalikes; federation dispatches this
// provider on the owning hub.
export function summarySessionVisible(
  sessionId: string,
  row: unknown,
  layout: unknown,
  open: Iterable<string> = [],
): boolean {
  const s = row as { session_id?: unknown; mode?: unknown } | null;
  if (!s || s.session_id !== sessionId || typeof s.mode !== 'string') return false;
  // Auth stays an allowlist: only the four live modes and stopped, never an
  // unknown/spawning row or a mode this gate has not been taught.
  if (!LIVE_MODES.includes(s.mode) && s.mode !== 'stopped') return false;
  const { updated_at: _ignored, ...access } = row as Record<string, unknown>;
  return listed(access, { layout: layoutSessionIds(layout), open });
}

/** Reject old daemons even if they ignore summary_source and send a full log.
 * Stream cap also prevents parsing/holding their unbounded response in main. */
async function boundedJSON(response: Response, limit: number): Promise<unknown> {
  if (!response.ok || !response.body) return null;
  const reader = response.body.getReader();
  let bytes = 0;
  const chunks: Uint8Array[] = [];
  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      bytes += value.length;
      if (bytes > limit) {
        await reader.cancel();
        return null;
      }
      chunks.push(value);
    }
    return JSON.parse(Buffer.concat(chunks).toString('utf8'));
  } finally {
    reader.releaseLock();
  }
}
export async function readSummarySource(sessionId: string): Promise<unknown> {
  const response = await fetch(
    `${CLAUDEMON_API_URL}/sessions/${encodeURIComponent(sessionId)}/conversation?summary_source=1`,
    { signal: AbortSignal.timeout(5000) },
  );
  return boundedJSON(response, 5000);
}
export function createAgentStatusSummaryService(): AgentStatusSummaryService {
  const service = new AgentStatusSummaryService({
    owningHub: CLAUDEMON_API_URL,
    config: () => configService.getConfig().agents?.statusSummary,
    authorize: async (sessionId) => {
      if (!/^[a-zA-Z0-9_.-]{1,128}$/.test(sessionId) || sessionId.includes('..'))
        throw new Error('agents.summarizeStatus: permission denied');
      const response = await fetch(
        `${CLAUDEMON_API_URL}/sessions/${encodeURIComponent(sessionId)}?summary_meta=1`,
        { signal: AbortSignal.timeout(5000) },
      ).catch(() => {
        throw new SummaryUnavailable();
      });
      if (response.status >= 500) throw new SummaryUnavailable();
      if (response.status === 404) throw new Error('agents.summarizeStatus: permission denied');
      const row = (await boundedJSON(response, 1000).catch(() => {
        throw new SummaryUnavailable();
      })) as {
        projection?: string;
        mode?: string;
      } | null;
      if (
        row?.projection !== 'agent-status-access/v1' ||
        Object.keys(row).length !== 4 ||
        typeof (row as { archived?: unknown }).archived !== 'boolean'
      )
        throw new SummaryUnavailable();
      const stopped = (row as { mode?: string } | null)?.mode === 'stopped';
      const [layout, open] = stopped
        ? await Promise.all([callHub('layout.get'), openSessionIds()])
        : [null, new Set<string>()];
      if (!summarySessionVisible(sessionId, row, layout, open))
        throw new Error('agents.summarizeStatus: permission denied');
    },
    source: readSummarySource,
  });
  let last = JSON.stringify(configService.getConfig().agents?.statusSummary);
  configService.onChange((cfg) => {
    const next = JSON.stringify(cfg.agents?.statusSummary);
    if (next !== last) {
      last = next;
      service.invalidate();
    }
  });
  return service;
}
