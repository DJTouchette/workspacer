import { DatabaseSync } from 'node:sqlite';
import fs from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { afterEach, expect, it, vi } from 'vitest';
vi.mock('./configService', () => ({ getConfigDir: () => '/unused-intent-artifact-tests' }));
import { IntentWorkspaceStore } from './intentWorkspaceStore';
import { IntentArtifactStore, INTENT_ARTIFACT_SCHEMA } from './intentArtifactStore';
import { INTENT_ARTIFACT_LIMITS, type IntentArtifactResponse } from '../shared/intentArtifacts';
const roots: string[] = [];
const databases: DatabaseSync[] = [];
afterEach(() => {
  vi.restoreAllMocks();
  for (const db of databases.splice(0)) if (db.isOpen) db.close();
  for (const root of roots.splice(0)) fs.rmSync(root, { recursive: true, force: true });
});

it('never holds a SQLite write lock while saving bytes and rejects a concurrent owner revision before committing metadata', () => {
  const f = fixture();
  const other = open(f.filename);
  other.db.exec('PRAGMA busy_timeout=0');
  const write = fs.writeFileSync.bind(fs);
  let changed = false;
  vi.spyOn(fs, 'writeFileSync').mockImplementation(((
    file: Parameters<typeof fs.writeFileSync>[0],
    ...args: unknown[]
  ) => {
    if (!changed && typeof file === 'number') {
      changed = true;
      other.workspaces.request({
        action: 'update',
        id: f.workspace.id,
        expectedRevision: 1,
        fields: { ...f.workspace, title: 'Concurrent owner edit' },
        reason: 'While artifact bytes are written',
      });
    }
    return (write as (...values: unknown[]) => void)(file, ...args);
  }) as typeof fs.writeFileSync);
  expect(() => f.store.request(f.upload)).toThrow('changed elsewhere');
  expect(changed).toBe(true);
  expect(f.store.request({ action: 'artifacts', id: f.workspace.id })).toMatchObject({
    artifacts: [],
  });
  expect(fs.readdirSync(path.join(f.root, 'intent-artifacts'))).toEqual([]);
});
function open(filename: string) {
  const db = new DatabaseSync(filename);
  databases.push(db);
  const workspaces = new IntentWorkspaceStore(db);
  db.exec(INTENT_ARTIFACT_SCHEMA);
  return { db, workspaces, store: new IntentArtifactStore(db) };
}
function fixture() {
  const root = fs.mkdtempSync(path.join(tmpdir(), 'intent-artifacts-'));
  roots.push(root);
  const filename = path.join(root, 'intents.sqlite');
  const opened = open(filename);
  const result = opened.workspaces.request({
    action: 'create',
    projectRoot: '/not/read',
    fields: {
      title: 'Export',
      outcome: 'CSV',
      successCriteria: 'Works offline',
      constraints: '',
      sourceUrl: '',
      status: 'active',
    },
  });
  if (result.action !== 'create') throw new Error('Expected workspace');
  const workspace = result.workspace;
  const upload = {
    action: 'addArtifact',
    id: workspace.id,
    expectedRevision: 1,
    artifactId: 'text-one',
    title: 'Notes',
    mimeType: 'text/markdown',
    dataBase64: Buffer.from('# Snapshot\n').toString('base64'),
    criterionId: 'r1:c1',
  };
  return { root, filename, workspace, upload, ...opened };
}
const artifact = (result: IntentArtifactResponse) => {
  if (result.action !== 'addArtifact') throw new Error('Expected artifact');
  return result.artifact;
};
const png = Buffer.from('89504e470d0a1a0a0000000049454e44ae426082', 'hex').toString('base64');
it('retains immutable bytes, version chain, criterion snapshot and annotations across restart', () => {
  const f = fixture();
  const first = artifact(
    f.store.request({ ...f.upload, author: 'agent', sha256: 'fake', cwd: '/etc' }),
  );
  expect(first).toMatchObject({
    author: 'user',
    criterion: { id: 'r1:c1', text: 'Works offline' },
    kind: 'text',
  });
  expect(first.sha256).toHaveLength(64);
  expect(f.store.request(f.upload)).toMatchObject({ artifact: first });
  expect(() => f.store.request({ ...f.upload, title: 'Mutated' })).toThrow('different content');
  const annotation = {
    action: 'annotateArtifact',
    id: f.workspace.id,
    expectedRevision: 1,
    annotationId: 'note',
    artifactId: first.id,
    artifactSha256: first.sha256,
    text: 'Check this result.',
  };
  f.store.request(annotation);
  const next = artifact(
    f.store.request({
      ...f.upload,
      artifactId: 'text-two',
      versionOf: first.id,
      dataBase64: Buffer.from('Second version').toString('base64'),
    }),
  );
  f.db.close();
  const reopened = open(f.filename).store;
  expect(
    reopened.request({ action: 'readArtifact', id: f.workspace.id, artifactId: first.id }),
  ).toMatchObject({ dataBase64: f.upload.dataBase64 });
  expect(reopened.request({ action: 'artifacts', id: f.workspace.id })).toMatchObject({
    artifacts: [expect.objectContaining({ id: next.id, versionOf: first.id }), first],
    annotations: [expect.objectContaining({ artifactId: first.id, artifactSha256: first.sha256 })],
  });
  expect(
    fs.readFileSync(path.join(f.root, 'intent-artifacts', first.contentId + '.bin'), 'utf8'),
  ).toBe('# Snapshot\n');
});
it('rejects forged types, path payloads, overlimits, invalid signatures, and artifact tampering', () => {
  const f = fixture();
  expect(() =>
    f.store.request({ ...f.upload, dataBase64: undefined, path: '/etc/passwd' }),
  ).toThrow('upload');
  expect(() => f.store.request({ ...f.upload, mimeType: 'image/svg+xml' })).toThrow('media type');
  expect(() => f.store.request({ ...f.upload, mimeType: 'image/png' })).toThrow('signature');
  expect(() =>
    f.store.request({
      ...f.upload,
      dataBase64: Buffer.alloc(INTENT_ARTIFACT_LIMITS.bytes + 1).toString('base64'),
    }),
  ).toThrow('oversized');
  expect(() => f.store.request({ ...f.upload, dataBase64: '!!!!' })).toThrow('upload');
  expect(() =>
    f.store.request({ ...f.upload, dataBase64: Buffer.from([0xff, 0xfe]).toString('base64') }),
  ).toThrow('UTF-8');
  const saved = artifact(f.store.request(f.upload));
  const filename = path.join(f.root, 'intent-artifacts', saved.contentId + '.bin');
  fs.writeFileSync(filename, 'x'.repeat(saved.bytes));
  expect(() =>
    f.store.request({ action: 'readArtifact', id: f.workspace.id, artifactId: saved.id }),
  ).toThrow('integrity');
  fs.rmSync(filename);
  fs.symlinkSync('/etc/passwd', filename);
  expect(() =>
    f.store.request({ action: 'readArtifact', id: f.workspace.id, artifactId: saved.id }),
  ).toThrow();
});
it('refuses linked storage directories, cross-workspace links and stale-revision annotations', () => {
  const f = fixture();
  const other = fixture();
  fs.symlinkSync(other.root, path.join(f.root, 'intent-artifacts'));
  expect(() => f.store.request(f.upload)).toThrow();
  expect(fs.readdirSync(other.root).filter((name) => name.endsWith('.bin'))).toEqual([]);
  fs.unlinkSync(path.join(f.root, 'intent-artifacts'));
  const saved = artifact(f.store.request(f.upload));
  expect(() => other.store.request({ ...other.upload, versionOf: saved.id })).toThrow(
    'does not belong',
  );
  const note = {
    action: 'annotateArtifact',
    id: f.workspace.id,
    expectedRevision: 1,
    annotationId: 'a',
    artifactId: saved.id,
    artifactSha256: saved.sha256,
    text: 'Comment',
  };
  expect(() => f.store.request({ ...note, artifactSha256: 'wrong' })).toThrow('version changed');
  expect(() => f.store.request({ ...note, point: { x: 0.5, y: 0.5 } })).toThrow('screenshot');
  f.workspaces.request({
    action: 'update',
    id: f.workspace.id,
    expectedRevision: 1,
    fields: { ...f.workspace, outcome: 'Revised export outcome' },
    reason: 'New intent',
  });
  expect(() => f.store.request(note)).toThrow('changed elsewhere');
  expect(() => f.store.request({ ...note, expectedRevision: 2 })).toThrow(
    'current intent revision',
  );
});
it('pins normalized screenshot annotations and ordered demonstration steps to exact hashes', () => {
  const f = fixture();
  const saved = artifact(f.store.request({ ...f.upload, mimeType: 'image/png', dataBase64: png }));
  const note = {
    action: 'annotateArtifact',
    id: f.workspace.id,
    expectedRevision: 1,
    annotationId: 'a',
    artifactId: saved.id,
    artifactSha256: saved.sha256,
    text: 'Button',
    point: { x: 0.25, y: 0.75 },
  };
  expect(() => f.store.request({ ...note, point: { x: Infinity, y: 0 } })).toThrow('coordinates');
  expect(() => f.store.request({ ...note, point: { x: -0.1, y: 0 } })).toThrow('coordinates');
  expect(f.store.request(note)).toMatchObject({
    annotation: { point: note.point, artifactSha256: saved.sha256 },
  });
  const second = artifact(
    f.store.request({
      ...f.upload,
      artifactId: 'screenshot-two',
      mimeType: 'image/png',
      dataBase64: png,
    }),
  );
  const demo = {
    action: 'createDemonstration',
    id: f.workspace.id,
    expectedRevision: 1,
    demonstrationId: 'demo',
    title: 'Export flow',
    steps: [
      { artifactId: second.id, caption: 'Start here' },
      { artifactId: saved.id, caption: 'See output' },
    ],
  };
  const result = f.store.request(demo);
  expect(result).toMatchObject({
    demonstration: {
      steps: [
        { artifactId: second.id, sha256: second.sha256, caption: 'Start here' },
        { artifactId: saved.id, sha256: saved.sha256, caption: 'See output' },
      ],
    },
  });
  expect(f.store.request(demo)).toEqual(result);
  expect(() => f.store.request({ ...demo, steps: [...demo.steps].reverse() })).toThrow(
    'different content',
  );
});
it('records bounded alternative hypotheses and explicit reasoned selections without changing workspace state', () => {
  const f = fixture();
  const saved = artifact(f.store.request(f.upload));
  const group = {
    action: 'createAlternativeGroup',
    id: f.workspace.id,
    expectedRevision: 1,
    groupId: 'group',
    title: 'CSV strategy',
    purpose: 'Choose output approach',
    budgetMinutes: 30,
    alternatives: [
      {
        id: 'a',
        title: 'Stream',
        hypothesis: 'Lower memory',
        artifactIds: [saved.id],
        executionIds: [],
      },
      {
        id: 'b',
        title: 'Batch',
        hypothesis: 'Simpler validation',
        artifactIds: [],
        executionIds: [],
      },
    ],
  };
  f.store.request(group);
  expect(() =>
    f.store.request({
      ...group,
      groupId: 'oversized',
      alternatives: Array.from({ length: 7 }, (_, i) => ({
        ...group.alternatives[0],
        id: String(i),
      })),
    }),
  ).toThrow('two and six');
  const choice = {
    action: 'selectAlternative',
    id: f.workspace.id,
    expectedRevision: 1,
    selectionId: 'choose-a',
    groupId: 'group',
    alternativeId: 'a',
    reason: 'Memory is the primary constraint',
  };
  expect(() => f.store.request({ ...choice, reason: '' })).toThrow('reason');
  const first = f.store.request(choice);
  expect(f.store.request(choice)).toEqual(first);
  const another = open(f.filename).store;
  expect(() => another.request({ ...choice, selectionId: 'choose-b', alternativeId: 'b' })).toThrow(
    'changed elsewhere',
  );
  another.request({
    ...choice,
    selectionId: 'choose-b',
    alternativeId: 'b',
    expectedSelectionId: 'choose-a',
    reason: 'Revised tradeoff after review',
  });
  expect(f.store.request({ action: 'artifacts', id: f.workspace.id })).toMatchObject({
    selections: [
      expect.objectContaining({ alternativeId: 'b', previousSelectionId: 'choose-a' }),
      expect.objectContaining({ alternativeId: 'a' }),
    ],
  });
  expect(f.workspaces.request({ action: 'list' })).toMatchObject({
    workspaces: [expect.objectContaining({ revision: 1, status: 'active' })],
  });
  f.workspaces.request({
    action: 'update',
    id: f.workspace.id,
    expectedRevision: 1,
    fields: { ...f.workspace, outcome: 'Revised export outcome' },
    reason: 'New revision',
  });
  expect(() =>
    f.store.request({
      ...choice,
      selectionId: 'stale',
      expectedRevision: 2,
      expectedSelectionId: 'choose-b',
    }),
  ).toThrow('earlier intent');
});
it('stores URL references without fetching and cleans bytes when metadata commit fails', () => {
  const f = fixture();
  const request = {
    action: 'addArtifact',
    id: f.workspace.id,
    expectedRevision: 1,
    artifactId: 'url',
    title: 'Live preview',
    url: 'https://example.com/preview',
  };
  expect(() => f.store.request({ ...request, url: 'javascript:alert(1)' })).toThrow('HTTP');
  expect(() => f.store.request({ ...request, url: 'https://user:password@example.com' })).toThrow(
    'credentials',
  );
  expect(f.store.request(request)).toMatchObject({
    artifact: { kind: 'url', bytes: 0, url: request.url },
  });
  expect(fs.readdirSync(f.root)).not.toContain('intent-artifacts');
  f.db.exec(
    "CREATE TRIGGER reject_artifact BEFORE INSERT ON intent_artifacts BEGIN SELECT RAISE(ABORT, 'disk failed'); END",
  );
  expect(() => f.store.request(f.upload)).toThrow('disk failed');
  expect(fs.readdirSync(path.join(f.root, 'intent-artifacts'))).toEqual([]);
});
