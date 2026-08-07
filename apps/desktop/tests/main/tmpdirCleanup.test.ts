/**
 * The guard on the guard.
 *
 * tests/support/tmpdirCleanup.ts stops the main suite leaking mkdtemp sandboxes
 * into a per-user-quota /tmp. A full quota does not present as a test failure —
 * it presents as the SHELL becoming unable to exec, which is how it went
 * unnoticed until 213k stale directories had to be swept by hand.
 *
 * Its failure mode is silence. Drop `setupFiles` from vitest.config.ts and every
 * suite still passes; the leak simply resumes and nothing says so for weeks. So
 * both halves are pinned here: that the wiring exists in the config, and — the
 * part a config grep cannot tell you — that the setup actually RAN in this
 * process and the redirect is live.
 */
import { describe, it, expect } from 'vitest';
import fs from 'fs';
import os from 'os';
import path from 'path';

describe('temp-dir cleanup is wired', () => {
  it('vitest.config.ts still registers the setup file', () => {
    const config = fs.readFileSync(path.join(__dirname, '..', '..', 'vitest.config.ts'), 'utf8');
    expect(config).toMatch(/setupFiles\s*:/);
    expect(config).toContain('tests/support/tmpdirCleanup');
  });

  it('the redirect is live in this process, not merely configured', () => {
    // The setup file points TMPDIR at a per-file sandbox it created. If it did
    // not run, os.tmpdir() is the bare system temp dir and this fails — which is
    // the case a config-only assertion would happily miss.
    const tmp = fs.realpathSync(os.tmpdir());
    expect(path.basename(tmp)).toMatch(/^wks-vitest-/);
    expect(process.env.TMPDIR).toBeTruthy();
  });

  it('a sandbox made through os.tmpdir() lands inside this file own sandbox', () => {
    // The property that actually reclaims the leak: whatever idiom a test uses,
    // its sandbox is inside the one directory afterAll removes wholesale.
    const dir = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'wks-probe-')));
    expect(dir.startsWith(fs.realpathSync(os.tmpdir()) + path.sep)).toBe(true);
  });
});
