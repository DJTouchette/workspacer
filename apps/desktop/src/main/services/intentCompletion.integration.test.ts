import { DatabaseSync } from 'node:sqlite';
import fs from 'node:fs';
import path from 'node:path';
import { tmpdir } from 'node:os';
import { afterEach, expect, it, vi } from 'vitest';
vi.mock('./configService', () => ({ getConfigDir: () => '/unused-intent-completion-tests' }));
import { IntentWorkspaceStore } from './intentWorkspaceStore';
import { INTENT_EVIDENCE_SCHEMA } from './intentEvidenceStore';
import { INTENT_CONTROL_SCHEMA } from './intentControlStore';
import { IntentSourceStore } from './intentSourceStore';
import { sourceSnapshot } from './intentSourceAdapters';
import { intentCriteria } from '../shared/intentEvidence';
import type { IntentWorkspace } from '../shared/intentWorkspace';

const opened: DatabaseSync[] = [];
afterEach(() => {
  for (const db of opened.splice(0)) if (db.isOpen) db.close();
  vi.unstubAllGlobals();
  vi.unstubAllEnvs();
});
function legacy() {
  const directory = fs.mkdtempSync(path.join(tmpdir(), 'intent-completion-'));
  const projectRoot = path.join(directory, 'project');
  fs.mkdirSync(projectRoot);
  const file = path.join(directory, 'intent-workspaces.sqlite');
  const db = new DatabaseSync(file);
  opened.push(db);
  // Exact durable v4 table contract: v5 identity/source/knowledge/artifact tables
  // intentionally do not exist until the production constructor migrates them.
  db.exec(
    `
    PRAGMA foreign_keys=ON;
    CREATE TABLE intent_workspaces(id TEXT PRIMARY KEY,project_root TEXT NOT NULL,revision INTEGER NOT NULL,updated_at TEXT NOT NULL,snapshot TEXT NOT NULL);
    CREATE TABLE intent_revisions(workspace_id TEXT NOT NULL REFERENCES intent_workspaces(id),revision INTEGER NOT NULL,at TEXT NOT NULL,reason TEXT NOT NULL,snapshot TEXT NOT NULL,PRIMARY KEY(workspace_id,revision));
    CREATE TABLE intent_executions(id TEXT PRIMARY KEY,workspace_id TEXT NOT NULL,intent_revision INTEGER NOT NULL,snapshot TEXT NOT NULL,FOREIGN KEY(workspace_id,intent_revision) REFERENCES intent_revisions(workspace_id,revision));
    CREATE TABLE intent_work_links(id TEXT PRIMARY KEY,workspace_id TEXT NOT NULL REFERENCES intent_workspaces(id),kind TEXT NOT NULL,target TEXT NOT NULL,created_at TEXT NOT NULL,UNIQUE(workspace_id,kind,target));
    CREATE TABLE intent_directions(id TEXT PRIMARY KEY,workspace_id TEXT NOT NULL,execution_id TEXT NOT NULL REFERENCES intent_executions(id),intent_revision INTEGER NOT NULL,snapshot TEXT NOT NULL,FOREIGN KEY(workspace_id,intent_revision) REFERENCES intent_revisions(workspace_id,revision));
  ` +
      INTENT_EVIDENCE_SCHEMA +
      INTENT_CONTROL_SCHEMA +
      'PRAGMA user_version=4;',
  );
  const workspace: IntentWorkspace = {
    id: 'w',
    projectRoot,
    revision: 2,
    title: 'Export',
    outcome: 'CSV output',
    constraints: 'Keep permissions',
    successCriteria: 'CSV opens',
    sourceUrl: 'https://example.test/legacy-ticket',
    status: 'active',
    createdAt: '2026-09-12',
    updatedAt: '2026-09-13',
  };
  const current = JSON.stringify(workspace);
  const first = JSON.stringify({ ...workspace, revision: 1, title: 'Initial export' });
  db.prepare('INSERT INTO intent_workspaces VALUES(?,?,?,?,?)').run(
    'w',
    projectRoot,
    2,
    workspace.updatedAt,
    current,
  );
  db.prepare('INSERT INTO intent_revisions VALUES(?,?,?,?,?)').run(
    'w',
    1,
    '2026-09-12',
    'Initial',
    first,
  );
  db.prepare('INSERT INTO intent_revisions VALUES(?,?,?,?,?)').run(
    'w',
    2,
    '2026-09-13',
    'Refined',
    current,
  );
  const oldPacket = 'IMMUTABLE EARLIER REQUEST: ' + projectRoot;
  const execution = JSON.stringify({
    id: 'old-run',
    workspaceId: 'w',
    intentRevision: 1,
    kind: 'launch',
    state: 'launching',
    task: 'Export',
    contextPacket: oldPacket,
    session: null,
    lastObservation: null,
    createdAt: '2026-09-12',
    updatedAt: '2026-09-12',
  });
  db.prepare('INSERT INTO intent_executions VALUES(?,?,?,?)').run('old-run', 'w', 1, execution);
  return { db, file, directory, projectRoot, workspace, current, first, execution, oldPacket };
}
it('migrates real v4 rows, preserves cross-module provenance through root relocation and restart', async () => {
  const f = legacy();
  const store = new IntentWorkspaceStore(f.db);
  expect(f.db.prepare('PRAGMA user_version').get()?.user_version).toBe(5);
  expect(f.db.prepare('PRAGMA foreign_key_check').all()).toEqual([]);
  const latest = JSON.parse(
    String(f.db.prepare('SELECT snapshot FROM intent_workspaces WHERE id=?').get('w')!.snapshot),
  );
  expect(latest.projectId).toBeTruthy();
  expect(latest.repositoryId).toBeTruthy();
  expect(
    String(f.db.prepare('SELECT snapshot FROM intent_revisions WHERE revision=2').get()!.snapshot),
  ).toBe(f.current);
  expect(String(f.db.prepare('SELECT snapshot FROM intent_executions').get()!.snapshot)).toBe(
    f.execution,
  );

  const imported = await store.sources.request({
    action: 'addSource',
    id: 'w',
    sourceId: 'manual-source',
    expectedRevision: 2,
    connection: { provider: 'manual', url: 'https://example.test/requirement', credentialEnv: '' },
    title: 'Team requirement',
    content: 'CSV must preserve headings',
  });
  expect(imported.action).toBe('addSource');
  const contextDir = path.join(f.projectRoot, '.rivet', 'context', 'domains');
  fs.mkdirSync(contextDir, { recursive: true });
  fs.writeFileSync(
    path.join(contextDir, 'exports.md'),
    '# Export rules\nPreserve access checks.\n',
  );
  const catalog = store.knowledge.request({ action: 'knowledge', id: 'w' });
  if (catalog.action !== 'knowledge') throw new Error('Expected knowledge catalog');
  const capture = store.knowledge.request({
    action: 'captureKnowledge',
    id: 'w',
    expectedRevision: 2,
    captureId: 'knowledge-capture',
    path: catalog.documents[0].path,
    expectedSha256: catalog.documents[0].sha256,
  });
  expect(capture.action).toBe('captureKnowledge');
  const content = 'CSV sample: name,value\n';
  const artifact = store.artifacts.request({
    action: 'addArtifact',
    id: 'w',
    expectedRevision: 2,
    artifactId: 'sample',
    title: 'CSV sample',
    mimeType: 'text/plain',
    dataBase64: Buffer.from(content).toString('base64'),
    criterionId: intentCriteria(2, f.workspace.successCriteria)[0].id,
  });
  expect(artifact.action).toBe('addArtifact');
  expect(store.sources.contextPacket('w')).toContain('CSV must preserve headings');
  expect(store.knowledge.contextPacket('w')).toContain('Preserve access checks.');

  const movedRoot = path.join(f.directory, 'moved');
  fs.renameSync(f.projectRoot, movedRoot);
  const relocation = store.projects.request({
    action: 'relocateIntentRepository',
    projectId: latest.projectId,
    repositoryId: latest.repositoryId,
    expectedRevision: 1,
    root: movedRoot,
    reason: 'Project moved on disk',
  });
  if (relocation.action !== 'relocateIntentRepository') throw new Error('Expected root relocation');
  expect(relocation.workspaces[0]).toMatchObject({
    id: 'w',
    projectId: latest.projectId,
    repositoryId: latest.repositoryId,
    projectRoot: movedRoot,
    revision: 3,
  });
  expect(
    String(f.db.prepare('SELECT snapshot FROM intent_revisions WHERE revision=1').get()!.snapshot),
  ).toBe(f.first);
  expect(
    String(f.db.prepare('SELECT snapshot FROM intent_revisions WHERE revision=2').get()!.snapshot),
  ).toBe(f.current);
  expect(String(f.db.prepare('SELECT snapshot FROM intent_executions').get()!.snapshot)).toBe(
    f.execution,
  );

  f.db.close();
  const reopened = new DatabaseSync(f.file);
  opened.push(reopened);
  const restored = new IntentWorkspaceStore(reopened);
  expect(
    restored.artifacts.request({ action: 'readArtifact', id: 'w', artifactId: 'sample' }),
  ).toMatchObject({
    dataBase64: Buffer.from(content).toString('base64'),
    artifact: { intentRevision: 2 },
  });
  expect(await restored.sources.request({ action: 'sources', id: 'w' })).toMatchObject({
    sources: [{ accepted: { content: 'CSV must preserve headings' } }],
  });
  expect(restored.knowledge.request({ action: 'knowledge', id: 'w' })).toMatchObject({
    captures: [{ intentRevision: 2, content: '# Export rules\nPreserve access checks.\n' }],
  });
  expect(restored.projects.request({ action: 'projects' })).toMatchObject({
    projects: [
      { id: latest.projectId, repositories: [{ id: latest.repositoryId, root: movedRoot }] },
    ],
  });
  expect(reopened.prepare('PRAGMA foreign_key_check').all()).toEqual([]);
});

