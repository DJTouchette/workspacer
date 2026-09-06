import { beforeEach, describe, expect, it, vi } from 'vitest';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import { execFileSync } from 'child_process';
const env = vi.hoisted(() => ({ config: '' }));
vi.mock('./configService', () => ({ getConfigDir: () => env.config }));
const sent = vi.hoisted(() => vi.fn().mockResolvedValue({ ok: true }));
vi.mock('./claudemonSessionClient', () => ({ claudemonSessionClient: { message: sent } }));
import { FleetReviewStore, fleetReviewStore, REVIEW_LIMITS } from './fleetReviewStore';
import { createWorktree, removeAgentWorktree } from './worktreeService';
import { supervisorNudge } from './supervisorNudge';
import { isSecretPath } from '../lib/pathConfinement';
import { parseFleetMessage } from '../shared/fleetMessages';
import type { ClaudeSessionState } from './claudeSessionStore';

let root: string;
let repo: string;
const git = (cwd: string, ...args: string[]) =>
  execFileSync('git', ['-c', 'commit.gpgsign=false', ...args], { cwd, encoding: 'utf8' }).trim();
const write = (cwd: string, file: string, content: string | Buffer) =>
  fs.writeFileSync(path.join(cwd, file), content);
const commit = (cwd: string) => {
  git(cwd, 'add', '.');
  git(cwd, 'commit', '-qm', 'test');
};
beforeEach(() => {
  root = fs.mkdtempSync(path.join(os.tmpdir(), 'fleet-review-'));
  env.config = path.join(root, 'config');
  repo = path.join(root, 'repo');
  fs.mkdirSync(repo);
  git(repo, 'init', '-q', '-b', 'main');
  git(repo, 'config', 'user.name', 'Test');
  git(repo, 'config', 'user.email', 'test@example.invalid');
  write(repo, 'first.txt', 'base\n');
  write(repo, 'delete.txt', 'remove me\n');
  commit(repo);
  sent.mockClear();
  supervisorNudge.forgetWorker('worker');
});
async function allocate() {
  const wt = await createWorktree({
    repoCwd: repo,
    name: 'worker',
    rootOverride: path.join(root, 'trees'),
  });
  expect(wt.reviewAllocation).toBeDefined();
  fleetReviewStore.register('manager', 'worker', wt.reviewAllocation!);
  return wt;
}
const req = (id: string, file?: string) => ({
  ownerSessionId: 'manager',
  workerSessionId: 'worker',
  evidenceId: id,
  ...(file === undefined ? {} : { file }),
});

it('production allocation, finish wake, and teardown retain a full multi-commit range across restart and manager HEAD changes', async () => {
  const wt = await allocate();
  write(wt.path!, 'first.txt', 'first commit\n');
  commit(wt.path!);
  write(wt.path!, 'second.txt', 'second commit\n');
  commit(wt.path!);
  const head = git(wt.path!, 'rev-parse', 'HEAD');
  const worker = {
    sessionId: 'worker',
    cwd: '/worker-supplied/wrong',
    label: 'Worker',
    status: 'active',
    ambientState: 'idle',
    conversation: [
      { role: 'user', content: 'task' },
      { role: 'assistant', content: 'commit: fabricated' },
    ],
  } as ClaudeSessionState;
  supervisorNudge.onFinished(worker, 'manager', 'commit: fabricated');
  // Production teardown can run BEFORE the 1.5-second coalesced wake.
  expect(
    (await removeAgentWorktree({ cwd: wt.path!, rootOverride: path.join(root, 'trees') })).ok,
  ).toBe(true);
  await vi.waitFor(() => expect(sent).toHaveBeenCalled(), { timeout: 5000 });
  const entry = parseFleetMessage(sent.mock.calls[0][1])!.entries[0];
  expect(entry.reviewEvidenceId).toBeTruthy();
  const request = req(entry.reviewEvidenceId!, 'first.txt');
  const before = fleetReviewStore.read(request);
  expect(before.ok).toBe(true);
  if (!before.ok) return;
  expect(before.evidence.headCommit).toBe(head);
  expect(before.evidence.baseCommit).toBe(wt.reviewAllocation!.baseCommit);
  expect(before.evidence.files[0].diff).toContain('-base');
  expect(before.evidence.files[0].diff).toContain('+first commit');
  git(repo, 'branch', '-D', wt.branch!);
  write(repo, 'first.txt', 'manager changed HEAD\n');
  commit(repo);
  const restarted = new FleetReviewStore(() => path.join(env.config, 'fleet-review.json'));
  expect(restarted.read(request)).toEqual(before);
  const list = restarted.read(req(entry.reviewEvidenceId!));
  expect(list.ok && list.evidence.files.map((f) => f.path)).toEqual(['first.txt', 'second.txt']);
  expect(list.ok && list.evidence.files.every((f) => f.diff === undefined)).toBe(true);
  expect(restarted.forget(req(entry.reviewEvidenceId!))).toEqual({ ok: true });
  expect(fleetReviewStore.read(request).ok).toBe(false);
  expect(await fleetReviewStore.capture('manager', 'worker', 'session-ended')).toBeUndefined();
  expect(fs.statSync(path.join(env.config, 'fleet-review.json')).mode & 0o777).toBe(0o600);
  expect(isSecretPath(path.join(env.config, 'fleet-review.json'))).toBe(true);
});

