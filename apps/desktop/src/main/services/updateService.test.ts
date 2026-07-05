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
const electronApp = { isPackaged: true };
const showMessageBox = vi.fn(async () => ({ response: 1 }));
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

  it('applies the configured channel to the updater', async () => {
    configValue = { updates: { enabled: true, channel: 'beta' } };
    const svc = await loadService();
    svc.start(fakeWindow());
    expect(autoUpdater.channel).toBe('beta');
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
    showMessageBox.mockResolvedValueOnce({ response: 1 }); // "Later"
    const svc = await loadService();
    svc.start(fakeWindow());

    autoUpdater.emit('update-downloaded', { version: '1.2.3' });
    await new Promise((r) => setTimeout(r, 0));

    expect(showMessageBox).toHaveBeenCalledTimes(1);
    expect(autoUpdater.quitAndInstall).not.toHaveBeenCalled();
    svc.stop();
  });
});
