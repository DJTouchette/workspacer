import { DatabaseSync } from 'node:sqlite';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { afterEach, expect, it, vi } from 'vitest';
import { IntentSourceStore, INTENT_SOURCE_SCHEMA } from './intentSourceStore';
import { sourceSnapshot, sourceConnection } from './intentSourceAdapters';
import {
  normalizeIntegration,
  resolveIntegration,
  type IntentIntegrationDraft,
} from '../shared/intentIntegrations';
import type { IntentSourceConnection } from '../shared/intentSources';
vi.mock('./configService', () => ({ getConfigDir: () => '/unused-integration-tests' }));
import { IntentWorkspaceStore, INTENT_WORKSPACE_SCHEMA_VERSION } from './intentWorkspaceStore';
const opened: DatabaseSync[] = [],
  dirs: string[] = [];
afterEach(() => {
  for (const db of opened.splice(0)) if (db.isOpen) db.close();
  for (const dir of dirs.splice(0)) rmSync(dir, { recursive: true, force: true });
});
const jira: IntentIntegrationDraft = {
  provider: 'jira',
  name: 'Team Jira',
  baseUrl: 'https://team.atlassian.net/',
  defaultProjectKey: 'team',
  repository: '',
  credentialEnv: 'WORKSPACER_SOURCE_FIXTURE',
  enabled: true,
};
const ado: IntentIntegrationDraft = {
  ...jira,
  provider: 'ado',
  name: 'Azure',
  baseUrl: 'https://dev.azure.com/org/My%20Project',
  repository: 'Repo One',
};
function fixture(file = ':memory:') {
  const db = new DatabaseSync(file);
  opened.push(db);
  db.exec(
    'CREATE TABLE IF NOT EXISTS intent_workspaces(id TEXT PRIMARY KEY,snapshot TEXT NOT NULL);' +
      INTENT_SOURCE_SCHEMA,
  );
  for (const [id, projectRoot] of [
    ['w', '/project'],
    ['w2', '/project'],
    ['other', '/other'],
  ])
    db.prepare('INSERT OR IGNORE INTO intent_workspaces VALUES(?,?)').run(
      id,
      JSON.stringify({ revision: 1, projectRoot, status: 'draft' }),
    );
  let now = Date.now();
  const sync = vi.fn(async (c: IntentSourceConnection) => ({
    nativeId: 'TEAM-1',
    snapshot: sourceSnapshot('r1', 'Requirement', 'Original', {}),
    projection: {
      objectType: 'issue' as const,
      nativeId: 'TEAM-1',
      url: c.url,
      state: 'active',
      revision: 'r1',
      summary: {},
    },
    payload: { safe: 'data' },
  }));
  const store = new IntentSourceStore(db, { sync, read: vi.fn(), comment: vi.fn() }, () => now);
  const save = (
    integration = jira,
    integrationId = 'j',
    expectedVersion = 0,
    operationId = `save-${integrationId}-${expectedVersion}`,
  ) =>
    store.request({
      action: 'saveIntegration',
      id: 'w',
      integrationId,
      expectedVersion,
      operationId,
      integration,
    });
  const attach = {
    action: 'attachSource',
    id: 'w',
    sourceId: 's',
    expectedRevision: 1,
    reference: {
      integrationId: 'j',
      expectedIntegrationVersion: 1,
      objectType: 'issue',
      identifier: ' team-1 ',
    },
  };
  return {
    db,
    store,
    sync,
    save,
    attach,
    advance: () => {
      now += 301000;
    },
  };
}
it('CRUD is project scoped, multi-connection, CAS guarded and idempotent including removal', async () => {
  const f = fixture();
  const first = await f.save();
  expect(await f.save()).toEqual(first);
  await f.save(ado, 'a');
  expect(await f.store.request({ action: 'integrations', id: 'w2' })).toMatchObject({
    integrations: [
      { id: 'j', version: 1 },
      { id: 'a', version: 1 },
    ],
  });
  expect(await f.store.request({ action: 'integrations', id: 'other' })).toMatchObject({
    integrations: [],
  });
  await expect(
    f.store.request({
      action: 'saveIntegration',
      id: 'other',
      integrationId: 'j',
      expectedVersion: 1,
      operationId: 'cross',
      integration: jira,
    }),
  ).rejects.toThrow('does not belong');
  await expect(f.save({ ...jira, name: 'Changed' })).rejects.toThrow('Operation ID');
  await expect(f.save(jira, 'j', 0, 'stale')).rejects.toThrow('Integration changed');
  await f.save({ ...jira, name: 'Changed' }, 'j', 1);
  const remove = {
    action: 'removeIntegration',
    id: 'w',
    integrationId: 'j',
    expectedVersion: 2,
    operationId: 'remove',
  };
  const removed = await f.store.request(remove);
  expect(await f.store.request(remove)).toEqual(removed);
  expect(await f.store.request({ action: 'integrations', id: 'w' })).toMatchObject({
    integrations: [{ id: 'a' }],
  });
  await expect(f.save(jira, 'j', 3)).rejects.toThrow('Integration changed');
});
it('normalizes identifiers, constructs provider URLs and requires an unambiguous PR repository', () => {
  const issue = resolveIntegration(jira, { objectType: 'issue', identifier: ' team-123 ' });
  expect(issue.url).toBe('https://team.atlassian.net/browse/TEAM-123');
  expect(resolveIntegration(jira, { objectType: 'issue', identifier: '123' })).toEqual(issue);
  expect(sourceConnection(issue)).toEqual(issue);
  const pr = resolveIntegration(ado, { objectType: 'pull-request', identifier: '12' });
  expect(pr.url).toBe('https://dev.azure.com/org/My%20Project/_git/Repo%20One/pullrequest/12');
  expect(sourceConnection(pr)).toEqual(pr);
  expect(resolveIntegration(ado, { objectType: 'work-item', identifier: '12' }).url).toBe(
    'https://dev.azure.com/org/My%20Project/_workitems/edit/12',
  );
  expect(() =>
    resolveIntegration(
      { ...ado, repository: '' },
      { objectType: 'pull-request', identifier: '12' },
    ),
  ).toThrow('repository is required');
  expect(
    resolveIntegration(
      { ...ado, repository: '' },
      { objectType: 'pull-request', identifier: 'repo#12' },
    ).url,
  ).toContain('/_git/repo/pullrequest/12');
  expect(() =>
    resolveIntegration(ado, {
      objectType: 'pull-request',
      identifier: 'repo#12',
      repository: 'other',
    }),
  ).toThrow('Ambiguous');
  for (const identifier of ['0', '-1', '1/2', '../12', '12?x', '1%2f2', '12#13#14', '12\n3'])
    expect(() => resolveIntegration(ado, { objectType: 'work-item', identifier })).toThrow();
});
it.each([
  'http://team.atlassian.net',
  'https://user:pass@team.atlassian.net',
  'https://team.atlassian.net:443',
  'https://team.atlassian.net:444',
  'https://team.atlassian.net.evil.test',
  'https://team.atlassian.net/?token=x',
  'https://team.atlassian.net/#x',
  'https://team.atlassian.net/./',
  'https://team.atlassian.net\\@evil.test',
])('rejects unsafe Jira base %s', (baseUrl) => {
  expect(() => normalizeIntegration({ ...jira, baseUrl })).toThrow();
});
it.each([
  'https://dev.azure.com/org/..',
  'https://dev.azure.com/org/%2e%2e',
  'https://dev.azure.com/org/a%2fb',
  'https://dev.azure.com/org/a%5cb',
  'https://dev.azure.com/org/a%00b',
  'https://dev.azure.com/org/a%252fb',
  'https://dev.azure.com/org/project/_git/repo',
])('rejects unsafe Azure path %s', (baseUrl) => {
  expect(() => normalizeIntegration({ ...ado, baseUrl })).toThrow();
});
it('pins imports, credential boundaries, artifacts and automatic refresh across edits, disable and restart', async () => {
  const dir = mkdtempSync(path.join(tmpdir(), 'intent-integrations-'));
  dirs.push(dir);
  const file = path.join(dir, 'state.sqlite');
  const f = fixture(file);
  await f.save({ ...jira, password: 'must-not-persist' } as IntentIntegrationDraft);
  const before = f.db.prepare('SELECT snapshot FROM intent_workspaces WHERE id=?').get('w');
  const imported = await f.store.request(f.attach);
  const resolved = sourceConnection(
    resolveIntegration(jira, { objectType: 'issue', identifier: 'TEAM-1' }),
  );
  expect(f.sync).toHaveBeenLastCalledWith(resolved, undefined);
  expect(imported).toMatchObject({
    action: 'attachSource',
    source: { ...resolved, integration: { id: 'j', version: 1 }, external: { status: 'fresh' } },
  });
  f.advance();
  await f.store.request({
    action: 'addSource',
    id: 'w2',
    sourceId: 'legacy',
    expectedRevision: 1,
    connection: resolved,
  });
  expect(f.sync).toHaveBeenLastCalledWith(resolved, undefined);
  expect(await f.store.request({ action: 'integrations', id: 'w' })).toMatchObject({
    integrations: [{ references: 1 }],
  });
  await expect(
    f.store.request({
      action: 'removeIntegration',
      id: 'w',
      integrationId: 'j',
      expectedVersion: 1,
      operationId: 'rm',
    }),
  ).rejects.toThrow('referenced');
  await f.save(
    {
      ...jira,
      baseUrl: 'https://other.atlassian.net',
      credentialEnv: 'WORKSPACER_SOURCE_OTHER',
      enabled: false,
    },
    'j',
    1,
  );
  expect(await f.store.request(f.attach)).toEqual(JSON.parse(JSON.stringify(imported)));
  await expect(f.store.request({ ...f.attach, sourceId: 'new' })).rejects.toThrow('disabled');
  await expect(
    f.store.request({ ...f.attach, reference: { ...f.attach.reference, identifier: 'TEAM-2' } }),
  ).rejects.toThrow('already used');
  expect(f.db.prepare('SELECT snapshot FROM intent_workspaces WHERE id=?').get('w')).toEqual(
    before,
  );
  expect(JSON.stringify(f.db.prepare('SELECT * FROM intent_integrations').all())).not.toContain(
    'must-not-persist',
  );
  expect(f.store.contextPacket('w')).not.toContain('WORKSPACER_SOURCE_');
  f.db.close();
  const reopened = fixture(file);
  reopened.advance();
  reopened.advance();
  await reopened.store.tick();
  expect(reopened.sync.mock.calls.filter(([c]) => c.url === resolved.url)).toHaveLength(1);
  for (const [c] of reopened.sync.mock.calls) expect(c.credentialEnv).toBe(resolved.credentialEnv);
  expect(reopened.store.sync.artifacts('s')).toHaveLength(1);
});
it('rejects an integration edit racing import before persisting the source', async () => {
  const f = fixture();
  await f.save();
  const original = f.sync.getMockImplementation()!;
  f.sync.mockImplementation(async (c) => {
    await f.save({ ...jira, credentialEnv: 'WORKSPACER_SOURCE_CHANGED' }, 'j', 1);
    return original(c);
  });
  await expect(f.store.request(f.attach)).rejects.toThrow('Integration changed');
  expect(await f.store.request({ action: 'sources', id: 'w' })).toMatchObject({ sources: [] });
});
it('migrates a version 7 store and preserves legacy source records', () => {
  const db = new DatabaseSync(':memory:');
  opened.push(db);
  new IntentWorkspaceStore(db);
  db.exec(
    'DROP TABLE intent_integration_operations; DROP TABLE intent_integrations; PRAGMA user_version=7;',
  );
  new IntentWorkspaceStore(db);
  expect(db.prepare('PRAGMA user_version').get()?.user_version).toBe(
    INTENT_WORKSPACE_SCHEMA_VERSION,
  );
  expect(db.prepare('SELECT count(*) AS count FROM intent_integrations').get()?.count).toBe(0);
});
