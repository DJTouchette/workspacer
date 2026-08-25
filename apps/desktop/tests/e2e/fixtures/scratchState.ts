/**
 * The blast shield for every E2E fixture that boots a real workspacer binary.
 *
 * Why this file exists: a second stack on this machine can silently open the
 * developer's LIVE state — the same files the app they are running right now is
 * writing to — and a test suite that can do that is worse than no test suite.
 *
 * The sharpest instance of that has since been closed on master (`105a5f25`
 * pins claudemon's `--db-path` in the serve plan and refuses to share it), but
 * the SHAPE of the hazard is structural and still live: every persistent path
 * these binaries touch is derived from `$HOME` / `os.UserConfigDir()` when it
 * is not passed, the desktop's own claudemon spawn still omits `--db-path`, and
 * a fixture is exactly the caller most likely to forget a flag. So this file
 * assumes nothing about which flags a binary happens to pin today.
 *
 * The defence is three layers, in increasing strength:
 *
 *   1. **Redirect.** `scratchEnv()` points HOME and every XDG_* variable at a
 *      throwaway directory, so any path a child derives from `~` or
 *      `os.UserConfigDir()` lands in the scratch tree. The hub computes its
 *      `--tokens-file`, `--layout-file`, `--push-dir`, `--peers-file` and
 *      `--jobs-file` defaults from exactly those (`cmd/hub/main.go:217-245`,
 *      `internal/authtoken/authtoken.go:233-252`), and it computes them in the
 *      CHILD process, so the child's env is what decides.
 *   2. **Assert the redirect.** `assertScratchEnv()` / `assertScratchPath()`
 *      refuse an env or a path that is not under the scratch root, so a future
 *      edit that drops one variable fails on the spot instead of quietly
 *      reaching for the real one.
 *   3. **Prove it at runtime.** `assertNoLiveStateHandles(pid)` walks
 *      `/proc/<pid>/fd` after the child is up and throws if it holds ANY
 *      descriptor under a real state path. Layers 1 and 2 are promises about
 *      what the code intends; this one is a measurement of what it did.
 *
 * Ports get the same treatment: `freePort()` binds :0 and hands back what the
 * kernel gave, and `assertNotLivePort()` refuses the running app's ports
 * outright, so no fixture can ever be pointed at the hub/claudemon the
 * developer is using.
 *
 * NOTE ON /tmp: the scratch root deliberately lives under `~/.cache`, not
 * `os.tmpdir()`. /tmp on this project's dev boxes is a per-user-quota tmpfs and
 * leaked test dirs have exhausted it before. Override with `WKS_E2E_SCRATCH`.
 */
import { spawnSync } from 'child_process';
import * as fs from 'fs';
import * as net from 'net';
import * as os from 'os';
import * as path from 'path';

/** Ports the developer's live stack listens on. Never ours, under any flag. */
export const LIVE_PORTS = [7890, 7891, 7895] as const;

/**
 * Absolute paths that belong to the developer's real installation. Anything a
 * fixture-spawned process touches under one of these is a bug in the fixture.
 * Derived from the REAL home, deliberately read before we clobber HOME.
 */
export function liveStatePaths(): string[] {
  const home = realHome();
  if (!home) return [];
  return [
    path.join(home, '.config', 'workspacer'), // config.yaml, tokens.json, remote-token, peers.json, workspacer.db
    path.join(home, '.config', 'workspacer-hub'), // layout.json, jobs.json, push keys
    path.join(home, '.local', 'share', 'claudemon'), // state.db (XDG_DATA_HOME default)
    path.join(home, '.claudemon'), // state.db (no-XDG fallback)
    path.join(home, '.workspacer'), // handoffs, reports, worktrees
  ];
}

/** The real home directory, captured before any fixture rewrites HOME. */
const REAL_HOME = os.homedir();
function realHome(): string {
  return REAL_HOME;
}

/** Root under which every scratch directory is created. */
export function scratchRoot(): string {
  return process.env.WKS_E2E_SCRATCH || path.join(REAL_HOME, '.cache', 'wks-e2e');
}

/** Create a fresh, uniquely-named scratch directory under the scratch root. */
export function makeScratchDir(prefix: string): string {
  const root = scratchRoot();
  fs.mkdirSync(root, { recursive: true });
  return fs.mkdtempSync(path.join(root, prefix.endsWith('-') ? prefix : prefix + '-'));
}

/**
 * Throw unless `p` is inside the scratch root. Use on every path a fixture is
 * about to hand a spawned binary — that is the moment a typo turns into a write
 * to the developer's config.
 */
export function assertScratchPath(p: string, what: string): string {
  const abs = path.resolve(p);
  const root = path.resolve(scratchRoot());
  if (abs !== root && !abs.startsWith(root + path.sep)) {
    throw new Error(
      `REFUSING to point ${what} at ${abs}: it is outside the E2E scratch root ${root}. ` +
        `A fixture must never hand a real path to a spawned binary.`,
    );
  }
  for (const live of liveStatePaths()) {
    if (abs === live || abs.startsWith(live + path.sep)) {
      throw new Error(`REFUSING to point ${what} at ${abs}: that is LIVE state (${live}).`);
    }
  }
  return abs;
}

/** Env for a spawned binary: the real environment with every home/state-bearing
 *  variable redirected into `dir`. Pass this, never `process.env`. */
