import { DatabaseSync } from 'node:sqlite';
import fs from 'node:fs';
import {
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  writeFileSync,
  symlinkSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { afterEach, expect, it, vi } from 'vitest';
vi.mock('./configService', () => ({ getConfigDir: () => '/unused-intent-evidence-tests' }));
import { IntentWorkspaceStore } from './intentWorkspaceStore';
import { IntentEvidenceStore, INTENT_EVIDENCE_SCHEMA } from './intentEvidenceStore';
import type { IntentEvidenceResponse } from '../shared/intentEvidence';

const roots: string[] = [];
const opened: DatabaseSync[] = [];
afterEach(() => {
  vi.restoreAllMocks();
  for (const db of opened.splice(0)) if (db.isOpen) db.close();
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

it('leaves evidence file writes outside the SQLite lock and rejects a concurrently changed intent before committing', async () => {
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
      other.workspaceStore.request({
        action: 'update',
        id: f.workspace.id,
        expectedRevision: 1,
        fields: { ...f.workspace, title: 'Concurrent owner edit' },
        reason: 'While evidence bytes are written',
      });
    }
    return (write as (...values: unknown[]) => void)(file, ...args);
  }) as typeof fs.writeFileSync);
  await expect(f.store.capture(f.git, live)).rejects.toThrow('changed elsewhere');
  expect(changed).toBe(true);
  expect(f.store.request({ action: 'evidence', id: f.workspace.id })).toMatchObject({
    evidence: [],
  });
  expect(readdirSync(path.join(f.root, 'intent-evidence'))).toEqual([]);
});

it('allows owner metadata writes while an evidence artifact is being read', async () => {
  const f = fixture();
  await f.store.capture(f.git, live);
  const other = open(f.filename);
  other.db.exec('PRAGMA busy_timeout=0');
  const read = fs.readSync.bind(fs);
  let checked = false;
  vi.spyOn(fs, 'readSync').mockImplementation(((...args: unknown[]) => {
    if (!checked) {
      checked = true;
      other.db.exec('BEGIN IMMEDIATE');
      other.db.exec('ROLLBACK');
    }
    return (read as (...values: unknown[]) => number)(...args);
  }) as typeof fs.readSync);
  expect(
    f.store.request({ action: 'readEvidence', id: f.workspace.id, evidenceId: f.git.evidenceId }),
  ).toMatchObject({ artifact: captured.artifact });
  expect(checked).toBe(true);
});
function open(filename: string) {
  const db = new DatabaseSync(filename);
  opened.push(db);
  const workspaceStore = new IntentWorkspaceStore(db);
  db.exec(INTENT_EVIDENCE_SCHEMA);
  return { db, workspaceStore };
}
const captured = {
  cwd: '/host/actual',
  repositoryRoot: '/host/actual',
  headCommit: 'a'.repeat(40),
  capturedAt: '2026-09-13T00:00:00.000Z',
  scope: 'tracked-working-tree-against-head' as const,
  changedFiles: ['answer.ts'],
  omissions: [],
  artifact: '+actual host bytes\n',
};
function fixture() {
  const root = mkdtempSync(path.join(tmpdir(), 'intent-evidence-'));
  roots.push(root);
  const filename = path.join(root, 'intent.sqlite');
  const { db, workspaceStore } = open(filename);
  const result = workspaceStore.request({
    action: 'create',
    projectRoot: '/renderer/project',
    fields: {
      title: 'Export',
      outcome: 'Produce CSV',
      constraints: '',
      successCriteria: 'Valid CSV\n\nWorks offline',
      sourceUrl: '',
      status: 'review',
    },
  });
  if (result.action !== 'create') throw new Error('Expected workspace');
  const workspace = result.workspace;
  const linked = workspaceStore.request({
    action: 'attachSession',
    id: workspace.id,
    expectedRevision: 1,
    session: {
      sessionId: 'local',
      hub: '',
      cwd: '/renderer/forged',
      provider: 'codex',
      label: 'Worker',
    },
  });
  if (linked.action !== 'attachSession') throw new Error('Expected execution');
  const capture = vi.fn(async () => captured);
  const store = new IntentEvidenceStore(db, { capture });
  const manual = {
    action: 'addEvidence',
    id: workspace.id,
    expectedRevision: 1,
    evidenceId: 'e-one',
    criterionId: 'r1:c1',
    note: 'I opened the CSV and checked its fields.',
    reference: '',
    assessment: 'user-verified',
  };
  const git = {
    action: 'captureEvidence',
    id: workspace.id,
    expectedRevision: 1,
    evidenceId: 'git-one',
    criterionId: 'r1:c1',
    executionId: linked.execution.id,
  };
  const review = {
    action: 'recordReview',
    id: workspace.id,
    expectedRevision: 1,
    reviewId: 'review-one',
    reason: 'Both behaviors were checked.',
    decision: 'accept',
    evidenceIds: ['e-one', 'e-two'],
  };
  return { root, filename, db, workspaceStore, workspace, store, capture, manual, git, review };
}
const evidence = (response: IntentEvidenceResponse) => {
  if (!('evidence' in response) || Array.isArray(response.evidence))
    throw new Error('Expected evidence');
  return response.evidence;
};
const live = [{ sessionId: 'local', cwd: '/host/actual' }];

