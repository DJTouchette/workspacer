/**
 * The advisory-lock MECHANISM, extracted from configLock.ts so a second file
 * with concurrent writers doesn't get a second, subtly different copy of it.
 *
 * O_EXCL create beside the guarded file (same filesystem, same permissions),
 * steal a lock older than `staleMs` (its holder died mid-write), give up rather
 * than write unlocked. Synchronous on purpose: both callers sit on synchronous
 * read-modify-write paths, and the point of a lock is that nothing interleaves
 * inside it.
 *
 * The POLICY stays with each caller, because it differs and one of the two is
 * contract-pinned:
 *  - configLock.ts's `staleMs` and lock filename are a cross-language contract
 *    with the Go brain (contracts/config-lock.json) — a side that expires locks
 *    sooner steals one the other still holds — and its wait budget is a quarter
 *    second because it blocks the Electron main thread from an ipcMain.handle.
 *  - briefService.ts guards a file agents write concurrently and can afford to
 *    wait longer, and has no second-language twin.
 */
import * as fs from 'fs';
import * as path from 'path';

/** Beside the file it guards, so it shares its filesystem and permissions. */
export const LOCK_SUFFIX = '.lock';

/** Block the calling thread. Both callers are sync, so there is no yielding. */
function sleepSync(ms: number): void {
  // Atomics.wait on a throwaway buffer is the one portable synchronous sleep in
  // Node; a busy-loop would spin a core for the same wall time.
  Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, ms);
}

/** True when the lock file is old enough that its holder must have died. */
function lockIsStale(lockPath: string, staleMs: number): boolean {
  try {
    return Date.now() - fs.statSync(lockPath).mtimeMs > staleMs;
  } catch {
    // Vanished between the EEXIST and the stat — the holder released it, so it
    // is not stale, it is gone. Retrying will take it.
    return false;
  }
}

export interface FileLockOptions {
  /** A lock older than this is treated as abandoned and stolen. */
  staleMs: number;
  /** How long to wait for a held lock before giving up. */
  maxWaitMs: number;
  /** Poll interval while waiting. */
  retryMs?: number;
  /** Thrown when the wait budget runs out. The caller supplies it so its own
   *  error type (and message) reaches its own callers unchanged. */
  onTimeout: (targetPath: string, waitedMs: number) => Error;
}

/**
 * Run `fn` holding an advisory lock for `targetPath`. Throws whatever
 * `onTimeout` returns if the lock cannot be taken in time — the caller must
 * treat that as a failed write rather than writing anyway, which is the whole
 * point.
 */
export function withFileLock<T>(targetPath: string, opts: FileLockOptions, fn: () => T): T {
  const lockPath = targetPath + LOCK_SUFFIX;
  const retryMs = opts.retryMs ?? 10;
  // The lock lives beside the file it guards, and that directory may not exist
  // yet — the writer creates it, and the writer runs INSIDE the lock. Without
  // this the very first write throws ENOENT here and is reported as refused.
  fs.mkdirSync(path.dirname(targetPath), { recursive: true });
  const deadline = Date.now() + opts.maxWaitMs;

  for (;;) {
    try {
      // 'wx' is O_CREAT|O_EXCL — atomic "create only if absent" against both
      // another process and another thread here.
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
      if (lockIsStale(lockPath, opts.staleMs)) {
        try {
          fs.rmSync(lockPath, { force: true });
        } catch {
          /* another waiter got there first — the retry below re-races fairly */
        }
        continue;
      }
      if (Date.now() >= deadline) throw opts.onTimeout(targetPath, opts.maxWaitMs);
      sleepSync(retryMs);
    }
  }

  try {
    return fn();
  } finally {
    try {
      fs.rmSync(lockPath, { force: true });
    } catch {
      /* a failed release wedges the file until staleMs; nothing better to do */
    }
  }
}
