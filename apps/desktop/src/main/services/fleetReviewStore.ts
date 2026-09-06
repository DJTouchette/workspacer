/** Immutable, host-owned review bytes. Never a live filesystem or revision API. */
import { execFile } from 'child_process';
import { randomUUID } from 'crypto';
import * as fs from 'fs';
import * as path from 'path';
import { promisify } from 'util';
import { gitArgs } from '../lib/gitExec';
import { atomicWriteFileSync } from '../lib/atomicWriteFile';
import { isSecretPath } from '../lib/pathConfinement';
import { getConfigDir } from './configService';
import type {
  FleetReviewEvidence,
  FleetReviewRequest,
  FleetReviewResponse,
} from '../shared/fleetReview';

const exec = promisify(execFile);
export const REVIEW_LIMITS = {
  records: 64,
  allocations: 256,
  files: 200,
  diffBytes: 1024 * 1024,
  totalBytes: 24 * 1024 * 1024,
};
export interface ReviewAllocation {
  projectRoot: string;
  allocatedCwd: string;
  commonDir: string;
  branch: string;
  baseCommit: string;
}
interface Allocation extends ReviewAllocation {
  generation: string;
  ownerSessionId: string;
  workerSessionId: string;
}
interface State {
  allocations: Allocation[];
  records: FleetReviewEvidence[];
}
function credentialPath(file: string): boolean {
  return file
    .split('/')
    .some((p) =>
      /^(?:\.env(?:\..*)?|\.ssh|\.aws|credentials(?:\.json)?|id_(?:rsa|ed25519)|.*\.(?:pem|p12|pfx|key))$/i.test(
        p,
      ),
    );
}
const sha = /^[a-f0-9]{40,64}$/;
async function git(cwd: string, args: string[]): Promise<string> {
  const { stdout } = await exec(
    'git',
    ['--no-replace-objects', '--literal-pathspecs', ...gitArgs(args)],
    {
      cwd,
      timeout: 15000,
      maxBuffer: REVIEW_LIMITS.diffBytes,
      env: { ...process.env, GIT_OPTIONAL_LOCKS: '0' },
    },
  );
  return stdout;
}
/** Called immediately after allocation, BEFORE setup hooks or a worker run. */
export async function reviewAllocation(
  project: string,
  cwd: string,
  branch: string,
): Promise<ReviewAllocation> {
  const projectRoot = fs.realpathSync(project);
  const allocatedCwd = fs.realpathSync(cwd);
  const commonDir = fs.realpathSync(
    (await git(cwd, ['rev-parse', '--path-format=absolute', '--git-common-dir'])).trim(),
  );
  const projectCommon = fs.realpathSync(
    (await git(project, ['rev-parse', '--path-format=absolute', '--git-common-dir'])).trim(),
  );
  const own = fs.realpathSync((await git(cwd, ['rev-parse', '--absolute-git-dir'])).trim());
  const baseCommit = (await git(cwd, ['rev-parse', '--verify', 'HEAD^{commit}'])).trim();
  if (commonDir !== projectCommon || own === commonDir || !sha.test(baseCommit))
    throw new Error('Not an isolated project worktree');
  return { projectRoot, allocatedCwd, commonDir, branch, baseCommit };
}