it('keeps immutable revision-scoped criteria, provenance, and review history across reopen', () => {
  const f = fixture();
  const a = evidence(
    f.store.request({ ...f.manual, criterion: { text: 'forged' }, author: 'agent', git: captured }),
  );
  expect(a).toMatchObject({
    criterion: { id: 'r1:c1', text: 'Valid CSV', intentRevision: 1 },
    author: 'user',
    kind: 'manual',
    assessment: 'user-verified',
  });
  expect(a.git).toBeUndefined();
  expect(evidence(f.store.request(f.manual))).toEqual(a);
  expect(() => f.store.request({ ...f.manual, note: 'Changed' })).toThrow('different content');
  f.store.request({ ...f.manual, evidenceId: 'e-two', criterionId: 'r1:c2' });
  const review = f.store.request(f.review);
  expect(f.store.request(f.review)).toEqual(review);
  f.workspaceStore.request({
    action: 'update',
    id: f.workspace.id,
    expectedRevision: 1,
    fields: { ...f.workspace, successCriteria: 'Different requirement' },
    reason: 'Changed target',
  });
  expect(() => f.store.request({ ...f.manual, evidenceId: 'stale' })).toThrow('changed elsewhere');
  expect(() => f.store.request({ ...f.review, reviewId: 'stale' })).toThrow('changed elsewhere');
  f.db.close();
  const reopened = new IntentEvidenceStore(open(f.filename).db);
  expect(reopened.request({ action: 'evidence', id: f.workspace.id })).toMatchObject({
    criteria: [{ id: 'r2:c1', text: 'Different requirement' }],
    evidence: [expect.objectContaining({ id: 'e-two', intentRevision: 1 }), a],
    reviews: [expect.objectContaining({ evidenceIds: ['e-one', 'e-two'], intentRevision: 1 })],
  });
});

it('requires explicit current-criterion verification for acceptance and retains change requests', () => {
  const f = fixture();
  f.store.request({ ...f.manual, assessment: 'reported' });
  expect(() => f.store.request({ ...f.review, evidenceIds: ['e-one'] })).toThrow('every criterion');
  f.store.request({ ...f.manual, evidenceId: 'verified', linkedEvidenceId: 'e-one' });
  f.store.request({
    ...f.manual,
    evidenceId: 'e-two',
    criterionId: 'r1:c2',
    assessment: 'unresolved',
  });
  expect(() => f.store.request({ ...f.review, evidenceIds: ['verified', 'e-two'] })).toThrow(
    'every criterion',
  );
  const review = f.store.request({ ...f.review, decision: 'changes-requested' });
  expect(review).toMatchObject({
    review: {
      decision: 'changes-requested',
      reason: f.review.reason,
      evidenceIds: ['e-one', 'e-two'],
    },
  });
  expect(f.workspaceStore.request({ action: 'list' })).toMatchObject({
    workspaces: [expect.objectContaining({ status: 'review', revision: 1 })],
  });
});

it('refuses cross-workspace, cross-criterion and stale-revision references and malformed inputs', () => {
  const f = fixture();
  const other = fixture();
  f.store.request(f.manual);
  expect(() =>
    f.store.request({
      ...f.manual,
      evidenceId: 'wrong-criterion',
      criterionId: 'r1:c2',
      linkedEvidenceId: 'e-one',
    }),
  ).toThrow('same criterion');
  expect(() => other.store.request({ ...other.manual, linkedEvidenceId: 'e-one' })).toThrow(
    'does not belong',
  );
  expect(() =>
    f.store.request({ ...f.manual, evidenceId: 'bad-url', reference: 'file:///etc/passwd' }),
  ).toThrow('HTTP or HTTPS');
  expect(() =>
    f.store.request({
      ...f.manual,
      evidenceId: 'bad-url',
      reference: 'https://user:secret@example.com',
    }),
  ).toThrow('without credentials');
  expect(() =>
    f.store.request({ ...f.manual, evidenceId: 'bad-criterion', criterionId: 'r99:c1' }),
  ).toThrow('saved intent');
  expect(() =>
    f.store.request({ ...f.manual, evidenceId: 'bad-state', assessment: 'host-verified' }),
  ).toThrow('assessment');
  expect(() => f.store.request({ ...f.review, reason: '' })).toThrow('reason');
  f.workspaceStore.request({
    action: 'update',
    id: f.workspace.id,
    expectedRevision: 1,
    fields: { ...f.workspace, outcome: 'Revised export outcome' },
    reason: 'Revision',
  });
  expect(() =>
    f.store.request({ ...f.review, expectedRevision: 2, evidenceIds: ['e-one'] }),
  ).toThrow('current intent');
});

