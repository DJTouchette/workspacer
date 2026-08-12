/**
 * Tests for updateService — the electron-updater wiring.
 *
 * The load-bearing behaviour: it only acts in a packaged build (dev is a no-op),
 * it honours the `updates.enabled` config gate, it does a startup check plus a
 * ~4h interval re-check, and updater errors (offline / unsigned-mac refusal)
 * are swallowed and never bubble. electron + electron-updater are fully mocked.
 */

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { EventEmitter } from 'events';

// ─── electron mock ───────────────────────────────────────────────────────────
// `app.isPackaged` is mutated per-test to exercise the dev / packaged branches.
const electronApp = { isPackaged: true, getVersion: () => '0.0.0-test' };
// Default to "Later" (index 2). It must NOT default to the "What's new" index:
// that button re-prompts by design, so a test that emits update-downloaded
// without setting a response would loop forever rather than fail.
const showMessageBox = vi.fn(async () => ({ response: 2 }));
vi.mock('electron', () => ({
  app: electronApp,
  dialog: { showMessageBox: (...a: unknown[]) => showMessageBox(...(a as [])) },
  BrowserWindow: class {},
}));

// ─── electron-updater mock ───────────────────────────────────────────────────
// A real EventEmitter so the service's `.on(...)` handlers are exercised and we
// can drive lifecycle events (update-downloaded, error) from the test.
class MockUpdater extends EventEmitter {
  autoDownload = true;
  autoInstallOnAppQuit = true;
  channel = '';
  checkForUpdates = vi.fn(async () => ({}));
  quitAndInstall = vi.fn();
  setFeedURL = vi.fn();
}
let autoUpdater: MockUpdater;
vi.mock('electron-updater', () => ({
  get autoUpdater() {
    return autoUpdater;
  },
}));

// ─── config mock ─────────────────────────────────────────────────────────────
let configValue: any = { updates: { enabled: true, channel: 'latest' } };
vi.mock('./configService', () => ({
  configService: { getConfig: () => configValue },
}));

// ─── external-URL mock ───────────────────────────────────────────────────────
// The real openExternalUrl (hubCapabilities) is the shared scheme-checked
// opener; here we only care THAT the notes button routes through it and with
// which URL. Mocked as a module so the rest of that file's graph stays out.
const openExternalUrl = vi.fn(async (_url: string) => ({ ok: true }) as any);
vi.mock('./hubCapabilities', () => ({
  openExternalUrl: (url: string) => openExternalUrl(url),
}));

// A fake window that looks alive to the service.
function fakeWindow() {
  return { isDestroyed: () => false } as any;
}

// Fresh module + mocks per test so the singleton's internal state doesn't leak.
async function loadService() {
  autoUpdater = new MockUpdater();
  vi.resetModules();
  return (await import('./updateService')).updateService;
}

beforeEach(() => {
  vi.clearAllMocks();
  vi.spyOn(console, 'log').mockImplementation(() => {});
  vi.spyOn(console, 'warn').mockImplementation(() => {});
  electronApp.isPackaged = true;
  electronApp.getVersion = () => '0.0.0-test';
  configValue = { updates: { enabled: true, channel: 'latest' } };
});

afterEach(() => {
  vi.useRealTimers();
});

