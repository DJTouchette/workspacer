import { it, expect, vi, afterEach } from 'vitest';
vi.mock('./configService', () => ({
  configService: {
    getConfig: () => ({
      agents: { statusSummary: { enabled: true, provider: 'claude', model: 'haiku' } },
    }),
    onChange: vi.fn(),
  },
}));
vi.mock('./claudemonDaemon', () => ({ CLAUDEMON_API_URL: 'http://127.0.0.1:0' }));
vi.mock('./hubClient', () => ({ callHub: vi.fn() }));
vi.mock('./directCompletion', () => ({
  complete: vi.fn(),
  completionSupported: () => true,
  servesModel: () => true,
}));
import {
  readSummarySource,
  summarySessionVisible,
  createAgentStatusSummaryService,
} from './agentStatusSummaryRuntime';
import { complete } from './directCompletion';
afterEach(() => vi.unstubAllGlobals());
it('visibility requires an owning local row; stopped rows need curation', () => {
  expect(summarySessionVisible('s', { session_id: 'lookalike', mode: 'responding' }, null)).toBe(
    false,
  );
  expect(
    summarySessionVisible(
      's',
      {
        projection: 'agent-status-access/v1',
        archived: false,
        session_id: 's',
        mode: 'responding',
      },
      null,
    ),
  ).toBe(true);
  expect(summarySessionVisible('s', { session_id: 's', mode: 'unknown' }, null)).toBe(false);
  expect(summarySessionVisible('s', { session_id: 's', mode: 'stopped' }, null)).toBe(false);
  expect(
    summarySessionVisible(
      's',
      { session_id: 's', mode: 'stopped' },
      { data: { agents: [{ lastSessionId: 's' }] } },
    ),
  ).toBe(true);
  expect(
    summarySessionVisible(
      's',
      { session_id: 's', mode: 'stopped' },
      { data: { agents: [{ global: true, lastSessionId: 's' }] } },
    ),
  ).toBe(false);
});
it('rejects an old daemon ignoring the projection before the model, even for a small transcript', async () => {
  const fetchMock = vi.fn(
    async (url: string) =>
      new Response(
        JSON.stringify(
          url.includes('conversation')
            ? { seq: 1, items: [{ content: 'SECRET' }] }
            : {
                projection: 'agent-status-access/v1',
                archived: false,
                session_id: 's',
                mode: 'responding',
              },
        ),
      ),
  );
  vi.stubGlobal('fetch', fetchMock);
  expect(await createAgentStatusSummaryService().summarize('s')).toMatchObject({
    reason: 'source-unavailable',
  });
  expect(complete).not.toHaveBeenCalled();
});
it('cancels oversized HTTP bodies before JSON parsing', async () => {
  const cancel = vi.fn();
  let sent = false;
  const body = new ReadableStream({
    pull(c) {
      if (!sent) {
        sent = true;
        c.enqueue(new TextEncoder().encode('x'.repeat(6000)));
      }
    },
    cancel,
  });
  vi.stubGlobal(
    'fetch',
    vi.fn(async () => new Response(body)),
  );
  expect(await readSummarySource('s')).toBeNull();
  expect(cancel).toHaveBeenCalled();
});
it('a disconnected owning daemon returns unavailable, with no model call', async () => {
  vi.stubGlobal(
    'fetch',
    vi.fn(async () => {
      throw new Error('private network detail');
    }),
  );
  expect(await createAgentStatusSummaryService().summarize('s')).toMatchObject({
    reason: 'source-unavailable',
  });
  expect(complete).not.toHaveBeenCalled();
});
