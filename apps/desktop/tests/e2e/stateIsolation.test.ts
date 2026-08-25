/**
 * The safety proof for the `/app` E2E harness.
 *
 * The hazard is concrete, not theoretical. The hub derives `--tokens-file`,
 * `--layout-file`, `--push-dir`, `--peers-file` and `--jobs-file` from
 * `os.UserConfigDir()` when they are not given (`cmd/hub/main.go:217-245`), and
 * claudemon resolves its SQLite database from `XDG_DATA_HOME`/`$HOME` when
 * `--db-path` is not passed (`store/mod.rs:315-323`; the serve plan pins it
 * since `105a5f25`, the desktop's spawn still does not). A fixture that spawns
 * either binary with an un-redirected environment therefore reads and writes
 * the developer's REAL `~/.config/workspacer` and their live `state.db` — the
 * same files the app they are running right now is writing to. A test suite
 * that can corrupt a live session database is worse than no test suite.
 *
 * `fixtures/scratchState.ts` defends against that in three layers. This file
 * proves all three, and — the part that matters — proves the guards are
 * LOAD-BEARING rather than vacuous, by feeding them real paths and real live
 * ports and asserting they refuse.
 *
 * These are Playwright tests only because that is where the fixture lives; they
 * drive no browser.
 */
import { test, expect } from '@playwright/test';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import { startAppHub, type AppHub } from './fixtures/appHub';
import {
  LIVE_PORTS,
  assertNoLiveStateHandles,
  assertNotLivePort,
  assertScratchEnv,
  assertScratchPath,
  freePort,
  liveStatePaths,
  scratchRoot,
} from './fixtures/scratchState';

// ═══ layer 2 — the guards refuse anything that is not scratch ═══════════════
// Run first and without a hub: if these are vacuous, nothing below means much.

test.describe('the guards themselves', () => {
  test('assertScratchPath refuses every live state path', () => {
    const live = liveStatePaths();
    expect(
      live.length,
      'no live state paths were derived — the guard would be a no-op',
    ).toBeGreaterThan(0);

    for (const p of live) {
      expect(() => assertScratchPath(p, 'a test'), p).toThrow(/REFUSING/);
      expect(() => assertScratchPath(path.join(p, 'config.yaml'), 'a test'), p).toThrow(/REFUSING/);
    }
    // And anything else outside the scratch root, including the obvious ones.
    expect(() => assertScratchPath(os.homedir(), 'a test')).toThrow(/REFUSING/);
    expect(() => assertScratchPath('/etc/passwd', 'a test')).toThrow(/REFUSING/);
    // A near-miss sibling of the scratch root must not slip through on a prefix
    // match: `<root>-evil` starts with `<root>` as a string but is not inside it.
    expect(() => assertScratchPath(scratchRoot() + '-evil', 'a test')).toThrow(/REFUSING/);

    // …and accepts what it should.
    expect(assertScratchPath(path.join(scratchRoot(), 'ok', 'file.json'), 'a test')).toContain(
      scratchRoot(),
    );
  });

  test('assertScratchEnv refuses an env with any home variable unset or real', () => {
    const good = {
      HOME: path.join(scratchRoot(), 'x', 'home'),
      XDG_CONFIG_HOME: path.join(scratchRoot(), 'x', 'config'),
      XDG_DATA_HOME: path.join(scratchRoot(), 'x', 'data'),
      XDG_STATE_HOME: path.join(scratchRoot(), 'x', 'state'),
    };
    expect(() => assertScratchEnv(good)).not.toThrow();

    for (const key of Object.keys(good)) {
      // Unset — the child would fall back to the real one. This is the exact
      // shape of the claudemon `--db-path` hazard.
      expect(() => assertScratchEnv({ ...good, [key]: undefined }), key).toThrow(/REFUSING/);
      // Set, but to the developer's real directory.
      expect(() => assertScratchEnv({ ...good, [key]: os.homedir() }), key).toThrow(/REFUSING/);
    }
  });

  test('assertNotLivePort refuses the running stack’s ports', async () => {
    for (const p of LIVE_PORTS) {
      expect(() => assertNotLivePort(p), String(p)).toThrow(/REFUSING/);
    }
    // freePort() routes through the same guard, so it can never hand one back.
    const port = await freePort();
    expect(LIVE_PORTS).not.toContain(port);
  });

  test('assertNoLiveStateHandles detects a handle on live state', () => {
    // Prove the fd walk actually looks: open a descriptor on a real state path
    // in THIS process and check the guard catches it. Read-only, and closed
    // immediately — the point is the detector, not the file.
    const target = liveStatePaths().find((p) => fs.existsSync(p));
    test.skip(!target, 'no live workspacer state on this machine to prove the detector against');
    test.skip(!fs.existsSync('/proc'), 'no procfs — the fd walk is a no-op here');

    const fd = fs.openSync(target!, fs.constants.O_RDONLY);
    try {
      expect(() => assertNoLiveStateHandles(process.pid)).toThrow(/open handle\(s\) on LIVE/);
    } finally {
      fs.closeSync(fd);
    }
    // With it closed the same process passes, so the throw above was about the
    // handle and not about something incidental.
    expect(assertNoLiveStateHandles(process.pid)).toBe(true);
  });
});

// ═══ layers 1 + 3 — a real fixture hub, measured ════════════════════════════

test.describe('a fixture hub in flight', () => {
  let hub: AppHub;

  test.beforeAll(async () => {
    hub = await startAppHub();
  });
  test.afterAll(async () => {
    await hub?.stop();
  });

  test('holds no file descriptor on the developer’s live state', () => {
    test.skip(!fs.existsSync(`/proc/${hub.pid}/fd`), 'no procfs — cannot measure');
    // This is a measurement, not a promise: it walks the live process's open
    // descriptors. `startAppHub` already ran it once at boot; running it again
    // after the client work above covers anything opened lazily.
    expect(assertNoLiveStateHandles(hub.pid)).toBe(true);
  });

  test('runs with HOME and every XDG variable redirected into the scratch tree', () => {
    const envFile = `/proc/${hub.pid}/environ`;
    test.skip(!fs.existsSync(envFile), 'no procfs — cannot measure');

    const env: Record<string, string> = {};
    for (const entry of fs.readFileSync(envFile, 'utf8').split('\0')) {
      const eq = entry.indexOf('=');
      if (eq > 0) env[entry.slice(0, eq)] = entry.slice(eq + 1);
    }
    // Read out of the actual process, so this cannot pass by us asserting on
    // the object we intended to pass rather than the one the kernel got.
    assertScratchEnv(env);
    for (const key of ['HOME', 'XDG_CONFIG_HOME', 'XDG_DATA_HOME', 'XDG_STATE_HOME']) {
      expect(env[key], key).toContain(hub.scratchDir);
    }
  });

  test('listens on an ephemeral port, never the live stack’s', () => {
    const port = Number(new URL(hub.url).port);
    expect(port).toBeGreaterThan(0);
    expect(LIVE_PORTS).not.toContain(port);
  });

  test('persists its state inside the scratch directory', async () => {
    // A hub that wrote nowhere would pass every check above vacuously. Make it
    // write: the layout document is hub-owned and persisted to --layout-file.
    hub.setLayout({ agents: [], activeAgentId: '' });
    const layoutFile = path.join(hub.scratchDir, 'config', 'workspacer-hub', 'layout.json');
    await expect.poll(() => fs.existsSync(layoutFile), { timeout: 10_000 }).toBe(true);
    // …and it is where we said it would be.
    expect(() => assertScratchPath(layoutFile, 'the layout file')).not.toThrow();
  });
});
