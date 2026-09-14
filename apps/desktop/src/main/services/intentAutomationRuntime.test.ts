import { afterEach, expect, it, vi } from 'vitest';
import { DatabaseSync } from 'node:sqlite';
vi.mock('./intentWorkspaceStore', () => ({
  intentWorkspaceStoreIfUsed: vi.fn(),
  captureIntentSessions: vi.fn(() => []),
}));
import { intentWorkspaceStoreIfUsed } from './intentWorkspaceStore';
import { startIntentAutomationRuntime } from './intentAutomationRuntime';
import { IntentSourceStore, INTENT_SOURCE_SCHEMA } from './intentSourceStore';
import { createIntentSourceAdapter } from './intentSourceAdapters';
import { azurePr, jiraIssue } from '../../../tests/fixtures/intentSources';
import type { IntentAutomationEffects } from './intentAutomationStore';
afterEach(() => vi.useRealTimers());
it('owner runtime automatically refreshes linked ADO and Jira with mocked HTTP and stops scheduling on shutdown', async () => {
  vi.useFakeTimers();
  const db = new DatabaseSync(':memory:');
  db.exec(
    'CREATE TABLE intent_workspaces(id TEXT PRIMARY KEY,snapshot TEXT NOT NULL);' +
      INTENT_SOURCE_SCHEMA,
  );
  db.prepare('INSERT INTO intent_workspaces VALUES(?,?)').run('w', '{"revision":1}');
  let phase = 1;
  const fetcher = vi.fn(async (url: string | URL | Request) => {
    const u = String(url);
    if (u.endsWith('/pullRequests/12?api-version=7.1'))
      return new Response(
        JSON.stringify({ ...azurePr, status: phase === 1 ? 'active' : 'completed' }),
      );
    if (u.includes('?fields=*all'))
      return new Response(
        JSON.stringify({
          ...jiraIssue,
          fields: {
            ...jiraIssue.fields,
            updated: `r${phase}`,
            status: { name: phase === 1 ? 'Open' : 'Done' },
          },
        }),
      );
    return new Response(JSON.stringify({ value: [], values: [], comments: [], total: 0 }));
  });
  const sources = new IntentSourceStore(
    db,
    createIntentSourceAdapter(fetcher, { WORKSPACER_SOURCE_TEST: 'user:fake-token' }),
  );
  let stop: (() => void) | undefined;
  try {
    for (const [sourceId, provider, url] of [
      ['j', 'jira', 'https://team.atlassian.net/browse/TEAM-1'],
      ['a', 'ado', 'https://dev.azure.com/org/project/_git/repo/pullrequest/12'],
    ]) {
      await sources.request({
        action: 'addSource',
        id: 'w',
        sourceId,
        expectedRevision: 1,
        connection: { provider, url, credentialEnv: 'WORKSPACER_SOURCE_TEST' },
      });
    }
    const automation = { tick: vi.fn() };
    vi.mocked(intentWorkspaceStoreIfUsed).mockResolvedValue({
      sources,
      automation,
      capture: vi.fn(),
    } as never);
    stop = startIntentAutomationRuntime(() => [], {} as IntentAutomationEffects);
    await vi.advanceTimersByTimeAsync(10000);
    const initialCalls = fetcher.mock.calls.length;
    phase = 2;
    await vi.advanceTimersByTimeAsync(300000);
    const response = await sources.request({ action: 'sources', id: 'w' });
    expect(response).toMatchObject({
      sources: expect.arrayContaining([
        expect.objectContaining({
          id: 'a',
          external: expect.objectContaining({
            projection: expect.objectContaining({ state: 'completed' }),
          }),
        }),
        expect.objectContaining({
          id: 'j',
          external: expect.objectContaining({
            projection: expect.objectContaining({ state: 'Done' }),
          }),
        }),
      ]),
    });
    expect(fetcher.mock.calls.length).toBeGreaterThan(initialCalls);
    expect(automation.tick).toHaveBeenCalled();
    stop();
    const stoppedCalls = fetcher.mock.calls.length;
    await vi.advanceTimersByTimeAsync(600000);
    expect(fetcher).toHaveBeenCalledTimes(stoppedCalls);
  } finally {
    stop?.();
    db.close();
  }
});