it('refuses wrong owner, worker, id, selectors and any revision/root/command arguments', async () => {
  const wt = await allocate();
  write(wt.path!, 'first.txt', 'changed\n');
  commit(wt.path!);
  const id = (await fleetReviewStore.capture('manager', 'worker', 'turn-ended'))!;
  for (const bad of [
    { ownerSessionId: 'other' },
    { workerSessionId: 'other' },
    { evidenceId: 'unknown' },
    { file: '../first.txt' },
    { file: '/etc/passwd' },
    { file: 'missing' },
    { revision: 'HEAD' },
    { cwd: repo },
    { command: 'status' },
    { file: ':(glob)*' },
  ]) {
    expect(fleetReviewStore.read({ ...req(id), ...bad }).ok).toBe(false);
  }
  expect(fleetReviewStore.forget({ ...req(id), ownerSessionId: 'other' }).ok).toBe(false);
  expect(fleetReviewStore.read(req(id)).ok).toBe(true);
});

it('retains honest dirty, unresolvable and missing-allocation states without partial diffs', async () => {
  const wt = await allocate();
  write(wt.path!, 'untracked.txt', 'unsaved work');
  const id = (await fleetReviewStore.capture('manager', 'worker', 'turn-ended'))!;
  const e = fleetReviewStore.read(req(id));
  expect(e.ok && e.evidence.availability).toBe('dirty');
  expect(e.ok && e.evidence.files).toEqual([]);
  expect(
    (await removeAgentWorktree({ cwd: wt.path!, rootOverride: path.join(root, 'trees') })).skipped,
  ).toBe(true);
  expect(
    await fleetReviewStore.capture('manager', 'in-place-worker', 'turn-ended'),
  ).toBeUndefined();
  // A new allocation with no resolvable worktree never falls back to project HEAD.
  fleetReviewStore.register('manager', 'gone', {
    ...wt.reviewAllocation!,
    allocatedCwd: path.join(root, 'gone'),
  });
  const gone = (await fleetReviewStore.capture('manager', 'gone', 'session-ended'))!;
  const missing = fleetReviewStore.read({ ...req(gone), workerSessionId: 'gone' });
  expect(missing.ok && missing.evidence.availability).toBe('unavailable');
});

it('captures literal filenames, rename/deletion, binary summary and executable mode without touching permissions', async () => {
  const wt = await allocate();
  git(wt.path!, 'mv', 'first.txt', 'renamed name.txt');
  git(wt.path!, 'rm', 'delete.txt');
  write(wt.path!, 'tab\tnewline\n.txt', 'odd name\n');
  write(wt.path!, 'binary.dat', Buffer.from([0, 1, 2, 3]));
  write(wt.path!, 'run.sh', '#!/bin/sh\n');
  fs.chmodSync(path.join(wt.path!, 'run.sh'), 0o755);
  commit(wt.path!);
  const id = (await fleetReviewStore.capture('manager', 'worker', 'turn-ended'))!;
  const list = fleetReviewStore.read(req(id));
  expect(list.ok && list.evidence.availability).toBe('captured');
  for (const file of [
    'renamed name.txt',
    'delete.txt',
    'tab\tnewline\n.txt',
    'binary.dat',
    'run.sh',
  ]) {
    const r = fleetReviewStore.read(req(id, file));
    expect(r.ok && r.evidence.files[0].diff).toBeTruthy();
  }
  const rename = fleetReviewStore.read(req(id, 'renamed name.txt'));
  expect(rename.ok && rename.evidence.files[0].oldPath).toBe('first.txt');
  const binary = fleetReviewStore.read(req(id, 'binary.dat'));
  expect(binary.ok && binary.evidence.files[0].diff).toContain('Binary files');
  expect(fs.statSync(path.join(wt.path!, 'run.sh')).mode & 0o777).toBe(0o755);
});

