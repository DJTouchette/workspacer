import { configService } from './configService';
import { CLAUDEMON_API_URL } from './claudemonDaemon';
import { callHub } from './hubClient';
import { AgentStatusSummaryService, SummaryUnavailable } from './agentStatusSummary';

// Same fleet visibility boundary as brain/visibility.go: a local live row or
// a stopped row curated in this hub's layout. Remote ids are never resolved via
// desktop lookalikes; federation dispatches this provider on the owning hub.
export function summarySessionVisible(sessionId: string, row: unknown, layout: unknown): boolean {
  const s = row as { session_id?: string; mode?: string; archived?: boolean } | null;
  if (!s || s.session_id !== sessionId || !s.mode || s.mode === 'unknown' || s.archived)
    return false;
  if (['input', 'responding', 'approval', 'question'].includes(s.mode)) return true;
  if (s.mode !== 'stopped') return false;
  const doc = layout as {
    data?: {
      agents?: Array<{
        global?: boolean;
        sessionId?: string;
        lastSessionId?: string;
        tabs?: Array<{ panes?: Array<{ attachSessionId?: string }> }>;
      }>;
    };
  } | null;
  return !!doc?.data?.agents?.some(
    (a) =>
      !a.global &&
      (a.sessionId === sessionId ||
        a.lastSessionId === sessionId ||
        a.tabs?.some((t) => t.panes?.some((p) => p.attachSessionId === sessionId))),
  );
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
      const layout =
        (row as { mode?: string } | null)?.mode === 'stopped' ? await callHub('layout.get') : null;
      if (!summarySessionVisible(sessionId, row, layout))
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