describe('updateService – gating', () => {
  it('no-ops in a dev (non-packaged) build', async () => {
    electronApp.isPackaged = false;
    const svc = await loadService();
    svc.start(fakeWindow());
    expect(autoUpdater.checkForUpdates).not.toHaveBeenCalled();
  });

  it('no-ops when updates.enabled is false', async () => {
    configValue = { updates: { enabled: false } };
    const svc = await loadService();
    svc.start(fakeWindow());
    expect(autoUpdater.checkForUpdates).not.toHaveBeenCalled();
  });

  it('runs when packaged and enabled (default config)', async () => {
    const svc = await loadService();
    svc.start(fakeWindow());
    expect(autoUpdater.checkForUpdates).toHaveBeenCalledTimes(1);
    svc.stop();
  });

  it('defaults enabled=true when the updates block is absent', async () => {
    configValue = {}; // no updates block at all
    const svc = await loadService();
    svc.start(fakeWindow());
    expect(autoUpdater.checkForUpdates).toHaveBeenCalledTimes(1);
    svc.stop();
  });

  it('nightly builds retarget the updater at the rolling nightly feed', async () => {
    electronApp.getVersion = () => '0.126.2-nightly.20260711.abc1234';
    const svc = await loadService();
    svc.start(fakeWindow());
    expect(autoUpdater.setFeedURL).toHaveBeenCalledWith({
      provider: 'generic',
      url: 'https://github.com/DJTouchette/workspacer/releases/download/nightly',
      useMultipleRangeRequest: false,
    });
    svc.stop();
  });

  it('nightly builds force the latest channel (the nightly feed has no channel ymls)', async () => {
    electronApp.getVersion = () => '0.126.2-nightly.20260711.2258';
    configValue = { updates: { enabled: true, channel: 'beta' } };
    const svc = await loadService();
    svc.start(fakeWindow());
    expect(autoUpdater.channel).toBe('latest');
    svc.stop();
  });

  it('stable builds keep the default (GitHub /releases/latest) feed', async () => {
    const svc = await loadService();
    svc.start(fakeWindow());
    expect(autoUpdater.setFeedURL).not.toHaveBeenCalled();
    svc.stop();
  });

  it('applies a well-formed configured channel to the updater', async () => {
    configValue = { updates: { enabled: true, channel: 'beta' } };
    const svc = await loadService();
    svc.start(fakeWindow());
    expect(autoUpdater.channel).toBe('beta');
    svc.stop();
  });

  // This used to assert VERBATIM pass-through of updates.channel. It isn't:
  // the channel is concatenated into the feed URL and is writable by anything
  // that can call config.save (bus + MCP facade included), so a traversal or a
  // scheme-relative value would repoint the updater at someone else's release
  // feed — code execution as the user, presented through our own install dialog.
  it('falls back to latest for a channel that could repoint the feed URL', async () => {
    for (const hostile of [
      '../../attacker/workspacer/releases/latest',
      '..',
      '/etc/passwd',
      '//evil.example.com/feed',
      'https://evil.example.com/latest',
      'beta/../../evil',
      '.hidden',
      '-flag',
      'has space',
    ]) {
      configValue = { updates: { enabled: true, channel: hostile } };
      const svc = await loadService();
      svc.start(fakeWindow());
      expect(autoUpdater.channel, hostile).toBe('latest');
      svc.stop();
    }
  });

  it('keeps ordinary channel names intact', async () => {
    for (const ok of ['latest', 'beta', 'alpha', 'next-2', 'rc_1', '1.2.x']) {
      configValue = { updates: { enabled: true, channel: ok } };
      const svc = await loadService();
      svc.start(fakeWindow());
      expect(autoUpdater.channel, ok).toBe(ok);
      svc.stop();
    }
  });

  it('falls back to latest for a non-string channel', async () => {
    configValue = { updates: { enabled: true, channel: { evil: true } } };
    const svc = await loadService();
    svc.start(fakeWindow());
    expect(autoUpdater.channel).toBe('latest');
    svc.stop();
  });
});

describe('updateService – scheduling', () => {
  it('re-checks on the ~4h interval', async () => {
    vi.useFakeTimers();
    const svc = await loadService();
    svc.start(fakeWindow());
    expect(autoUpdater.checkForUpdates).toHaveBeenCalledTimes(1); // startup

    vi.advanceTimersByTime(4 * 60 * 60 * 1000);
    expect(autoUpdater.checkForUpdates).toHaveBeenCalledTimes(2);

    vi.advanceTimersByTime(4 * 60 * 60 * 1000);
    expect(autoUpdater.checkForUpdates).toHaveBeenCalledTimes(3);
    svc.stop();
  });

  it('stop() cancels further scheduled checks', async () => {
    vi.useFakeTimers();
    const svc = await loadService();
    svc.start(fakeWindow());
    expect(autoUpdater.checkForUpdates).toHaveBeenCalledTimes(1);

    svc.stop();
    vi.advanceTimersByTime(4 * 60 * 60 * 1000 * 3);
    expect(autoUpdater.checkForUpdates).toHaveBeenCalledTimes(1); // no more
  });
});

