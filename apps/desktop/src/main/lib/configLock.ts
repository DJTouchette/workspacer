/**
 * Cross-process advisory lock for config.yaml.
 *
 * config.yaml has two writers — this process (the Settings pane, in-process) and
 * the Go brain (config.save over the bus, which is what the web and mobile
 * Settings panes use). Both do refresh-if-changed → deepMerge → atomic write, and
 * the mtime gate each of them has closes only the first step. Nothing spans the
 * three, so an interleaved write from the other process is silently lost and both
 * report success.
 *
 * Neither process can simply own the file: headless `workspacer serve` runs with
 * no Electron, and the brain is optional (the hub serves brain-less when no
 * binary is found). So the file arbitrates instead. See
 * `contracts/config-lock.json` — the Go twin is `cmd/brain/configlock.go`, and
 * `staleMs` in particular must agree between them.
 *
 * Deliberately synchronous: `configService.saveConfig` is synchronous and is
 * called straight from an `ipcMain.handle`, so making this async would ripple
 * through the whole config path. That does mean it blocks the Electron main
 * thread, which is why the wait budget here is a quarter of a second rather than
 * the seconds the brain allows itself — a lock this side cannot get quickly is
 * reported as a failed save, not waited out.
 */
import * as fs from 'fs';
import * as path from 'path';

/** Beside the file it guards, so it shares its filesystem and permissions. */
const LOCK_SUFFIX = '.lock';
/**
 * A lock older than this is treated as abandoned and stolen.
 *
 * MUST match the Go twin. A side that expires locks sooner than the other will
 * steal one the other still believes it holds — which is worse than no lock,
 * because both then write believing they are exclusive.
 */
export const LOCK_STALE_MS = 10_000;
/** Main-thread budget. Short on purpose — see the note above. */
const MAX_WAIT_MS = 250;
const RETRY_MS = 10;

/** Block the calling thread. `saveConfig` is sync, so there is no yielding here. */
function sleepSync(ms: number): void {
  // Atomics.wait on a throwaway buffer is the one portable synchronous sleep in
  // Node; a busy-loop would spin a core for the same wall time.
  Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, ms);
}

export class ConfigLockTimeout extends Error {
  constructor(path: string) {
    super(`config.yaml is locked by another process (waited ${MAX_WAIT_MS}ms): ${path}`);
    this.name = 'ConfigLockTimeout';
  }
}

/**
 * Run `fn` holding the lock for `configPath`.
 *
 * Throws {@link ConfigLockTimeout} if the lock cannot be taken in time — the
 * caller must treat that as a failed save rather than writing anyway, which is
 * the whole point.
 */
export function withConfigLock<T>(configPath: string, fn: () => T): T {
  const lockPath = configPath + LOCK_SUFFIX;
  // The lock lives beside the file it guards, and on a fresh install that
  // directory does not exist yet — the writer creates it, and the writer runs
  // INSIDE the lock. Without this the very first save on a new machine throws
  // ENOENT here and is reported as a refused save.
  fs.mkdirSync(path.dirname(configPath), { recursive: true });
  const deadline = Date.now() + MAX_WAIT_MS;

  for (;;) {
    try {
      // 'wx' is O_CREAT|O_EXCL — atomic "create only if absent" against both the
      // other process and another thread here.
      const fd = fs.openSync(lockPath, 'wx');
      try {
        fs.writeSync(fd, `${process.pid} ${new Date().toISOString()}\n`);
      } catch {
        /* diagnostics only; holding the lock is what matters */
      } finally {
        fs.closeSync(fd);
      }
      break;
    } catch (err) {
      if ((err as NodeJS.ErrnoException).code !== 'EEXIST') throw err;

      // Held. Steal it if the holder died mid-write, or wait a moment.
      if (lockIsStale(lockPath)) {
        try {
          fs.rmSync(lockPath, { force: true });
        } catch {
          /* another waiter got there first — the retry below re-races fairly */
        }
        continue;
      }
      if (Date.now() >= deadline) throw new ConfigLockTimeout(configPath);
      sleepSync(RETRY_MS);
    }
  }

  try {
    return fn();
  } finally {
    try {
      fs.rmSync(lockPath, { force: true });
    } catch {
      /* a failed release would wedge config until staleMs; nothing better to do */
    }
  }
}

/** True when the lock file is old enough that its holder must have died. */
function lockIsStale(lockPath: string): boolean {
  try {
    return Date.now() - fs.statSync(lockPath).mtimeMs > LOCK_STALE_MS;
  } catch {
    // Vanished between the EEXIST and the stat — the holder released it, so it
    // is not stale, it is gone. Retrying will take it.
    return false;
  }
}