export function scratchEnv(dir: string, extra: Record<string, string> = {}): NodeJS.ProcessEnv {
  assertScratchPath(dir, 'the scratch home');
  const home = path.join(dir, 'home');
  for (const sub of ['home', 'config', 'data', 'state', 'cache']) {
    fs.mkdirSync(path.join(dir, sub), { recursive: true });
  }
  return {
    ...process.env,
    HOME: home,
    USERPROFILE: home, // Windows equivalent, for the same reason
    XDG_CONFIG_HOME: path.join(dir, 'config'),
    XDG_DATA_HOME: path.join(dir, 'data'),
    XDG_STATE_HOME: path.join(dir, 'state'),
    XDG_CACHE_HOME: path.join(dir, 'cache'),
    // Belt and braces for the claudemon fallback that ignores XDG entirely.
    CLAUDEMON_DB_PATH: path.join(dir, 'data', 'claudemon', 'state.db'),
    ...extra,
  };
}

/** Throw unless every home/state variable in `env` points into the scratch root. */
export function assertScratchEnv(env: NodeJS.ProcessEnv): void {
  for (const key of ['HOME', 'XDG_CONFIG_HOME', 'XDG_DATA_HOME', 'XDG_STATE_HOME']) {
    const v = env[key];
    if (!v)
      throw new Error(`REFUSING to spawn: ${key} is unset, so the child would use the real one.`);
    assertScratchPath(v, `$${key}`);
  }
}

/** Ask the kernel for an unused loopback port. Never guess one. */
export async function freePort(): Promise<number> {
  const port = await new Promise<number>((resolve, reject) => {
    const srv = net.createServer();
    srv.on('error', reject);
    srv.listen(0, '127.0.0.1', () => {
      const p = (srv.address() as net.AddressInfo).port;
      srv.close(() => resolve(p));
    });
  });
  return assertNotLivePort(port);
}

/** Throw if `port` is one the developer's running stack owns. */
export function assertNotLivePort(port: number): number {
  if ((LIVE_PORTS as readonly number[]).includes(port)) {
    throw new Error(
      `REFUSING port ${port}: it belongs to the running workspacer stack (${LIVE_PORTS.join(', ')}).`,
    );
  }
  return port;
}

/**
 * Walk `/proc/<pid>/fd` and throw if the process holds a descriptor under any
 * live state path. This is the only check here that measures rather than
 * promises — call it AFTER the child is up and has opened its files.
 *
 * Non-Linux (or a /proc we cannot read) returns `false` rather than passing
 * silently, so a caller can report "unproven" instead of "proven safe".
 */
export function assertNoLiveStateHandles(pid: number): boolean {
  const fdDir = `/proc/${pid}/fd`;
  let entries: string[];
  try {
    entries = fs.readdirSync(fdDir);
  } catch {
    return false; // no procfs — cannot prove it either way
  }
  const live = liveStatePaths();
  const offending: string[] = [];
  for (const fd of entries) {
    let target: string;
    try {
      target = fs.readlinkSync(path.join(fdDir, fd));
    } catch {
      continue; // fd closed between readdir and readlink
    }
    if (!target.startsWith('/')) continue; // socket:[...], pipe:[...], anon_inode:...
    for (const l of live) {
      if (target === l || target.startsWith(l + path.sep)) offending.push(`fd ${fd} -> ${target}`);
    }
  }
  if (offending.length) {
    throw new Error(
      `pid ${pid} has ${offending.length} open handle(s) on LIVE workspacer state — ` +
        `the fixture leaked out of its scratch dir:\n  ${offending.join('\n  ')}`,
    );
  }
  return true;
}

/**
 * Serialise a build step across Playwright worker PROCESSES.
 *
 * Every e2e fixture here rebuilds the hub binary before booting it, because a
 * stale binary would mean the suite tests nothing. Playwright runs spec files
 * in parallel workers, so two fixtures can reach `go build -o hub ./cmd/hub`
 * for the SAME output path at the same moment and one of them ends up
 * launching a half-written file. That is not theoretical: it is what
 * `mobileClient` + `mobileNodes` did to each other the first time both ran.
 *
 * A directory is the mutex — `mkdir` is atomic on every filesystem we care
 * about, unlike "check then create".
 */
export function withBuildLock<T>(fn: () => T): T {
  const lock = path.join(scratchRoot(), 'build.lock');
  fs.mkdirSync(path.dirname(lock), { recursive: true });
  const deadline = Date.now() + 300_000;
  for (;;) {
    try {
      fs.mkdirSync(lock);
      break;
    } catch {
      if (Date.now() > deadline) {
        // Five minutes means a crashed worker left the lock behind rather than
        // a live one holding it. Take it over rather than failing the run.
        fs.rmSync(lock, { recursive: true, force: true });
        continue;
      }
      // Synchronous on purpose: the callers are synchronous build steps, and an
      // async wait here would let a second worker into the critical section.
      spawnSync('sleep', ['0.25']);
    }
  }
  try {
    return fn();
  } finally {
    fs.rmSync(lock, { recursive: true, force: true });
  }
}

/** Delete a scratch directory, refusing anything outside the scratch root. */
export function removeScratchDir(dir: string): void {
  assertScratchPath(dir, 'the directory being deleted');
  fs.rmSync(dir, { recursive: true, force: true });
}