describe('updateService – renderer status surface', () => {
  it('tracks the lifecycle: checking → downloading → downloaded, and installs on demand', async () => {
    const svc = await loadService();
    svc.start(fakeWindow());
    expect(svc.getStatus().state).toBe('idle');

    autoUpdater.emit('checking-for-update');
    expect(svc.getStatus().state).toBe('checking');

    autoUpdater.emit('update-available', { version: '9.9.9' });
    expect(svc.getStatus()).toMatchObject({ state: 'downloading', version: '9.9.9' });

    autoUpdater.emit('download-progress', { percent: 41.7 });
    expect(svc.getStatus()).toMatchObject({ state: 'downloading', percent: 42 });

    autoUpdater.emit('update-downloaded', { version: '9.9.9' });
    expect(svc.getStatus()).toMatchObject({ state: 'downloaded', version: '9.9.9' });

    svc.installNow();
    expect(autoUpdater.quitAndInstall).toHaveBeenCalledTimes(1);
    svc.stop();
  });

  it('installNow is a no-op unless an update is downloaded', async () => {
    const svc = await loadService();
    svc.start(fakeWindow());
    svc.installNow();
    expect(autoUpdater.quitAndInstall).not.toHaveBeenCalled();
    svc.stop();
  });

  it('checkNow reports unsupported in a dev build without touching the updater', async () => {
    electronApp.isPackaged = false;
    const svc = await loadService();
    const status = await svc.checkNow();
    expect(status.state).toBe('unsupported');
    expect(autoUpdater.checkForUpdates).not.toHaveBeenCalled();
  });

  it('checkNow checks even when auto-update is disabled (explicit user ask)', async () => {
    configValue = { updates: { enabled: false, channel: 'latest' } };
    const svc = await loadService();
    svc.start(fakeWindow());
    expect(svc.getStatus().state).toBe('disabled');
    await svc.checkNow();
    expect(autoUpdater.checkForUpdates).toHaveBeenCalledTimes(1);
  });
});

describe('updateService – behaviour', () => {
  it('configures background download but not install-on-quit', async () => {
    const svc = await loadService();
    svc.start(fakeWindow());
    expect(autoUpdater.autoDownload).toBe(true);
    expect(autoUpdater.autoInstallOnAppQuit).toBe(false);
    svc.stop();
  });

  it('swallows updater errors (never rejects)', async () => {
    const svc = await loadService();
    svc.start(fakeWindow());
    // Emitting an error must not throw or produce an unhandled rejection.
    expect(() => autoUpdater.emit('error', new Error('code signing required'))).not.toThrow();
    svc.stop();
  });

  it('prompts on update-downloaded and installs when the user accepts', async () => {
    showMessageBox.mockResolvedValueOnce({ response: 0 }); // "Restart now"
    const svc = await loadService();
    svc.start(fakeWindow());

    autoUpdater.emit('update-downloaded', { version: '1.2.3' });
    // Let the async dialog handler settle.
    await new Promise((r) => setTimeout(r, 0));

    expect(showMessageBox).toHaveBeenCalledTimes(1);
    expect(autoUpdater.quitAndInstall).toHaveBeenCalledTimes(1);
    svc.stop();
  });

  it('does not install when the user defers the update', async () => {
    showMessageBox.mockResolvedValueOnce({ response: 2 }); // "Later"
    const svc = await loadService();
    svc.start(fakeWindow());

    autoUpdater.emit('update-downloaded', { version: '1.2.3' });
    await new Promise((r) => setTimeout(r, 0));

    expect(showMessageBox).toHaveBeenCalledTimes(1);
    expect(autoUpdater.quitAndInstall).not.toHaveBeenCalled();
    svc.stop();
  });
});

