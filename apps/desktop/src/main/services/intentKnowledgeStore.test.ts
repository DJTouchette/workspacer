import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { DatabaseSync } from 'node:sqlite';
import { afterEach, expect, it, vi } from 'vitest';
vi.mock('./configService', () => ({ getConfigDir: () => '/unused-knowledge-tests' }));
import { IntentWorkspaceStore } from './intentWorkspaceStore';
import type { IntentKnowledgeCapture, IntentKnowledgePromotion } from '../shared/intentKnowledge';

const opened: DatabaseSync[] = [];
afterEach(() => {
  vi.restoreAllMocks();
  for (const db of opened.splice(0)) if (db.isOpen) db.close();
});
function fixture() {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'intent-knowledge-'));
  const root = path.join(directory, 'repo');
  const context = path.join(root, '.rivet/context/modules');
  fs.mkdirSync(context, { recursive: true });
  fs.writeFileSync(path.join(context, 'sample.md'), '# Shared convention\n\nOriginal knowledge.\n');
  const db = new DatabaseSync(path.join(directory, 'work.sqlite'));
  opened.push(db);
  const store = new IntentWorkspaceStore(db);
  const created = store.request({
    action: 'create',
    projectRoot: root,
    fields: {
      title: 'Feature',
      outcome: 'Outcome',
      constraints: '',
      successCriteria: 'Works',
      sourceUrl: '',
      status: 'active',
    },
  });
  if (created.action !== 'create') throw new Error('Expected workspace');
  const id = created.workspace.id;
  const list = () => {
    const result = store.request({ action: 'knowledge', id });
    if (result.action !== 'knowledge') throw new Error('Expected knowledge');
    return result;
  };
  const capture = () => {
    const document = list().documents[0];
    const result = store.request({
      action: 'captureKnowledge',
      id,
      captureId: 'capture-one',
      expectedRevision: 1,
      path: document.path,
      expectedSha256: document.sha256,
    });
    if (result.action !== 'captureKnowledge') throw new Error('Expected capture');
    return result.capture;
  };
  const finding = () =>
    store.request({
      action: 'recordFinding',
      id,
      findingId: 'finding-one',
      expectedRevision: 1,
      title: 'Reuse the permission helper',
      observation: 'The shared helper handles tenant access.',
      captureIds: [],
    });
  const prepare = (kind: 'learning' | 'context' = 'learning') => {
    finding();
    const result = store.request({
      action: 'prepareKnowledgePromotion',
      id,
      proposalId: 'proposal-one',
      findingId: 'finding-one',
      expectedRevision: 1,
      kind,
      path: '.rivet/context/modules/sample.md',
    });
    if (result.action !== 'prepareKnowledgePromotion') throw new Error('Expected proposal');
    return result.proposal;
  };
  return { directory, root, context, db, store, id, list, capture, finding, prepare };
}
it('captures immutable document provenance, detects drift, and survives restart', () => {
  const f = fixture(),
    capture = f.capture();
  fs.writeFileSync(path.join(f.context, 'sample.md'), '# Changed\nNew content');
  const listed = f.list();
  expect(listed.captures[0]).toEqual(capture);
  expect(listed.documents[0].sha256).not.toBe(capture.sha256);
  expect(() =>
    f.store.request({
      action: 'captureKnowledge',
      id: f.id,
      captureId: 'another',
      expectedRevision: 1,
      path: capture.path,
      expectedSha256: capture.sha256,
    }),
  ).toThrow('changed');
  f.db.close();
  const db = new DatabaseSync(path.join(f.directory, 'work.sqlite'));
  opened.push(db);
  expect(new IntentWorkspaceStore(db).request({ action: 'knowledge', id: f.id })).toMatchObject({
    captures: [capture],
  });
});
it('requires saved revision and workspace-scoped capture/finding identities', () => {
  const f = fixture();
  expect(() =>
    f.store.request({
      action: 'recordFinding',
      id: f.id,
      findingId: 'bad',
      title: 'Finding',
      observation: 'Observation',
      captureIds: [],
    }),
  ).toThrow('saved intent revision');
  expect(() =>
    f.store.request({
      action: 'recordFinding',
      id: f.id,
      findingId: 'bad',
      expectedRevision: 2,
      title: 'Finding',
      observation: 'Observation',
      captureIds: [],
    }),
  ).toThrow('Intent changed');
  expect(() =>
    f.store.request({
      action: 'recordFinding',
      id: f.id,
      findingId: 'bad',
      expectedRevision: 1,
      title: 'Finding',
      observation: 'Observation',
      captureIds: ['foreign'],
    }),
  ).toThrow('this workspace');
  expect(f.list().findings).toEqual([]);
});
it('prepares without writing and publishes only exact reviewed content; refuses source drift', () => {
  const f = fixture(),
    proposal = f.prepare();
  const target = path.join(f.root, proposal.path);
  expect(fs.existsSync(target)).toBe(false);
  expect(
    f.store.request({ action: 'publishKnowledgePromotion', id: f.id, proposalId: proposal.id }),
  ).toMatchObject({ proposal: { status: 'written' } });
  expect(fs.readFileSync(target, 'utf8')).toBe(proposal.content);
  f.store.request({ action: 'publishKnowledgePromotion', id: f.id, proposalId: proposal.id });
  expect(fs.readFileSync(target, 'utf8')).toBe(proposal.content);
  const other = fixture();
  const append = other.prepare('context');
  fs.appendFileSync(path.join(other.root, append.path), '\nConcurrent editor change');
  expect(() =>
    other.store.request({
      action: 'publishKnowledgePromotion',
      id: other.id,
      proposalId: append.id,
    }),
  ).toThrow('document changed');
  expect(fs.readFileSync(path.join(other.root, append.path), 'utf8')).not.toContain(
    'tenant access',
  );
});
it('keeps receipt-write failure unknown and reconciles by host file hash without replay', () => {
  const f = fixture(),
    proposal = f.prepare();
  f.db.exec(
    "CREATE TRIGGER fail_knowledge_receipt BEFORE UPDATE ON intent_knowledge WHEN json_extract(new.snapshot,'$.status')='written' BEGIN SELECT RAISE(ABORT,'disk full'); END",
  );
  expect(() =>
    f.store.request({ action: 'publishKnowledgePromotion', id: f.id, proposalId: proposal.id }),
  ).toThrow('outcome needs inspection');
  expect(f.list().proposals[0].status).toBe('unknown');
  f.db.exec('DROP TRIGGER fail_knowledge_receipt');
  expect(
    f.store.request({ action: 'reconcileKnowledgePromotion', id: f.id, proposalId: proposal.id }),
  ).toMatchObject({ proposal: { status: 'written' } });
  expect(fs.readFileSync(path.join(f.root, proposal.path), 'utf8')).toBe(proposal.content);
});
it('never writes when a durable claim fails', () => {
  const f = fixture(),
    proposal = f.prepare();
  f.db.exec(
    "CREATE TRIGGER fail_knowledge_claim BEFORE UPDATE ON intent_knowledge BEGIN SELECT RAISE(ABORT,'claim disk full'); END",
  );
  expect(() =>
    f.store.request({ action: 'publishKnowledgePromotion', id: f.id, proposalId: proposal.id }),
  ).toThrow('claim disk full');
  expect(fs.existsSync(path.join(f.root, proposal.path))).toBe(false);
});
it('rejects static symlinks and parent swaps cannot expose outside knowledge bytes', () => {
  const f = fixture();
  const document = f.list().documents[0];
  const outside = path.join(f.directory, 'outside');
  fs.mkdirSync(path.join(outside, 'context/modules'), { recursive: true });
  fs.writeFileSync(path.join(outside, 'context/modules/sample.md'), 'OUTSIDE SECRET');
  const open = fs.openSync.bind(fs);
  let swapped = false;
  vi.spyOn(fs, 'openSync').mockImplementation(((file, flags, mode) => {
    if (!swapped && String(file).endsWith('/sample.md')) {
      swapped = true;
      fs.renameSync(path.join(f.root, '.rivet'), path.join(f.root, '.rivet-original'));
      fs.symlinkSync(outside, path.join(f.root, '.rivet'), 'dir');
    }
    return open(file, flags, mode);
  }) as typeof fs.openSync);
  const result = f.store.request({
    action: 'captureKnowledge',
    id: f.id,
    captureId: 'pinned',
    expectedRevision: 1,
    path: document.path,
    expectedSha256: document.sha256,
  });
  expect(result).toMatchObject({
    capture: { content: expect.stringContaining('Original knowledge') },
  });
  expect(() => f.list()).toThrow('ordinary project');
});
it('parent swaps cannot redirect learning-file writes outside the project', () => {
  const f = fixture(),
    proposal = f.prepare();
  fs.mkdirSync(path.join(f.root, '.rivet/learnings'));
  const outside = path.join(f.directory, 'outside-learning');
  fs.mkdirSync(outside);
  const open = fs.openSync.bind(fs);
  let swapped = false;
  vi.spyOn(fs, 'openSync').mockImplementation(((file, flags, mode) => {
    if (
      !swapped &&
      String(file).endsWith(path.basename(proposal.path)) &&
      typeof flags === 'number' &&
      flags & fs.constants.O_WRONLY
    ) {
      swapped = true;
      fs.renameSync(
        path.join(f.root, '.rivet/learnings'),
        path.join(f.root, '.rivet/learnings-original'),
      );
      fs.symlinkSync(outside, path.join(f.root, '.rivet/learnings'), 'dir');
    }
    return open(file, flags, mode);
  }) as typeof fs.openSync);
  expect(() =>
    f.store.request({ action: 'publishKnowledgePromotion', id: f.id, proposalId: proposal.id }),
  ).toThrow('needs inspection');
  expect(fs.readdirSync(outside)).toEqual([]);
  expect(f.list().proposals[0].status).toBe('unknown');
});
it('checks the pinned target bytes before replacing context, even if its pathname is swapped', () => {
  const f = fixture(),
    proposal = f.prepare('context');
  const moved = path.join(f.root, '.rivet/context/modules-original');
  const baseline = fs.readFileSync(path.join(f.context, 'sample.md'), 'utf8');
  const open = fs.openSync.bind(fs);
  let swapped = false;
  vi.spyOn(fs, 'openSync').mockImplementation(((file, flags, mode) => {
    if (
      !swapped &&
      String(file).endsWith('.tmp') &&
      typeof flags === 'number' &&
      flags & fs.constants.O_WRONLY
    ) {
      swapped = true;
      fs.renameSync(f.context, moved);
      fs.mkdirSync(f.context);
      fs.writeFileSync(path.join(f.context, 'sample.md'), baseline);
      fs.writeFileSync(path.join(moved, 'sample.md'), 'Concurrent edit in pinned directory');
    }
    return open(file, flags, mode);
  }) as typeof fs.openSync);
  expect(() =>
    f.store.request({ action: 'publishKnowledgePromotion', id: f.id, proposalId: proposal.id }),
  ).toThrow('changed during promotion');
  expect(fs.readFileSync(path.join(moved, 'sample.md'), 'utf8')).toBe(
    'Concurrent edit in pinned directory',
  );
  expect(fs.readFileSync(path.join(f.context, 'sample.md'), 'utf8')).toBe(baseline);
});

