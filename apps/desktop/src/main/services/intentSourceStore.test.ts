import { DatabaseSync } from 'node:sqlite';
import { mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { afterEach, it, expect, vi } from 'vitest';
import { IntentSourceStore, INTENT_SOURCE_SCHEMA } from './intentSourceStore';
import { sourceSnapshot, type IntentSourceAdapter } from './intentSourceAdapters';
const opened: DatabaseSync[] = [];
afterEach(() => {
  for (const db of opened.splice(0)) db.close();
});
const connection = {
  provider: 'jira',
  url: 'https://team.atlassian.net/browse/TEAM-1',
  credentialEnv: 'WORKSPACER_SOURCE_TEST',
};
const original = sourceSnapshot('r1', 'Title', 'Original requirements', {
  status: { name: 'Review' },
});
function fixture(file = ':memory:') {
  const db = new DatabaseSync(file);
  opened.push(db);
  db.exec(
    'PRAGMA foreign_keys=ON; CREATE TABLE IF NOT EXISTS intent_workspaces(id TEXT PRIMARY KEY,snapshot TEXT NOT NULL);',
  );
  db.exec(INTENT_SOURCE_SCHEMA);
  db.prepare('INSERT OR IGNORE INTO intent_workspaces VALUES(?,?)').run(
    'w',
    JSON.stringify({ revision: 1 }),
  );
  const adapter: IntentSourceAdapter = {
    read: vi.fn().mockResolvedValue({ nativeId: 'TEAM-1', snapshot: original }),
    comment: vi.fn().mockResolvedValue({ status: 'accepted', detail: 'Accepted', remoteId: '9' }),
  };
  let now = Date.now();
  const advance = () => {
    now += 6000;
  };
  const store = new IntentSourceStore(db, adapter, () => now);
  const add = { action: 'addSource', id: 'w', sourceId: 's', expectedRevision: 1, connection };
  const prepare = {
    action: 'prepareSourceComment',
    id: 'w',
    sourceId: 's',
    commentId: 'c',
    expectedRevision: 1,
    expectedVersion: 1,
    text: 'A reviewed user comment',
  };
  const publish = { action: 'publishSourceComment', id: 'w', commentId: 'c', attemptId: 'a' };
  return { db, store, adapter, add, prepare, publish, advance };
}
it('manual sources require no credentials or I/O, preserve notes, and cannot publish', async () => {
  const f = fixture();
  const result = await f.store.request({
    ...f.add,
    connection: { provider: 'manual', url: 'https://example.test/task', credentialEnv: 'ignored' },
    title: 'Source',
    content: 'Requirements',
  });
  expect(result).toMatchObject({
    source: { credentialEnv: '', accepted: { content: 'Requirements' } },
  });
  expect(f.adapter.read).not.toHaveBeenCalled();
  await expect(f.store.request(f.prepare)).rejects.toThrow('do not publish');
});
it('keeps immutable accepted snapshots through refresh/accept and database reopen', async () => {
  const file = path.join(mkdtempSync(path.join(tmpdir(), 'intent-source-')), 'state.db');
  const f = fixture(file);
  await f.store.request(f.add);
  const changed = sourceSnapshot('r2', 'Changed', 'New requirements', { state: 'Different' });
  vi.mocked(f.adapter.read).mockResolvedValue({ nativeId: 'TEAM-1', snapshot: changed });
  f.advance();
  const refresh = await f.store.request({
    action: 'refreshSource',
    id: 'w',
    sourceId: 's',
    expectedVersion: 1,
  });
  expect(refresh).toMatchObject({ source: { accepted: original, candidate: changed, version: 2 } });
  await expect(f.store.request({ ...f.prepare, expectedVersion: 2 })).rejects.toThrow('drift');
  await expect(
    f.store.request({
      action: 'acceptSource',
      id: 'w',
      sourceId: 's',
      expectedVersion: 1,
      candidateDigest: changed.digest,
    }),
  ).rejects.toThrow('Source changed');
  await f.store.request({
    action: 'acceptSource',
    id: 'w',
    sourceId: 's',
    expectedVersion: 2,
    candidateDigest: changed.digest,
  });
  expect(await fixture(file).store.request({ action: 'sources', id: 'w' })).toMatchObject({
    sources: [{ accepted: changed, history: [original], candidate: null }],
  });
});
it('records proposals without publishing and refuses stale intent or source', async () => {
  const f = fixture();
  await f.store.request(f.add);
  await f.store.request(f.prepare);
  expect(f.adapter.comment).not.toHaveBeenCalled();
  f.db.prepare('UPDATE intent_workspaces SET snapshot=?').run('{"revision":2}');
  await expect(f.store.request(f.publish)).rejects.toThrow('Intent changed');
  expect(f.adapter.comment).not.toHaveBeenCalled();
});
it('observed remote drift blocks publishing and saves a reviewable candidate', async () => {
  const f = fixture();
  await f.store.request(f.add);
  await f.store.request(f.prepare);
  vi.mocked(f.adapter.read).mockResolvedValue({
    nativeId: 'TEAM-1',
    snapshot: sourceSnapshot('r2', 'New', 'Drift', {}),
  });
  expect(await f.store.request(f.publish)).toMatchObject({
    comment: { attempts: [{ status: 'failed' }] },
  });
  expect(f.adapter.comment).not.toHaveBeenCalled();
  expect(await f.store.request({ action: 'sources', id: 'w' })).toMatchObject({
    sources: [{ candidate: { revision: 'r2' }, accepted: original }],
  });
});
it('persists unknown before I/O, refuses concurrent/restarted replay, and sends immutable user text', async () => {
  const file = path.join(mkdtempSync(path.join(tmpdir(), 'intent-source-')), 'state.db');
  const f = fixture(file);
  await f.store.request(f.add);
  await f.store.request(f.prepare);
  let resolve!: (value: { status: 'accepted'; detail: string }) => void;
  vi.mocked(f.adapter.comment).mockImplementation(
    () =>
      new Promise((r) => {
        resolve = r;
      }),
  );
  const sending = f.store.request({
    ...f.publish,
    text: 'INJECTED',
    connection: { url: 'https://evil.test' },
  });
  await vi.waitFor(() => expect(f.adapter.comment).toHaveBeenCalled());
  const second = fixture(file);
  expect(await second.store.request({ ...f.publish, attemptId: 'new' })).toMatchObject({
    comment: { attempts: [{ status: 'unknown' }] },
  });
  expect(second.adapter.comment).not.toHaveBeenCalled();
  resolve({ status: 'accepted', detail: 'Accepted' });
  await sending;
  expect(f.adapter.comment).toHaveBeenCalledWith(
    expect.objectContaining(connection),
    'A reviewed user comment',
  );
  await f.store.request({ ...f.publish, attemptId: 'third' });
  expect(f.adapter.comment).toHaveBeenCalledTimes(1);
});
it('treats unexpected post exceptions as uncertain and permits explicit retry only after confirmed refusal', async () => {
  const f = fixture();
  await f.store.request(f.add);
  await f.store.request(f.prepare);
  vi.mocked(f.adapter.comment)
    .mockResolvedValueOnce({ status: 'failed', detail: 'Refused' })
    .mockRejectedValueOnce(new Error('lost response'));
  await f.store.request(f.publish);
  await f.store.request({ ...f.publish, attemptId: 'retry' });
  const response = await f.store.request({ ...f.publish, attemptId: 'again' });
  expect(response).toMatchObject({
    comment: { attempts: [{ status: 'failed' }, { status: 'unknown' }] },
  });
  expect(f.adapter.comment).toHaveBeenCalledTimes(2);
});
it('rejects cross-workspace records and reused identities', async () => {
  const f = fixture();
  await f.store.request(f.add);
  f.db.prepare('INSERT INTO intent_workspaces VALUES(?,?)').run('other', '{"revision":1}');
  await expect(f.store.request({ ...f.prepare, id: 'other' })).rejects.toThrow('does not belong');
  await expect(
    f.store.request({
      ...f.add,
      connection: { ...connection, url: 'https://team.atlassian.net/browse/TEAM-2' },
    }),
  ).rejects.toThrow('different content');
});

it('compiles bounded accepted provenance without candidates or credential references', async () => {
  const f = fixture();
  await f.store.request(f.add);
  vi.mocked(f.adapter.read).mockResolvedValue({
    nativeId: 'TEAM-1',
    snapshot: sourceSnapshot('r2', 'Title', 'UNACCEPTED', {}),
  });
  f.advance();
  await f.store.request({ action: 'refreshSource', id: 'w', sourceId: 's', expectedVersion: 1 });
  const packet = f.store.contextPacket('w');
  expect(packet).toContain('Original requirements');
  expect(packet).toContain(original.digest);
  expect(packet).not.toContain('UNACCEPTED');
  expect(packet).not.toContain('WORKSPACER_SOURCE_TEST');
  expect(packet.length).toBeLessThan(32000);
});
it('does no I/O on failed durable claim and retains uncertainty on failed receipt write', async () => {
  const f = fixture();
  await f.store.request(f.add);
  await f.store.request(f.prepare);
  f.db.exec(
    "CREATE TRIGGER fail_source_claim BEFORE UPDATE ON intent_source_comments BEGIN SELECT RAISE(ABORT, 'disk failure'); END;",
  );
  vi.mocked(f.adapter.read).mockClear();
  await expect(f.store.request(f.publish)).rejects.toThrow('disk failure');
  expect(f.adapter.read).not.toHaveBeenCalled();
  expect(f.adapter.comment).not.toHaveBeenCalled();
  f.db.exec('DROP TRIGGER fail_source_claim');
  vi.mocked(f.adapter.comment).mockImplementation(async () => {
    f.db.exec(
      "CREATE TRIGGER fail_source_receipt BEFORE UPDATE ON intent_source_comments BEGIN SELECT RAISE(ABORT, 'disk failure'); END;",
    );
    return { status: 'accepted', detail: 'Accepted' };
  });
  await expect(f.store.request(f.publish)).rejects.toThrow('receipt could not be saved');
  expect(await f.store.request({ action: 'sources', id: 'w' })).toMatchObject({
    comments: [{ attempts: [{ status: 'unknown' }] }],
  });
});
