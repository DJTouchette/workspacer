/**
 * Global temp-directory reclamation for the main-process suite.
 *
 * `/tmp` here is a per-user-quota tmpfs, and one `vitest run` used to leak ~92
 * mkdtemp directories that nothing ever removed. That is not merely untidy: when
 * the quota fills, a shell can no longer write its redirection targets, so it
 * forks and then dies before exec — every command returns a nonzero exit with no
 * output. That failure took down an entire agent run before anyone connected it
 * to test hygiene, and 213k stale `wks-*` directories had to be swept by hand.
 *
 * The fix is deliberately NOT 69 hand-written `afterEach(rm)` calls across 35
 * files. That treats the instances and leaves the class open: the 70th call site
 * lands unswept and nothing notices until the quota does.
 *
 * Nor is it patching `fs.mkdtempSync`, which was tried first and reclaimed only
 * 3 of 92. Test files that say `import * as fs from 'fs'` bind an ESM namespace
 * object, and those are immutable — assigning to the CJS module object is simply
 * not visible to them. A fix that works for one import idiom and silently misses
 * the other is worse than none, because the leak count looks like progress.
 *
 * So instead: give each test FILE its own TMPDIR. `os.tmpdir()` reads the env var
 * on every call on POSIX, so every mkdtemp in that file — whatever idiom it used,
 * and any subprocess it spawns, which inherits the variable — lands inside one
 * directory this file owns. Cleanup is then a single recursive remove that cannot
 * miss a call site, because it never had to know about call sites at all.
 *
 * Per FILE, not per test: a sandbox stays alive for everything in the file,
 * including across `describe` blocks and any test that asserts on a sibling's
 * leftovers. Per file also means no cross-worker race — vitest runs files in
 * parallel, and each one is deleting only a directory it alone created.
 *
 * Registered via vitest.config.ts `setupFiles`; tests need no import and no
 * change. tmpdirCleanup.test.ts asserts that wiring is still in place, because a
 * setup file that quietly stops being loaded looks exactly like one that works.
 */
import fs from 'fs';
import os from 'os';
import path from 'path';
import { afterAll } from 'vitest';

/** The real system temp dir, captured before we redirect anything at it. */
const systemTmp = fs.realpathSync(os.tmpdir());

/**
 * This file's own sandbox. Named so a stray one is traceable to the suite that
 * made it rather than looking like application debris.
 */
const fileTmp = fs.mkdtempSync(path.join(systemTmp, 'wks-vitest-'));

// os.tmpdir() consults TMPDIR/TMP/TEMP on each call, so redirecting all three
// covers POSIX and Windows and any child process that inherits the environment.
process.env.TMPDIR = fileTmp;
process.env.TMP = fileTmp;
process.env.TEMP = fileTmp;

afterAll(() => {
  // Restore first, so anything running after teardown (a lingering handle, a
  // reporter) sees a real temp dir rather than one about to vanish.
  delete process.env.TMPDIR;
  delete process.env.TMP;
  delete process.env.TEMP;
  try {
    // Confined by construction: fileTmp is a path this module created directly
    // under the real system temp dir, so this can only ever remove our own.
    if (fileTmp.startsWith(systemTmp + path.sep)) {
      fs.rmSync(fileTmp, { recursive: true, force: true });
    }
  } catch {
    /* best effort — a leaked directory must never turn a passing file red */
  }
});
