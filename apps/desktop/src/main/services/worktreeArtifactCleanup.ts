import * as fs from 'fs/promises';
import * as path from 'path';
import { execFile } from 'child_process';
import { gitArgs } from '../lib/gitExec';
import {
  linkedWorktree,
  withWorktreeMaintenance,
  WORKTREE_ALLOCATION,
} from './worktreeMaintenance';

export interface CleanupSession {
  session_id?: string;
  mode: string;
  cwd?: string | null;
  live_cwd?: string | null;
  liveCwd?: string | null;
  updated_at?: string;
  status_line?: { cwd?: string; workspace?: { current_dir?: string; project_dir?: string } };
}
export interface CleanupState {
  maintenanceSupported: boolean;
  sessions: CleanupSession[];
}
export interface CleanupReport {
  root: string;
  apply: boolean;
  scannedWorktrees: number;
  removedBytes: number;
  reclaimableBytes: number;
  artifacts: Array<{
    worktree: string;
    path: string;
    bytes: number;
    action: 'planned' | 'removed' | 'skipped';
    reason?: string;
  }>;
  skipped: Array<{ worktree: string; reason: string }>;
  errors: Array<{ path: string; message: string }>;
}

export async function loadWorktreeCleanupState(daemonUrl: string): Promise<CleanupState> {
  const [health, response] = await Promise.all([
    fetch(`${daemonUrl}/health`, { signal: AbortSignal.timeout(10_000) }),
    fetch(`${daemonUrl}/sessions?state_only=true&include_archived=true&include_empty=true`, {
      signal: AbortSignal.timeout(10_000),
    }),
  ]);
  if (!health.ok || !response.ok) throw new Error('Daemon state unavailable; cleanup skipped');
  const sessions = await response.json();
  if (!Array.isArray(sessions) || sessions.some((s) => !s || typeof s.mode !== 'string'))
    throw new Error('Invalid daemon session state; cleanup skipped');
  return { maintenanceSupported: health.headers.get('X-Workspacer-Maintenance') === '1', sessions };
}

async function git(cwd: string, args: string[]): Promise<{ ok: boolean; output: string }> {
  return new Promise((resolve) =>
    execFile(
      'git',
      gitArgs(args),
      { cwd, timeout: 15_000, maxBuffer: 16 * 1024 * 1024 },
      (error, stdout) => resolve({ ok: !error, output: stdout || '' }),
    ),
  );
}
function contains(root: string, candidate: string): boolean {
  const relative = path.relative(root, candidate);
  return (
    !relative ||
    (relative !== '..' && !relative.startsWith(`..${path.sep}`) && !path.isAbsolute(relative))
  );
}
function overlaps(a: string, b: string): boolean {
  return contains(a, b) || contains(b, a);
}
async function canonical(value: string): Promise<string> {
  return fs.realpath(value).catch(() => path.resolve(value));
}
const names = new Set([
  'node_modules',
  'bin',
  'obj',
  'target',
  'build',
  'dist',
  '.next',
  '.nuxt',
  '.turbo',
  '.gradle',
  '__pycache__',
  '.venv',
]);
function generated(name: string, files: Set<string>): boolean {
  const node = files.has('package.json');
  const dotnet = [...files].some((file) => /\.(csproj|fsproj|vbproj)$/.test(file));
  const gradle = files.has('build.gradle') || files.has('build.gradle.kts');
  if (name === 'node_modules' || ['dist', '.next', '.nuxt', '.turbo'].includes(name)) return node;
  if (name === 'obj') return dotnet;
  if (name === 'bin') return dotnet || files.has('go.mod');
  if (name === 'target') return files.has('Cargo.toml') || files.has('pom.xml');
  if (name === 'build') return node || gradle || files.has('CMakeLists.txt');
  if (name === '.gradle') return gradle;
  if (name === '.venv') return files.has('pyproject.toml') || files.has('requirements.txt');
  return name === '__pycache__';
}

/** Bounded-to-source candidate walk. Full reference discovery is separate. */
async function scan(cwd: string): Promise<{ candidates: string[] }> {
  const candidates: string[] = [];
  const visit = async (dir: string): Promise<void> => {
    const entries = await fs.readdir(dir, { withFileTypes: true });
    const files = new Set(entries.filter((e) => e.isFile()).map((e) => e.name));
    for (const entry of entries) {
      const full = path.join(dir, entry.name);
      if (entry.isSymbolicLink()) continue;
      if (!entry.isDirectory()) continue;
      if (names.has(entry.name)) {
        if (generated(entry.name, files)) candidates.push(full);
        continue;
      }
      if (entry.name.startsWith('.')) continue;
      // A nested checkout belongs to its own lifecycle.
      if (await fs.lstat(path.join(full, '.git')).catch(() => null)) continue;
      await visit(full);
    }
  };
  await visit(cwd);
  return { candidates };
}