// The one moment the user decides whether to take a build is the install
// prompt, and the running app cannot show them what is in it: changelog.
// generated.ts is baked at build time and predates the release being offered.
// So the prompt links out to the release page, which is cut from the same
// CHANGELOG.md by scripts/changelog-section.mjs.
describe('updateService – the "What\'s new" button', () => {
  it('offers reading the notes as a THIRD option, not in place of either answer', async () => {
    const svc = await loadService();
    svc.start(fakeWindow());

    autoUpdater.emit('update-downloaded', { version: '1.2.3' });
    await new Promise((r) => setTimeout(r, 0));

    const opts = showMessageBox.mock.calls[0][1] as any;
    expect(opts.buttons).toEqual(['Restart now', "What's new", 'Later']);
    // Enter still restarts and Esc still defers — reading is neither.
    expect(opts.buttons[opts.defaultId]).toBe('Restart now');
    expect(opts.buttons[opts.cancelId]).toBe('Later');
    svc.stop();
  });

  it('opens the notes and RE-ASKS, so reading is not silently a deferral', async () => {
    showMessageBox
      .mockResolvedValueOnce({ response: 1 }) // "What's new"
      .mockResolvedValueOnce({ response: 0 }); // then "Restart now"
    const svc = await loadService();
    svc.start(fakeWindow());

    autoUpdater.emit('update-downloaded', { version: '1.2.3' });
    await new Promise((r) => setTimeout(r, 0));

    expect(openExternalUrl).toHaveBeenCalledWith(
      'https://github.com/DJTouchette/workspacer/releases/tag/v1.2.3',
    );
    expect(showMessageBox).toHaveBeenCalledTimes(2); // asked again
    expect(autoUpdater.quitAndInstall).toHaveBeenCalledTimes(1);
    svc.stop();
  });

  it('reading the notes and then deferring installs nothing', async () => {
    showMessageBox
      .mockResolvedValueOnce({ response: 1 }) // "What's new"
      .mockResolvedValueOnce({ response: 2 }); // then "Later"
    const svc = await loadService();
    svc.start(fakeWindow());

    autoUpdater.emit('update-downloaded', { version: '1.2.3' });
    await new Promise((r) => setTimeout(r, 0));

    expect(showMessageBox).toHaveBeenCalledTimes(2);
    expect(autoUpdater.quitAndInstall).not.toHaveBeenCalled();
    svc.stop();
  });

  it('a browser that will not open does not wedge the prompt', async () => {
    openExternalUrl.mockResolvedValueOnce({ ok: false, error: 'no handler' });
    showMessageBox
      .mockResolvedValueOnce({ response: 1 }) // "What's new"
      .mockResolvedValueOnce({ response: 0 }); // then "Restart now"
    const svc = await loadService();
    svc.start(fakeWindow());

    autoUpdater.emit('update-downloaded', { version: '1.2.3' });
    await new Promise((r) => setTimeout(r, 0));

    expect(autoUpdater.quitAndInstall).toHaveBeenCalledTimes(1);
    svc.stop();
  });

  // The loop's only exit that is not a user click. Without it, a window that
  // dies mid-read leaves showMessageBox answering into nothing forever.
  //
  // The bail-out at 5 is the instrument, not the behaviour: the loop only ever
  // awaits already-resolved promises, so an unguarded version starves the
  // macrotask queue and this test would HANG rather than fail. Capped, the
  // mutant surfaces as a plain "called 6 times, expected 1".
  it('stops re-prompting if the window goes away mid-read', async () => {
    let alive = true;
    const win = { isDestroyed: () => !alive } as any;
    showMessageBox.mockImplementation(async () => {
      alive = false; // the window dies while the notes are open
      return { response: showMessageBox.mock.calls.length >= 5 ? 2 : 1 };
    });
    const svc = await loadService();
    svc.start(win);

    autoUpdater.emit('update-downloaded', { version: '1.2.3' });
    await new Promise((r) => setTimeout(r, 0));

    expect(showMessageBox).toHaveBeenCalledTimes(1); // did not spin
    expect(autoUpdater.quitAndInstall).not.toHaveBeenCalled();
    showMessageBox.mockReset();
    showMessageBox.mockImplementation(async () => ({ response: 2 }));
    svc.stop();
  });
});

describe('releaseNotesUrl', () => {
  it('links a stable release at its own tag', async () => {
    const { releaseNotesUrl } = await import('./updateService');
    expect(releaseNotesUrl('0.149.0')).toBe(
      'https://github.com/DJTouchette/workspacer/releases/tag/v0.149.0',
    );
  });

  // A nightly's vX.Y.Z tag is the last STABLE release — not the build being
  // offered. The rolling prerelease is the one that describes it.
  it('links a nightly at the rolling nightly prerelease, not at vX.Y.Z', async () => {
    const { releaseNotesUrl } = await import('./updateService');
    expect(releaseNotesUrl('0.149.0-nightly.20260811.abc1234')).toBe(
      'https://github.com/DJTouchette/workspacer/releases/tag/nightly',
    );
  });

  // The version comes from the update FEED, and `new URL()` collapses `..`
  // before the browser ever sees it — so an unchecked version relocates the
  // link to another repo's page, reached through our own trusted dialog.
  it('refuses to build a tag URL from anything but a bare version', async () => {
    const { releaseNotesUrl } = await import('./updateService');
    const index = 'https://github.com/DJTouchette/workspacer/releases';
    for (const hostile of [
      '../../attacker/workspacer/releases/tag/v1.0.0',
      '1.2.3/../../../attacker/repo',
      '1.2.3#not-really',
      '1.2.3?x=y',
      'https://evil.example.com',
      '',
      'latest',
      undefined,
      { version: '1.2.3' },
    ]) {
      expect(releaseNotesUrl(hostile as unknown), String(hostile)).toBe(index);
    }
  });
});