it('refuses hardlinked knowledge documents instead of exposing bytes linked from outside the project', () => {
  const f = fixture();
  const outside = path.join(f.directory, 'outside-secret.md');
  fs.writeFileSync(outside, 'OUTSIDE SECRET');
  fs.linkSync(outside, path.join(f.context, 'hardlink.md'));
  expect(() => f.list()).toThrow();
});

it('keeps knowledge capture filesystem reads outside the SQLite write lock', () => {
  const f = fixture();
  const document = f.list().documents[0];
  const other = new DatabaseSync(path.join(f.directory, 'work.sqlite'));
  opened.push(other);
  other.exec('PRAGMA busy_timeout=0');
  const read = fs.readSync.bind(fs);
  let checked = false;
  vi.spyOn(fs, 'readSync').mockImplementation(((...args: unknown[]) => {
    if (!checked) {
      checked = true;
      other.exec('BEGIN IMMEDIATE');
      other.exec('ROLLBACK');
    }
    return (read as (...values: unknown[]) => number)(...args);
  }) as typeof fs.readSync);
  expect(
    f.store.request({
      action: 'captureKnowledge',
      id: f.id,
      captureId: 'without-write-lock',
      expectedRevision: 1,
      path: document.path,
      expectedSha256: document.sha256,
    }),
  ).toMatchObject({ action: 'captureKnowledge' });
  expect(checked).toBe(true);
});