it('uses only host-owned local cwd, stores immutable artifact bytes separately, and verifies integrity', async () => {
  const f = fixture();
  const record = evidence(
    await f.store.capture(
      {
        ...f.git,
        cwd: '/renderer/attack',
        git: { artifact: 'forged' },
        assessment: 'user-verified',
      },
      live,
    ),
  );
  expect(f.capture).toHaveBeenCalledExactlyOnceWith('/host/actual');
  expect(record).toMatchObject({
    kind: 'git',
    assessment: 'reported',
    git: { cwd: '/host/actual', headCommit: captured.headCommit },
  });
  expect(
    String(f.db.prepare('SELECT snapshot FROM intent_evidence').get()?.snapshot),
  ).not.toContain(captured.artifact);
  const filename = path.join(f.root, 'intent-evidence', record.git!.artifactId + '.diff');
  expect(readFileSync(filename, 'utf8')).toBe(captured.artifact);
  f.db.close();
  const reopened = new IntentEvidenceStore(open(f.filename).db);
  expect(
    reopened.request({ action: 'readEvidence', id: f.workspace.id, evidenceId: record.id }),
  ).toMatchObject({ artifact: captured.artifact });
  expect(await reopened.capture(f.git, [])).toMatchObject({ evidence: record });
  writeFileSync(filename, 'x'.repeat(Buffer.byteLength(captured.artifact)));
  expect(() =>
    reopened.request({ action: 'readEvidence', id: f.workspace.id, evidenceId: record.id }),
  ).toThrow('integrity');
});

it('refuses missing, wrong-hub and peer executions without consulting renderer paths', async () => {
  const f = fixture();
  await expect(f.store.capture(f.git, [])).rejects.toThrow('not currently available');
  await expect(
    f.store.capture(f.git, [{ sessionId: 'local', hub: 'peer', cwd: '/bad' }]),
  ).rejects.toThrow('not currently available');
  const attached = f.workspaceStore.request({
    action: 'attachSession',
    id: f.workspace.id,
    expectedRevision: 1,
    session: { sessionId: 'local', hub: 'peer', cwd: '/bad', provider: 'codex', label: 'Peer' },
  });
  if (attached.action !== 'attachSession') throw new Error('Expected execution');
  await expect(
    f.store.capture({ ...f.git, executionId: attached.execution.id }, live),
  ).rejects.toThrow('linked local execution');
  expect(f.capture).not.toHaveBeenCalled();
});

it('rejects an intent revision changed during capture without leaving an artifact or evidence row', async () => {
  const f = fixture();
  f.capture.mockImplementationOnce(async () => {
    f.workspaceStore.request({
      action: 'update',
      id: f.workspace.id,
      expectedRevision: 1,
      fields: { ...f.workspace, outcome: 'Revised export outcome' },
      reason: 'During capture',
    });
    return captured;
  });
  await expect(f.store.capture(f.git, live)).rejects.toThrow('changed elsewhere');
  expect(f.store.request({ action: 'evidence', id: f.workspace.id })).toMatchObject({
    evidence: [],
  });
  expect(readdirSync(f.root)).not.toContain('intent-evidence');
});

it('rolls back artifact bytes when durable insert fails, and concurrent IDs retain one snapshot', async () => {
  const f = fixture();
  f.db.exec(
    "CREATE TRIGGER fail_evidence BEFORE INSERT ON intent_evidence BEGIN SELECT RAISE(ABORT, 'disk failed'); END",
  );
  await expect(f.store.capture(f.git, live)).rejects.toThrow('disk failed');
  expect(readdirSync(path.join(f.root, 'intent-evidence'))).toEqual([]);
  f.db.exec('DROP TRIGGER fail_evidence');
  const [first, second] = await Promise.all([
    f.store.capture(f.git, live),
    f.store.capture(f.git, live),
  ]);
  expect(first).toEqual(second);
  expect(readdirSync(path.join(f.root, 'intent-evidence'))).toHaveLength(1);
});

it('refuses a redirected evidence directory before writing any host-captured bytes', async () => {
  const f = fixture();
  symlinkSync(f.root, path.join(f.root, 'intent-evidence'));
  await expect(f.store.capture(f.git, live)).rejects.toThrow();
  expect(readdirSync(f.root).filter((name) => name.endsWith('.diff'))).toEqual([]);
  expect(f.store.request({ action: 'evidence', id: f.workspace.id })).toMatchObject({
    evidence: [],
  });
});