async function measure(dir: string): Promise<{ bytes: number; modified: number }> {
  let bytes = 0;
  let modified = (await fs.lstat(dir)).mtimeMs;
  for (const entry of await fs.readdir(dir, { withFileTypes: true })) {
    if (entry.name === '.git') throw new Error('Contains a nested Git checkout');
    const full = path.join(dir, entry.name);
    const stat = await fs.lstat(full);
    if (stat.isSymbolicLink()) continue;
    modified = Math.max(modified, stat.mtimeMs);
    if (stat.isDirectory()) {
      const child = await measure(full);
      bytes += child.bytes;
      modified = Math.max(modified, child.modified);
    }
    // Hardlinks may belong to a shared package store; do not claim their bytes
    // will be freed. Count allocated blocks where the platform exposes them.
    else if (stat.nlink <= 1)
      bytes += typeof stat.blocks === 'number' ? stat.blocks * 512 : stat.size;
  }
  return { bytes, modified };
}

async function sharedReferences(cwd: string): Promise<Array<{ source: string; target: string }>> {
  // Git's registry includes the primary checkout and siblings under OTHER
  // configured roots. Restricting discovery to this sweep's root misses them.
  const list = await git(cwd, ['worktree', 'list', '--porcelain', '-z']);
  if (!list.ok) throw new Error('Cannot inspect registered worktree references');
  const roots = list.output
    .split('\0')
    .filter((part) => part.startsWith('worktree '))
    .map((part) => part.slice(9));
  if (!roots.length) throw new Error('Invalid worktree registry');
  const references: Array<{ source: string; target: string }> = [];
  let directories = 0;
  const deadline = Date.now() + 30_000;
  const visit = async (dir: string): Promise<void> => {
    if (++directories > 50_000 || Date.now() > deadline)
      throw new Error('Shared-reference scan exceeded its budget; cleanup skipped');
    for (const entry of await fs.readdir(dir, { withFileTypes: true })) {
      if (entry.name === '.git') continue;
      const full = path.join(dir, entry.name);
      if (entry.isSymbolicLink()) {
        const target = await fs.realpath(full).catch((error) => {
          if (error.code === 'ENOENT') return null;
          throw error;
        });
        if (target) references.push({ source: full, target });
      } else if (entry.isDirectory()) await visit(full);
    }
  };
  for (const root of roots) {
    const stat = await fs.lstat(root).catch((error) => {
      if (error.code === 'ENOENT') return null; // prunable/missing checkout
      throw error;
    });
    if (!stat) continue;
    if (!stat.isDirectory() || stat.isSymbolicLink())
      throw new Error('Registered worktree path changed');
    await visit(root);
  }
  return references;
}

async function inactivityReason(
  cwd: string,
  state: CleanupState,
  now: number,
  minAge: number,
): Promise<string | undefined> {
  for (const session of state.sessions) {
    const raw = [
      session.cwd,
      session.live_cwd,
      session.liveCwd,
      session.status_line?.cwd,
      session.status_line?.workspace?.current_dir,
      session.status_line?.workspace?.project_dir,
    ].filter((value): value is string => typeof value === 'string' && !!value);
    if (session.mode !== 'stopped' && (!raw.length || raw.some((p) => !path.isAbsolute(p))))
      return 'A live session has no verifiable working directory';
    const paths = await Promise.all(raw.map(canonical));
    if (!paths.some((p) => overlaps(cwd, p))) continue;
    if (session.mode !== 'stopped') return 'A live session uses this worktree';
    const stopped = Date.parse(session.updated_at ?? '');
    if (!Number.isFinite(stopped) || now - stopped < minAge)
      return 'Recently stopped session; waiting for cleanup cooldown';
  }
}

/** Reclaim only recognized ignored artifacts. Never remove a worktree, branch,
 * source file, tracked path, arbitrary ignored directory, or symlink target. */
