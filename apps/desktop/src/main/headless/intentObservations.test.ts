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
      projection: 'intent-completion-source/v1',
      redactionVersion: 1,
      redacted: false,
      sessionId: 'worker',
      text,
      truncated: false,
      interrupted: false,
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
    'http://daemon/sessions/worker/conversation?completion_source=1',
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
          ? new Response('x'.repeat(26001))
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

it('preserves the same bounded final assistant text as native capture, including reports longer than 800 characters', async () => {
  const { captureIntentSessions } = await import('../services/intentWorkspaceStore');
  const text = 'Verbatim report\n' + 'checked '.repeat(200);
  fetchMock.mockImplementation(async () => projection(text));
  const capture = vi.spyOn(store, 'capture');
  await captureHeadlessIntentSessions([{ ...session, ambientState: 'idle' }], 'http://daemon');
  const headless = capture.mock.calls.at(-1)![0][0];
  const native = captureIntentSessions([
    { ...session, ambientState: 'idle', conversation: [{ role: 'assistant', content: text }] },
  ])[0];
  expect(headless.finalReport).toEqual(native.finalReport);
  expect(headless.completionIdle).toEqual(native.completionIdle);
  expect(headless.observation.summary).toEqual(native.observation.summary);
});

it('rejects pre-redaction projections instead of persisting an irreparable boundary fragment', async () => {
  fetchMock.mockImplementation(
    async () =>
      new Response(
        JSON.stringify({
          projection: 'intent-completion-source/v1',
          sessionId: 'worker',
          text: '.'.repeat(3992) + 'sk-leake',
          truncated: true,
          interrupted: false,
        }),
      ),
  );
  await captureHeadlessIntentSessions([{ ...session, ambientState: 'idle' }], 'http://daemon');
  const result = store.request({ action: 'executions', id });
  expect(result).toHaveProperty('captureWarning');
  expect(JSON.stringify(result)).not.toContain('sk-leake');
});

it('retains daemon redaction provenance and does not truncate a sanitized projection twice', async () => {
  const { captureIntentSessions } = await import('../services/intentWorkspaceStore');
  const raw = 'password=' + 's'.repeat(5000) + '\nDone 😀';
  const native = captureIntentSessions([
    { ...session, conversation: [{ role: 'assistant', content: raw }] },
  ])[0];
  fetchMock.mockImplementation(
    async () =>
      new Response(
        JSON.stringify({
          projection: 'intent-completion-source/v1',
          sessionId: 'worker',
          redactionVersion: 1,
          ...native.finalReport,
        }),
      ),
  );
  const capture = vi.spyOn(store, 'capture');
  await captureHeadlessIntentSessions([session], 'http://daemon');
  const headless = capture.mock.calls.at(-1)![0][0];
  expect(headless.finalReport).toEqual(native.finalReport);
  expect(headless.finalReport).toMatchObject({
    redacted: true,
    truncated: false,
    text: 'password=[redacted]\nDone 😀',
  });
  expect(headless.observation.summary).toEqual(native.observation.summary);
});
