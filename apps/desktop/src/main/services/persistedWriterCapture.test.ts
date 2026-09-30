import { afterEach, expect, it, vi } from 'vitest';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { randomUUID } from 'node:crypto';
import { execFileSync } from 'node:child_process';
// Only the unused singleton locator is substituted. Classes below receive real
// isolated filenames; their serializers, locks, atomic writes and Git are real.
vi.mock('./configService', () => ({ getConfigDir: () => '/unused-capture-singleton' }));
import { DispatchHistoryStore } from './dispatchHistoryStore';
import { FleetReviewStore, reviewAllocation } from './fleetReviewStore';
import { ManagerReplacementState } from './managerReplacementState';
const scratch: string[] = [];
function directory(): string {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'wks-ts-writer-capture-'));
  scratch.push(dir);
  return fs.realpathSync(dir);
}
function capture(name: string, filename: string): void {
  const bytes = fs.readFileSync(filename);
  expect(bytes.length).toBeGreaterThan(100);
  // Explicit generator opt-in only. Never stringify/rewrite the writer's bytes.
  const output = process.env.WKS_PERSISTED_TS_CAPTURE;
  if (output) {
    fs.mkdirSync(output, { recursive: true });
    fs.writeFileSync(path.join(output, name), bytes);
  }
}
afterEach(() => {
  for (const dir of scratch.splice(0)) fs.rmSync(dir, { recursive: true, force: true });
});
it('captures task history emitted by the retained TypeScript writer', () => {
  const filename = path.join(directory(), 'dispatch-history.json');
  const store = new DispatchHistoryStore(() => filename);
  const owner = { sessionId: 'manager', isWakeTarget: true, status: 'active', label: 'Manager' };
  for (const sessionId of ['completed-worker', 'pending-worker'])
    expect(
      store.accept({
        owner,
        projectCwd: '/writer-project',
        executionCwd: '/writer-project',
        sessionId,
        title: sessionId,
        provider: 'codex',
      }),
    ).toBeDefined();
  store.observe({
    sessionId: 'completed-worker',
    status: 'ended',
    ambientState: 'idle',
    pendingApproval: null,
    pendingQuestions: null,
    usage: null,
    statusLine: { totalInputTokens: 123, totalOutputTokens: 45, costUSD: 0.25 },
  });
  store.validated('completed-worker', 'valid', 'retained-review-id');
  const raw = JSON.parse(fs.readFileSync(filename, 'utf8'));
  expect(raw.tasks).toHaveLength(2);
  expect(raw.tasks[0].attempts[0]).toMatchObject({
    lifecycle: 'ended',
    resultContract: 'valid',
    reviewEvidenceId: 'retained-review-id',
    metrics: { inputTokens: 123, outputTokens: 45, costUSD: 0.25 },
  });
  expect(raw).not.toHaveProperty('requests');
  expect(new DispatchHistoryStore(() => filename).list()).toHaveLength(2);
  capture('dispatch-history.json', filename);
});
it('captures real Git review evidence through the retained TypeScript writer', async () => {
  const root = directory(),
    repo = path.join(root, 'repo'),
    worktree = path.join(root, 'worker');
  fs.mkdirSync(repo);
  const git = (cwd: string, ...args: string[]) =>
    execFileSync('git', ['-c', 'commit.gpgsign=false', '-c', 'core.autocrlf=false', ...args], {
      cwd,
      encoding: 'utf8',
      env: {
        ...process.env,
        GIT_AUTHOR_DATE: '2026-09-01T00:00:00Z',
        GIT_COMMITTER_DATE: '2026-09-01T00:00:00Z',
      },
    }).trim();
  git(repo, 'init', '-q', '-b', 'main');
  git(repo, 'config', 'user.name', 'Writer Capture');
  git(repo, 'config', 'user.email', 'capture@example.invalid');
  fs.writeFileSync(path.join(repo, 'result.txt'), 'before\n');
  git(repo, 'add', '.');
  git(repo, 'commit', '-qm', 'base');
  git(repo, 'worktree', 'add', '-q', '-b', 'capture-worker', worktree);
  const filename = path.join(root, 'fleet-review.json'),
    store = new FleetReviewStore(() => filename);
  store.register(
    'manager',
    'review-worker',
    await reviewAllocation(repo, worktree, 'capture-worker'),
  );
  fs.writeFileSync(path.join(worktree, 'result.txt'), 'after retained review\n');
  git(worktree, 'add', '.');
  git(worktree, 'commit', '-qm', 'result');
  const id = await store.capture('manager', 'review-worker', 'turn-ended');
  expect(id).toBeTruthy();
  const request = {
    ownerSessionId: 'manager',
    workerSessionId: 'review-worker',
    evidenceId: id!,
    file: 'result.txt',
  };
  const before = store.read(request);
  expect(before.ok).toBe(true);
  if (!before.ok) throw Error('review capture missing');
  expect(before.evidence.availability).toBe('captured');
  expect(before.evidence.files[0].diff).toContain('+after retained review');
  git(repo, 'worktree', 'remove', '--force', worktree);
  fs.rmSync(repo, { recursive: true });
  expect(new FleetReviewStore(() => filename).read(request)).toEqual(before);
  expect(JSON.parse(fs.readFileSync(filename, 'utf8'))).not.toHaveProperty('readers');
  capture('fleet-review.json', filename);
});
it('captures an uncertain activating journal through the retained TypeScript writer', () => {
  const filename = path.join(directory(), 'manager-replacements.json'),
    store = new ManagerReplacementState(() => filename);
  const launch = {
    options: {
      manager: true,
      toolScope: 'operator' as const,
      cwd: '/writer-project',
      provider: 'codex' as const,
      transport: 'stream' as const,
    },
    grants: 'retained-identity',
  };
  store.rememberLaunch('old', launch);
  store.rememberChild({
    sessionId: 'queued-child',
    cwd: '/writer-project',
    parentSessionId: 'old',
  });
  store.edit((journal) =>
    journal.operations.push({
      operationId: randomUUID(),
      sourceSessionId: 'old',
      successorSessionId: 'new',
      paneId: 'pane',
      workspaceId: 'workspace',
      phase: 'activating',
      createdAt: Date.now(),
      updatedAt: Date.now(),
      committed: true,
      bound: true,
      artifactPath: '/writer-project/.workspacer/handoff.json',
      workerIds: ['worker'],
      taskIds: ['task'],
      deliveries: [
        { id: 'delivery', kind: 'kickoff', text: 'resume retained work', status: 'sending' },
      ],
      launch,
      metadata: [
        { sessionId: 'old', cwd: '/writer-project', isWakeTarget: true },
        { sessionId: 'worker', cwd: '/writer-project', parentSessionId: 'old' },
      ],
      signatures: { worker: 'recorded-signature' },
      finishes: { worker: { reply: 'retained completion', stopped: true } },
    }),
  );
  const raw = JSON.parse(fs.readFileSync(filename, 'utf8'));
  expect(raw.pendingMetadata['queued-child'].parentSessionId).toBe('old');
  expect(raw).not.toHaveProperty('pendingSignatures');
  expect(new ManagerReplacementState(() => filename).records()[0].deliveries[0].status).toBe(
    'sending',
  );
  capture('manager-replacements.json', filename);
});
