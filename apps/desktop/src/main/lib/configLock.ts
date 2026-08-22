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
import { withFileLock } from './fileLock';

/**
 * The O_EXCL/steal/wait MECHANISM lives in ./fileLock — briefService.ts needs
 * the same primitive for a file agents write concurrently, and two hand-rolled
 * copies of a lock is exactly the drift a lock exists to prevent. Everything
 * BELOW is this lock's policy, and it is policy that is contract-pinned: the
 * lock filename and staleMs must agree with the Go twin.
 */

// The lock FILENAME (`<config.yaml>.lock`) is contract-pinned too and now lives
// in ./fileLock as LOCK_SUFFIX — configLock.test.ts asserts the composed path
// against contracts/config-lock.json's lockFileSuffix, so moving it did not
// move the guard.
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
  return withFileLock(
    configPath,
    {
      staleMs: LOCK_STALE_MS,
      maxWaitMs: MAX_WAIT_MS,
      retryMs: RETRY_MS,
      onTimeout: (p) => new ConfigLockTimeout(p),
    },
    fn,
  );
}
