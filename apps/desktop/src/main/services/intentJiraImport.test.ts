import { DatabaseSync } from 'node:sqlite';
import path from 'node:path';
import { afterEach, expect, it, vi } from 'vitest';
import { IntentWorkspaceStore } from './intentWorkspaceStore';
import { sourceSnapshot } from './intentSourceAdapters';
vi.mock('./configService', () => ({ getConfigDir: () => '/unused-by-store-tests' }));
const opened: DatabaseSync[] = [];
afterEach(() => {
  vi.restoreAllMocks();
  for (const db of opened.splice(0)) db.close();
});
function setup() {
  const db = new DatabaseSync(':memory:');
  opened.push(db);
  const store = new IntentWorkspaceStore(db);
  const projectRoot = path.resolve('/project');
  const seed = store.request({
    action: 'create',
    projectRoot,
    fields: {
      title: 'Existing intent',
      outcome: '',
      constraints: '',
      successCriteria: '',
      sourceUrl: '',
      status: 'draft',
    },
  });
  if (seed.action !== 'create') throw new Error('create failed');
  store.sources.integrations.request({
    action: 'saveIntegration',
    id: seed.workspace.id,
    integrationId: 'jira',
    operationId: 'configure',
    expectedVersion: 0,
    integration: {
      provider: 'jira',
      name: 'Team Jira',
      baseUrl: 'https://team.atlassian.net',
      defaultProjectKey: 'TEAM',
      repository: '',
      credentialEnv: 'WORKSPACER_SOURCE_TEAM',
      enabled: true,
    },
  });
  const read = {
    nativeId: 'TEAM-123',
    snapshot: sourceSnapshot(
      'jira-rev',
      'Export CSV',
      'Only export filtered rows.\nPreserve permissions.',
      { status: { name: 'Done' } },
    ),
  };
  const fetch = vi.spyOn(store.sources.sync, 'import').mockResolvedValue(read);
  const input = {
    action: 'importJiraIntent',
    projectRoot,
    operationId: 'import-one',
    integrationId: 'jira',
    expectedIntegrationVersion: 1,
    identifier: 'TEAM-123',
  };
  const count = () => Number(db.prepare('SELECT count(*) AS n FROM intent_workspaces').get()!.n);
  return { db, store, projectRoot, seed: seed.workspace, read, fetch, input, count };
}
it('imports a Draft and pinned source together, then replays without another fetch or revision', async () => {
  const f = setup();
  expect(f.store.request({ action: 'jiraIntegrations', projectRoot: f.projectRoot })).toMatchObject(
    { integrations: [{ id: 'jira', version: 1 }] },
  );
  const imported = await f.store.importJira(f.input);
  expect(imported.workspace).toMatchObject({
    title: 'Export CSV',
    outcome: f.read.snapshot.content,
    constraints: '',
    successCriteria: '',
    sourceUrl: 'https://team.atlassian.net/browse/TEAM-123',
    status: 'draft',
    revision: 1,
  });
  expect(imported.source).toMatchObject({
    workspaceId: imported.workspace.id,
    accepted: f.read.snapshot,
    integration: { id: 'jira', version: 1 },
    candidate: null,
  });
  expect(await f.store.importJira(f.input)).toEqual(imported);
  expect(f.fetch).toHaveBeenCalledTimes(1);
  expect(f.count()).toBe(2);
  expect(f.db.prepare('SELECT count(*) AS n FROM intent_runs').get()!.n).toBe(0);
  await expect(f.store.importJira({ ...f.input, identifier: 'TEAM-124' })).rejects.toThrow(
    'already used',
  );
  expect(
    await f.store.sources.request({ action: 'sources', id: imported.workspace.id }),
  ).toMatchObject({ sources: [{ id: imported.source.id }] });
});
it('accepts a browse URL only on the selected Jira site', async () => {
  const f = setup();
  await expect(
    f.store.importJira({ ...f.input, identifier: 'https://other.atlassian.net/browse/TEAM-123' }),
  ).rejects.toThrow('selected connection');
  expect(f.fetch).not.toHaveBeenCalled();
  await f.store.importJira({
    ...f.input,
    identifier: 'https://team.atlassian.net/browse/TEAM-123',
  });
  expect(f.fetch).toHaveBeenCalledWith(
    expect.objectContaining({ url: 'https://team.atlassian.net/browse/TEAM-123' }),
  );
});
it('rejects stale or cross-project connections before fetching', async () => {
  const f = setup();
  await expect(f.store.importJira({ ...f.input, expectedIntegrationVersion: 2 })).rejects.toThrow(
    'changed',
  );
  await expect(
    f.store.importJira({ ...f.input, projectRoot: path.resolve('/another') }),
  ).rejects.toThrow('enabled Jira');
  expect(f.fetch).not.toHaveBeenCalled();
  expect(f.count()).toBe(1);
});
it('leaves no intent on Jira failure or a connection change during the fetch', async () => {
  const f = setup();
  f.fetch.mockRejectedValueOnce(new Error('Jira unavailable'));
  await expect(f.store.importJira(f.input)).rejects.toThrow('Jira unavailable');
  expect(f.count()).toBe(1);
  f.fetch.mockImplementationOnce(async () => {
    const connection = f.store.sources.integrations.list(f.seed.id)[0];
    f.store.sources.integrations.request({
      action: 'saveIntegration',
      id: f.seed.id,
      integrationId: 'jira',
      operationId: 'disable',
      expectedVersion: 1,
      integration: { ...connection, enabled: false },
    });
    return f.read;
  });
  await expect(f.store.importJira(f.input)).rejects.toThrow('enabled Jira');
  expect(f.count()).toBe(1);
});
it('rolls back the entire import when saving the source fails', async () => {
  const f = setup();
  vi.spyOn(f.store.sources.sync, 'record').mockImplementationOnce(() => {
    throw new Error('storage failure');
  });
  await expect(f.store.importJira(f.input)).rejects.toThrow('storage failure');
  expect(f.count()).toBe(1);
  expect(f.db.prepare('SELECT count(*) AS n FROM intent_sources').get()!.n).toBe(0);
  expect(f.db.prepare('SELECT count(*) AS n FROM intent_jira_imports').get()!.n).toBe(0);
  await f.store.importJira(f.input);
  expect(f.count()).toBe(2);
});