it('rolls back the entire v5 migration if identity backfill refuses corrupt legacy state', () => {
  const f = legacy();
  f.db
    .prepare('UPDATE intent_workspaces SET snapshot=? WHERE id=?')
    .run(JSON.stringify({ ...f.workspace, projectRoot: 'relative-root' }), 'w');
  expect(() => new IntentWorkspaceStore(f.db)).toThrow('absolute path');
  expect(f.db.prepare('PRAGMA user_version').get()?.user_version).toBe(4);
  expect(
    f.db
      .prepare(
        "SELECT name FROM sqlite_master WHERE name IN ('intent_projects','intent_sources','intent_artifacts','intent_knowledge')",
      )
      .all(),
  ).toEqual([]);
  expect(String(f.db.prepare('SELECT snapshot FROM intent_executions').get()!.snapshot)).toBe(
    f.execution,
  );
  f.db.prepare('UPDATE intent_workspaces SET snapshot=? WHERE id=?').run(f.current, 'w');
  new IntentWorkspaceStore(f.db);
  expect(f.db.prepare('PRAGMA user_version').get()?.user_version).toBe(5);
});

it('pins accepted source, knowledge, evidence and selections into future launch/direction/continue packets only', async () => {
  const f = legacy();
  const network = vi.fn(() => {
    throw new Error('Context compilation must never access the network');
  });
  vi.stubGlobal('fetch', network);
  vi.stubEnv('WORKSPACER_SOURCE_PRIVATE_TOKEN', 'PRIVATE_PACKET_CREDENTIAL_VALUE');
  const store = new IntentWorkspaceStore(f.db);
  const accepted = sourceSnapshot(
    'ticket-v1',
    'Team ticket',
    'ACCEPTED REQUIREMENT: preserve headings',
    { state: 'Team review' },
  );
  const candidate = sourceSnapshot(
    'ticket-v2',
    'Team ticket',
    'UNACCEPTED CANDIDATE: rename headings',
    { state: 'Updated' },
  );
  const providerRead = vi.fn().mockResolvedValue({ nativeId: 'TEAM-12', snapshot: accepted });
  const providerComment = vi.fn();
  const sources = new IntentSourceStore(f.db, { read: providerRead, comment: providerComment });
  await sources.request({
    action: 'addSource',
    id: 'w',
    sourceId: 'ticket',
    expectedRevision: 2,
    connection: {
      provider: 'jira',
      url: 'https://team.atlassian.net/browse/TEAM-12',
      credentialEnv: 'WORKSPACER_SOURCE_PRIVATE_TOKEN',
    },
  });
  providerRead.mockResolvedValue({ nativeId: 'TEAM-12', snapshot: candidate });
  await sources.request({
    action: 'refreshSource',
    id: 'w',
    sourceId: 'ticket',
    expectedVersion: 1,
  });
  const contextDir = path.join(f.projectRoot, '.rivet', 'context', 'domains');
  fs.mkdirSync(contextDir, { recursive: true });
  const doc = path.join(contextDir, 'rules.md');
  fs.writeFileSync(doc, '# Rules\nCAPTURED RULE: preserve access checks.\n');
  const captureKnowledge = (captureId: string) => {
    const catalog = store.knowledge.request({ action: 'knowledge', id: 'w' });
    if (catalog.action !== 'knowledge') throw new Error('Expected catalog');
    const result = store.knowledge.request({
      action: 'captureKnowledge',
      id: 'w',
      expectedRevision: 2,
      captureId,
      path: catalog.documents[0].path,
      expectedSha256: catalog.documents[0].sha256,
    });
    if (result.action !== 'captureKnowledge') throw new Error('Expected capture');
    return result.capture;
  };
  const knowledge = captureKnowledge('rules-v1');
  store.request({
    action: 'addEvidence',
    id: 'w',
    expectedRevision: 2,
    evidenceId: 'checked',
    criterionId: 'r2:c1',
    note: 'USER EVIDENCE EXCERPT: checked exported heading',
    reference: '',
    assessment: 'user-verified',
  });
  store.request({
    action: 'createAlternativeGroup',
    id: 'w',
    expectedRevision: 2,
    groupId: 'alternatives',
    title: 'Export approach',
    purpose: 'Compare two formats',
    budgetMinutes: 15,
    alternatives: [
      {
        id: 'a',
        title: 'Chosen CSV approach',
        hypothesis: 'SELECTED HYPOTHESIS A',
        artifactIds: [],
        executionIds: [],
      },
      {
        id: 'b',
        title: 'Another approach',
        hypothesis: 'UNSELECTED HYPOTHESIS B',
        artifactIds: [],
        executionIds: [],
      },
    ],
  });
  store.request({
    action: 'selectAlternative',
    id: 'w',
    expectedRevision: 2,
    selectionId: 'choice-a',
    groupId: 'alternatives',
    alternativeId: 'a',
    reason: 'Keep existing consumers compatible',
  });
  const launch = store.request({
    action: 'prepareExecution',
    id: 'w',
    expectedRevision: 2,
    executionId: 'future-launch',
    task: 'Implement export',
  });
  if (launch.action !== 'prepareExecution') throw new Error('Expected launch');
  store.request({
    action: 'linkExecution',
    id: 'w',
    executionId: 'future-launch',
    session: {
      sessionId: 'worker',
      hub: '',
      label: 'Worker',
      provider: 'codex',
      cwd: f.projectRoot,
    },
  });
  const direction = store.request({
    action: 'prepareDirection',
    id: 'w',
    expectedRevision: 2,
    directionId: 'future-direction',
    executionId: 'future-launch',
    text: 'Keep the export focused',
  });
  if (direction.action !== 'prepareDirection') throw new Error('Expected direction');
  const continuation = store.request({
    action: 'prepareControl',
    id: 'w',
    expectedRevision: 2,
    controlId: 'future-continuation',
    executionId: 'future-launch',
    kind: 'continue',
    text: 'Continue export checks',
  });
  if (continuation.action !== 'prepareControl') throw new Error('Expected continuation');
  const originals = [
    launch.execution.contextPacket!,
    direction.direction.packet,
    continuation.control.packet,
  ];
  for (const packet of originals) {
    for (const expected of [
      accepted.content,
      accepted.digest,
      accepted.revision,
      'TEAM-12',
      knowledge.sha256,
      knowledge.content.trim(),
      'USER EVIDENCE EXCERPT',
      'SELECTED HYPOTHESIS A',
    ])
      expect(packet).toContain(expected);
    for (const excluded of [
      candidate.content,
      candidate.digest,
      'WORKSPACER_SOURCE_PRIVATE_TOKEN',
      'PRIVATE_PACKET_CREDENTIAL_VALUE',
      'UNSELECTED HYPOTHESIS B',
    ])
      expect(packet).not.toContain(excluded);
  }
  const frozenLaunch = String(
    f.db.prepare('SELECT snapshot FROM intent_executions WHERE id=?').get('future-launch')!
      .snapshot,
  );
  const frozenDirection = String(
    f.db.prepare('SELECT snapshot FROM intent_directions WHERE id=?').get('future-direction')!
      .snapshot,
  );
  const frozenContinue = String(
    f.db.prepare('SELECT snapshot FROM intent_controls WHERE id=?').get('future-continuation')!
      .snapshot,
  );
  await sources.request({
    action: 'acceptSource',
    id: 'w',
    sourceId: 'ticket',
    expectedVersion: 2,
    candidateDigest: candidate.digest,
  });
  fs.writeFileSync(doc, '# Rules\nNEW CAPTURED RULE: add review notes.\n');
  const newKnowledge = captureKnowledge('rules-v2');
  store.request({
    action: 'selectAlternative',
    id: 'w',
    expectedRevision: 2,
    selectionId: 'choice-b',
    expectedSelectionId: 'choice-a',
    groupId: 'alternatives',
    alternativeId: 'b',
    reason: 'Reconsidered the alternative explicitly',
  });
  const later = store.request({
    action: 'prepareExecution',
    id: 'w',
    expectedRevision: 2,
    executionId: 'later-launch',
    task: 'Use the latest accepted context',
  });
  if (later.action !== 'prepareExecution') throw new Error('Expected later launch');
  expect(later.execution.contextPacket).toContain(candidate.content);
  expect(later.execution.contextPacket).toContain(newKnowledge.sha256);
  expect(later.execution.contextPacket).toContain('NEW CAPTURED RULE');
  expect(later.execution.contextPacket).toContain('UNSELECTED HYPOTHESIS B');
  expect(later.execution.contextPacket).not.toContain('SELECTED HYPOTHESIS A');
  expect(
    String(
      f.db.prepare('SELECT snapshot FROM intent_executions WHERE id=?').get('future-launch')!
        .snapshot,
    ),
  ).toBe(frozenLaunch);
  expect(
    String(
      f.db.prepare('SELECT snapshot FROM intent_directions WHERE id=?').get('future-direction')!
        .snapshot,
    ),
  ).toBe(frozenDirection);
  expect(
    String(
      f.db.prepare('SELECT snapshot FROM intent_controls WHERE id=?').get('future-continuation')!
        .snapshot,
    ),
  ).toBe(frozenContinue);
  expect(
    String(
      f.db.prepare('SELECT snapshot FROM intent_executions WHERE id=?').get('old-run')!.snapshot,
    ),
  ).toBe(f.execution);
  expect(network).not.toHaveBeenCalled();
  expect(providerComment).not.toHaveBeenCalled();
  expect(providerRead).toHaveBeenCalledTimes(2);
});
