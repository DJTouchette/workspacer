import { DatabaseSync } from 'node:sqlite';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
vi.mock('../services/configService', () => ({ getConfigDir: () => '/unused-intent-tests' }));
vi.mock('../services/intentWorkspaceStore', async (original) => ({
  ...(await original<typeof import('../services/intentWorkspaceStore')>()),
  intentWorkspaceStoreIfUsed: vi.fn(),
}));
import { IntentWorkspaceStore, intentWorkspaceStoreIfUsed } from '../services/intentWorkspaceStore';
import { captureHeadlessIntentSessions } from './intentObservations';

let db: DatabaseSync, store: IntentWorkspaceStore, id: string;
const session = {
  sessionId: 'worker',
  hub: '',
  cwd: '/project',
  label: 'Worker',
  provider: 'codex',
};
const fetchMock = vi.fn();
const projection = (text: string) =>
  new Response(
    JSON.stringify({
      projection: 'agent-status-source/v1',
      sessionId: 'worker',
      events: [{ kind: 'assistant_text', text }],
    }),
  );
beforeEach(() => {
  db = new DatabaseSync(':memory:');
  store = new IntentWorkspaceStore(db);
  vi.mocked(intentWorkspaceStoreIfUsed).mockResolvedValue(store);
  const created = store.request({
    action: 'create',
    projectRoot: '/project',
    fields: {
      title: 'Feature',
      outcome: '',
      constraints: '',
      successCriteria: '',
      sourceUrl: '',
      status: 'draft',
    },
  });
  if (created.action !== 'create') throw new Error('Expected workspace');
  id = created.workspace.id;
  store.request({ action: 'attachSession', id, expectedRevision: 1, session });
  fetchMock.mockReset().mockImplementation(async () => projection('Final report excerpt'));
  vi.stubGlobal('fetch', fetchMock);
});
afterEach(() => {
  db.close();
  vi.unstubAllGlobals();
});

it('captures sparse local reports without browser transcript demand', async () => {
  await captureHeadlessIntentSessions([{ ...session, status: 'ended' }], 'http://daemon');
  expect(fetchMock).toHaveBeenCalledWith(
    'http://daemon/sessions/worker/conversation?summary_source=1',
    expect.anything(),
  );
  expect(store.request({ action: 'executions', id })).toMatchObject({
    executions: [{ lastObservation: { state: 'stopped', summary: 'Final report excerpt' } }],
  });
});

it('never resolves unknown, peer, or offline session IDs against the local daemon', async () => {
  const peer = { ...session, hub: 'peer' };
  store.request({ action: 'attachSession', id, expectedRevision: 1, session: peer });
  await captureHeadlessIntentSessions(
    [{ ...session, hubOffline: true }, peer, { ...session, sessionId: 'unlinked' }],
    'http://daemon',
  );
  expect(fetchMock).not.toHaveBeenCalled();
  vi.mocked(intentWorkspaceStoreIfUsed).mockResolvedValue(undefined);
  await captureHeadlessIntentSessions([session], 'http://daemon');
  expect(fetchMock).not.toHaveBeenCalled();
});

it.each(['old daemon', 'oversized', 'failed request'])(
  'keeps old reports and exposes %s failures until a successful retry',
  async (failure) => {
    await captureHeadlessIntentSessions([{ ...session, ambientState: 'idle' }], 'http://daemon');
    fetchMock.mockImplementation(async () =>
      failure === 'old daemon'
        ? new Response(JSON.stringify({ items: [] }))
        : failure === 'oversized'
          ? new Response('x'.repeat(5001))
          : new Response('', { status: 503 }),
    );
    await captureHeadlessIntentSessions([{ ...session, status: 'ended' }], 'http://daemon');
    expect(store.request({ action: 'executions', id })).toMatchObject({
      captureWarning: expect.stringContaining('Background agent report capture failed'),
      executions: [{ lastObservation: { state: 'stopped', summary: 'Final report excerpt' } }],
    });
    await captureHeadlessIntentSessions([], 'http://daemon');
    expect(store.request({ action: 'executions', id })).toHaveProperty('captureWarning');
    fetchMock.mockImplementation(async () => projection('Recovered final report'));
    await captureHeadlessIntentSessions([{ ...session, status: 'ended' }], 'http://daemon');
    const result = store.request({ action: 'executions', id });
    expect(result).not.toHaveProperty('captureWarning');
    expect(result).toMatchObject({
      executions: [{ lastObservation: { summary: 'Recovered final report' } }],
    });
  },
);