export class FleetReviewStore {
  constructor(
    private readonly filename: () => string,
    private readonly secret = isSecretPath,
  ) {}
  private load(): State {
    try {
      if (fs.statSync(this.filename()).size > REVIEW_LIMITS.totalBytes)
        return { allocations: [], records: [] };
      const state = JSON.parse(fs.readFileSync(this.filename(), 'utf8')) as State;
      if (!Array.isArray(state.allocations) || !Array.isArray(state.records))
        throw new Error('Invalid store');
      return state;
    } catch {
      return { allocations: [], records: [] };
    }
  }
  private save(state: State): void {
    state.allocations = state.allocations.slice(-REVIEW_LIMITS.allocations);
    state.records = state.records.slice(-REVIEW_LIMITS.records);
    while (
      Buffer.byteLength(JSON.stringify(state)) > REVIEW_LIMITS.totalBytes &&
      state.records.length
    )
      state.records.shift();
    atomicWriteFileSync(this.filename(), JSON.stringify(state), { mode: 0o600 });
  }
  register(ownerSessionId: string, workerSessionId: string, allocation: ReviewAllocation): void {
    const state = this.load();
    state.allocations = state.allocations.filter((a) => a.workerSessionId !== workerSessionId);
    state.allocations.push({
      ...allocation,
      ownerSessionId,
      workerSessionId,
      generation: randomUUID(),
    });
    this.save(state);
  }
  private pending = new Map<string, Promise<string | undefined>>();
  capture(
    owner: string,
    worker: string,
    lifecycle: FleetReviewEvidence['lifecycle'],
  ): Promise<string | undefined> {
    const allocation = this.load().allocations.find(
      (a) => a.ownerSessionId === owner && a.workerSessionId === worker,
    );
    if (!allocation) return Promise.resolve(undefined);
    const key = `${owner}:${worker}:${allocation.generation}`;
    const pending = this.pending.get(key);
    if (pending) return pending;
    const promise = this.captureNow(owner, worker, lifecycle).finally(() =>
      this.pending.delete(key),
    );
    this.pending.set(key, promise);
    return promise;
  }
  /** Teardown awaits this before removing the worktree, even inside the wake coalesce window. */
  async beforeRemove(cwd: string): Promise<void> {
    const canonicalCwd = fs.realpathSync(cwd);
    for (const a of this.load().allocations.filter((a) => a.allocatedCwd === canonicalCwd)) {
      await this.capture(a.ownerSessionId, a.workerSessionId, 'before-worktree-removal');
    }
  }
  private async captureNow(
    owner: string,
    worker: string,
    lifecycle: FleetReviewEvidence['lifecycle'],
  ): Promise<string | undefined> {
    const state = this.load();
    const a = state.allocations.find(
      (a) => a.ownerSessionId === owner && a.workerSessionId === worker,
    );
    if (!a) return undefined;
    const e: FleetReviewEvidence = {
      id: randomUUID(),
      allocationId: a.generation,
      ownerSessionId: owner,
      workerSessionId: worker,
      projectRoot: a.projectRoot,
      allocatedCwd: a.allocatedCwd,
      branch: a.branch,
      baseCommit: a.baseCommit,
      capturedAt: new Date().toISOString(),
      lifecycle,
      availability: 'unavailable',
      files: [],
    };
    try {
      if (!sha.test(a.baseCommit) || fs.realpathSync(a.allocatedCwd) !== a.allocatedCwd)
        throw new Error('Worktree identity changed');
      const common = fs.realpathSync(
        (
          await git(a.allocatedCwd, ['rev-parse', '--path-format=absolute', '--git-common-dir'])
        ).trim(),
      );
      const top = fs.realpathSync(
        (await git(a.allocatedCwd, ['rev-parse', '--show-toplevel'])).trim(),
      );
      const branch = (await git(a.allocatedCwd, ['symbolic-ref', '--short', 'HEAD'])).trim();
      if (common !== a.commonDir || top !== a.allocatedCwd || branch !== a.branch)
        throw new Error('Worktree identity changed');
      const head = (await git(a.allocatedCwd, ['rev-parse', '--verify', 'HEAD^{commit}'])).trim();
      if (!sha.test(head)) throw new Error('Invalid head');
      e.headCommit = head;
      await git(a.allocatedCwd, ['merge-base', '--is-ancestor', a.baseCommit, head]);
      const dirty = async () =>
        (await git(a.allocatedCwd, ['status', '--porcelain=v1', '-z', '--untracked-files=all']))
          .length > 0;
      if (await dirty()) {
        e.availability = 'dirty';
        e.reason =
          'Uncommitted work: review is available only in the retained worktree. No partial committed diff was captured.';
      } else {
        const range = `${a.baseCommit}..${head}`;
        const names = (
          await git(a.allocatedCwd, [
            'diff',
            '--no-textconv',
            '--name-status',
            '-z',
            '-M',
            range,
            '--',
          ])
        ).split('\0');
        let bytes = 0;
        const deadline = Date.now() + 15000;
        for (let i = 0; i < names.length && names[i];) {
          if (Date.now() > deadline) throw new Error('Capture time limit exceeded');
          const status = names[i++];
          const first = names[i++];
          const oldPath = /^[RC]/.test(status) ? first : undefined;
          const file = oldPath ? names[i++] : first;
          const paths = oldPath ? [oldPath, file] : [file];
          if (
            paths.some(
              (p) =>
                !p ||
                p.split('/').some((part) => part === '..' || part === '.git') ||
                path.isAbsolute(p) ||
                credentialPath(p) ||
                p.includes('\uFFFD') ||
                this.secret(path.join(a.projectRoot, p)),
            )
          )
            throw new Error('Restricted file in commit range');
          if (e.files.length >= REVIEW_LIMITS.files) throw new RangeError('Too many files');
          const diff = await git(a.allocatedCwd, [
            'diff',
            '--no-textconv',
            '--no-color',
            '-M',
            range,
            '--',
            ...paths,
          ]);
          bytes += Buffer.byteLength(diff);
          if (bytes > REVIEW_LIMITS.diffBytes) throw new RangeError('Diff too large');
          e.files.push({ path: file, oldPath, status, diff });
        }
        if ((await dirty()) || (await git(a.allocatedCwd, ['rev-parse', 'HEAD'])).trim() !== head)
          throw new Error('Worktree changed during capture');
        e.availability = 'captured';
      }
    } catch (err) {
      e.files = [];
      const oversized =
        err instanceof RangeError ||
        (err as NodeJS.ErrnoException).code === 'ERR_CHILD_PROCESS_STDIO_MAXBUFFER';
      e.availability = oversized ? 'oversized' : 'unavailable';
      e.reason = oversized
        ? 'Review exceeds the 200-file / 1 MiB capture limit; no partial diff retained.'
        : 'Worktree or commit range unavailable, changed, or contains restricted paths. No fallback repository is used.';
    }
    // Reload after async Git: do not undo a concurrent revocation or another capture.
    const current = this.load();
    if (
      !current.allocations.some(
        (x) =>
          x.ownerSessionId === owner &&
          x.workerSessionId === worker &&
          x.generation === a.generation,
      )
    )
      return undefined;
    const previous = [...current.records]
      .reverse()
      .find((x) => x.ownerSessionId === owner && x.workerSessionId === worker);
    // A cleanup after a turn result must not rewrite that result's immutable bytes.
    if (
      e.availability === 'unavailable' &&
      !fs.existsSync(a.allocatedCwd) &&
      previous?.lifecycle === 'before-worktree-removal' &&
      previous.allocationId === a.generation
    )
      return previous.id;
    current.records.push(e);
    this.save(current);
    return e.id;
  }
  /** Exact schema check: no arbitrary root, pathspec, revision, or command arguments. */
  read(request: unknown): FleetReviewResponse {
    const r = this.validate(request);
    if (!r) return { ok: false, error: 'Invalid review request' };
    const e = this.load().records.find(
      (e) =>
        e.id === r.evidenceId &&
        e.ownerSessionId === r.ownerSessionId &&
        e.workerSessionId === r.workerSessionId,
    );
    if (!e)
      return {
        ok: false,
        error: 'Review data unavailable, revoked, or outside the owning manager result',
      };
    if (r.file !== undefined && !e.files.some((f) => f.path === r.file))
      return { ok: false, error: 'File is not in this captured result' };
    return {
      ok: true,
      evidence: {
        ...e,
        files: e.files
          .filter((f) => r.file === undefined || f.path === r.file)
          .map((f) => ({ ...f, diff: r.file === undefined ? undefined : f.diff })),
      },
    };
  }
  forget(request: unknown): { ok: boolean } {
    const r = this.validate(request);
    if (!r || !this.read(r).ok) return { ok: false };
    const state = this.load();
    state.records = state.records.filter((e) => e.id !== r.evidenceId);
    // Revocation also prevents recapture by a late finish/cleanup for this worker.
    state.allocations = state.allocations.filter(
      (a) => a.ownerSessionId !== r.ownerSessionId || a.workerSessionId !== r.workerSessionId,
    );
    this.save(state);
    return { ok: true };
  }
  private validate(request: unknown): FleetReviewRequest | undefined {
    if (!request || typeof request !== 'object' || Array.isArray(request)) return;
    const r = request as FleetReviewRequest;
    if (
      Object.keys(r).some(
        (k) => !['ownerSessionId', 'workerSessionId', 'evidenceId', 'file'].includes(k),
      )
    )
      return;
    if (
      ![r.ownerSessionId, r.workerSessionId, r.evidenceId].every(
        (x) => typeof x === 'string' && x.length > 0 && x.length < 200,
      )
    )
      return;
    if (
      r.file !== undefined &&
      (typeof r.file !== 'string' ||
        r.file.length > 4096 ||
        r.file.includes('\0') ||
        r.file.split(/[\\/]/).includes('..') ||
        path.isAbsolute(r.file))
    )
      return;
    return r;
  }
}
export const fleetReviewStore = new FleetReviewStore(() =>
  path.join(getConfigDir(), 'fleet-review.json'),
);