export async function cleanupWorktreeArtifacts(options: {
  root: string;
  apply: boolean;
  minAgeMs?: number;
  loadState: () => Promise<CleanupState>;
}): Promise<CleanupReport> {
  const minAge = options.minAgeMs ?? 60 * 60_000;
  if (!Number.isFinite(minAge) || minAge < 0) throw new Error('Invalid cleanup cooldown');
  const root = await canonical(options.root);
  const report: CleanupReport = {
    root,
    apply: options.apply,
    scannedWorktrees: 0,
    removedBytes: 0,
    reclaimableBytes: 0,
    artifacts: [],
    skipped: [],
    errors: [],
  };
  const roots = await fs.readdir(root, { withFileTypes: true }).catch((error) => {
    if (error.code === 'ENOENT') return [];
    throw error;
  });
  const worktrees: Array<{
    cwd: string;
    gitDir: string;
    created: number;
    dev: number;
    ino: number;
  }> = [];
  for (const repo of roots) {
    if (!repo.isDirectory() || repo.isSymbolicLink()) continue;
    const group = path.join(root, repo.name);
    for (const child of await fs.readdir(group, { withFileTypes: true })) {
      if (!child.isDirectory() || child.isSymbolicLink()) continue;
      const cwd = path.join(group, child.name);
      try {
        const linked = await linkedWorktree(cwd);
        if (
          !linked ||
          linked.cwd !== cwd ||
          !(await git(cwd, ['rev-parse', '--is-inside-work-tree'])).ok
        )
          continue;
        const common = await git(cwd, ['rev-parse', '--path-format=absolute', '--git-common-dir']);
        if (!common.ok || (await canonical(common.output.trim())) === linked.gitDir) continue;
        const stat = await fs.lstat(cwd);
        let created = stat.mtimeMs;
        const markerPath = path.join(linked.gitDir, WORKTREE_ALLOCATION);
        const marker = await fs.readFile(markerPath, 'utf8').catch((error) => {
          if (error.code === 'ENOENT') return null;
          throw error;
        });
        if (marker !== null) {
          const record = JSON.parse(marker);
          if (
            record.version !== 1 ||
            record.cwd !== cwd ||
            record.root !== root ||
            record.dev !== stat.dev ||
            record.ino !== stat.ino ||
            !Number.isFinite(Date.parse(record.createdAt))
          )
            throw new Error('Allocation identity changed; not cleaning');
          created = Date.parse(record.createdAt);
        } else {
          const branch = await git(cwd, ['symbolic-ref', '--short', 'HEAD']);
          if (!branch.ok || !branch.output.trim().startsWith('wks/')) continue;
        }
        worktrees.push({ cwd, gitDir: linked.gitDir, created, dev: stat.dev, ino: stat.ino });
      } catch (error) {
        report.skipped.push({ worktree: cwd, reason: String(error) });
      }
    }
  }
  report.scannedWorktrees = worktrees.length;
  if (!worktrees.length) return report;
  for (const tree of worktrees) {
    try {
      await withWorktreeMaintenance(tree.gitDir, async () => {
        const currentTree = await linkedWorktree(tree.cwd);
        const currentRoot = await fs.lstat(tree.cwd);
        if (
          currentTree?.cwd !== tree.cwd ||
          currentTree.gitDir !== tree.gitDir ||
          currentRoot.isSymbolicLink() ||
          currentRoot.dev !== tree.dev ||
          currentRoot.ino !== tree.ino
        )
          throw new Error('Worktree identity changed before cleanup');
        if (await fs.lstat(path.join(tree.gitDir, 'locked')).catch(() => null))
          throw new Error('Git worktree is locked; cleanup skipped');
        const state = await options.loadState();
        if (options.apply && !state.maintenanceSupported)
          throw new Error('Daemon lacks worktree-maintenance protection; update it before cleanup');
        const reason = await inactivityReason(tree.cwd, state, Date.now(), minAge);
        if (reason || Date.now() - tree.created < minAge) {
          report.skipped.push({
            worktree: tree.cwd,
            reason: reason ?? 'New worktree; waiting for cleanup cooldown',
          });
          return;
        }
        // Re-discover under the source lease: a child worktree could have
        // linked these dependencies since initial enumeration. createWorktree
        // holds this same lease while creating those links. Unreadable sibling
        // trees refuse cleanup because sharing cannot be ruled out.
        const references = await sharedReferences(tree.cwd);
        const { candidates } = await scan(tree.cwd);
        for (const candidate of candidates) {
          const item: CleanupReport['artifacts'][number] = {
            worktree: tree.cwd,
            path: candidate,
            bytes: 0,
            action: 'skipped',
          };
          report.artifacts.push(item);
          try {
            if (
              references.some(
                ({ source, target }) => !contains(candidate, source) && overlaps(candidate, target),
              )
            )
              throw new Error('Referenced by a managed worktree symlink');
            const rel = path.relative(tree.cwd, candidate).split(path.sep).join('/');
            const tracked = await git(tree.cwd, ['ls-files', '-z', '--', rel]);
            if (!tracked.ok || tracked.output)
              throw new Error('Contains tracked files or Git state is unavailable');
            const ignored = await git(tree.cwd, ['check-ignore', '-q', '--', rel]);
            if (!ignored.ok) throw new Error('Not Git-ignored');
            const original = await fs.lstat(candidate);
            if (
              !original.isDirectory() ||
              original.isSymbolicLink() ||
              (await fs.realpath(candidate)) !== candidate
            )
              throw new Error('Artifact path changed or traverses a symlink');
            const measured = await measure(candidate);
            item.bytes = measured.bytes;
            if (Date.now() - measured.modified < minAge)
              throw new Error('Artifact was recently written; waiting for cleanup cooldown');
            if (options.apply) {
              const current = await fs.lstat(candidate);
              if (
                !current.isDirectory() ||
                current.isSymbolicLink() ||
                current.dev !== original.dev ||
                current.ino !== original.ino ||
                (await fs.realpath(candidate)) !== candidate
              )
                throw new Error('Artifact identity changed during inspection');
              await fs.rm(candidate, { recursive: true });
              item.action = 'removed';
              report.removedBytes += item.bytes;
            } else item.action = 'planned';
            report.reclaimableBytes += item.bytes;
          } catch (error) {
            item.reason = String(error);
          }
        }
      });
    } catch (error) {
      report.skipped.push({ worktree: tree.cwd, reason: String(error) });
    }
  }
  return report;
}