it('bounds oversize diffs and record retention, and never retains restricted paths', async () => {
  const wt = await allocate();
  write(wt.path!, 'large.txt', 'a'.repeat(REVIEW_LIMITS.diffBytes + 1));
  commit(wt.path!);
  const id = (await fleetReviewStore.capture('manager', 'worker', 'turn-ended'))!;
  const large = fleetReviewStore.read(req(id));
  expect(large.ok && large.evidence.availability).toBe('oversized');
  expect(large.ok && large.evidence.files).toEqual([]);
  git(wt.path!, 'reset', '--hard', wt.reviewAllocation!.baseCommit);
  write(wt.path!, '.env', 'TOKEN=must-not-be-retained');
  commit(wt.path!);
  const privateStore = new FleetReviewStore(() => path.join(env.config, 'secret-test.json'));
  privateStore.register('manager', 'worker', wt.reviewAllocation!);
  const secretId = (await privateStore.capture('manager', 'worker', 'turn-ended'))!;
  const secret = privateStore.read(req(secretId));
  expect(secret.ok && secret.evidence.availability).toBe('unavailable');
  expect(fs.readFileSync(path.join(env.config, 'secret-test.json'), 'utf8')).not.toContain(
    'must-not-be-retained',
  );
  const stateFile = path.join(env.config, 'fleet-review.json');
  const state = JSON.parse(fs.readFileSync(stateFile, 'utf8'));
  state.records = Array.from({ length: REVIEW_LIMITS.records }, (_, i) => ({
    ...state.records[0],
    id: `old-${i}`,
  }));
  fs.writeFileSync(stateFile, JSON.stringify(state));
  await fleetReviewStore.capture('manager', 'worker', 'turn-ended');
  expect(JSON.parse(fs.readFileSync(stateFile, 'utf8')).records).toHaveLength(
    REVIEW_LIMITS.records,
  );
  expect(fleetReviewStore.read(req('old-0')).ok).toBe(false);
  expect(fleetReviewStore.read(req('old-1')).ok).toBe(true);
});

it('rejects a changed branch instead of showing an earlier result, and reads symlinks only as committed text', async () => {
  const wt = await allocate();
  const outside = path.join(root, 'outside-secret');
  fs.writeFileSync(outside, 'not captured');
  fs.symlinkSync(outside, path.join(wt.path!, 'link'));
  commit(wt.path!);
  const first = (await fleetReviewStore.capture('manager', 'worker', 'turn-ended'))!;
  const link = fleetReviewStore.read(req(first, 'link'));
  expect(link.ok && link.evidence.files[0].diff).toContain(outside);
  expect(link.ok && link.evidence.files[0].diff).not.toContain('not captured');
  git(wt.path!, 'checkout', '-qb', 'changed-branch');
  const second = (await fleetReviewStore.capture('manager', 'worker', 'turn-ended'))!;
  expect(second).not.toBe(first);
  const changed = fleetReviewStore.read(req(second));
  expect(changed.ok && changed.evidence.availability).toBe('unavailable');
  expect(fleetReviewStore.read(req(first, 'link'))).toEqual(link);
});

it('refuses more than the bounded file count without retaining a partial list', async () => {
  const wt = await allocate();
  for (let i = 0; i <= REVIEW_LIMITS.files; i++) write(wt.path!, `file-${i}`, 'a\n');
  commit(wt.path!);
  const id = (await fleetReviewStore.capture('manager', 'worker', 'turn-ended'))!;
  const result = fleetReviewStore.read(req(id));
  expect(result.ok && result.evidence.availability).toBe('oversized');
  expect(result.ok && result.evidence.files).toEqual([]);
});

it('revocation during a new capture cannot resurrect evidence or its allocation', async () => {
  const wt = await allocate();
  write(wt.path!, 'first.txt', 'first\n');
  commit(wt.path!);
  const id = (await fleetReviewStore.capture('manager', 'worker', 'turn-ended'))!;
  const pending = fleetReviewStore.capture('manager', 'worker', 'session-ended');
  expect(fleetReviewStore.forget(req(id))).toEqual({ ok: true });
  expect(await pending).toBeUndefined();
  expect(fleetReviewStore.read(req(id)).ok).toBe(false);
});

it('captures before removal when the configured worktree root is a symlink', async () => {
  const physical = path.join(root, 'physical');
  fs.mkdirSync(physical);
  const alias = path.join(root, 'alias');
  fs.symlinkSync(physical, alias);
  const wt = await createWorktree({ repoCwd: repo, name: 'aliased', rootOverride: alias });
  expect(wt.reviewAllocation!.allocatedCwd).not.toBe(wt.path);
  fleetReviewStore.register('manager', 'worker', wt.reviewAllocation!);
  write(wt.path!, 'first.txt', 'alias change\n');
  commit(wt.path!);
  expect((await removeAgentWorktree({ cwd: wt.path!, rootOverride: alias })).ok).toBe(true);
  const id = (await fleetReviewStore.capture('manager', 'worker', 'session-ended'))!;
  const result = fleetReviewStore.read(req(id, 'first.txt'));
  expect(result.ok && result.evidence.availability).toBe('captured');
  expect(result.ok && result.evidence.lifecycle).toBe('before-worktree-removal');
});

it('a replacement allocation cannot inherit an earlier generation capture in flight', async () => {
  const wt = await allocate();
  write(wt.path!, 'first.txt', 'changed\n');
  commit(wt.path!);
  const old = fleetReviewStore.capture('manager', 'worker', 'turn-ended');
  fleetReviewStore.register('manager', 'worker', wt.reviewAllocation!);
  const current = fleetReviewStore.capture('manager', 'worker', 'turn-ended');
  expect(await old).toBeUndefined();
  expect(await current).toBeTruthy();
});
