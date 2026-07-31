/**
 * SECURITY.md #10: the will-attach-webview guard must force safe web prefs and
 * confine a webview's src to remote-browsing schemes. index.ts wires these pure
 * helpers into the Electron event; this suite pins the policy.
 */

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { EventEmitter } from 'events';
import {
  applySafeWebviewPreferences,
  installWebviewNavigationGuard,
  isWebviewSrcAllowed,
  type MutableWebPreferences,
} from './webviewGuard';

describe('applySafeWebviewPreferences', () => {
  it('strips preload and forces node integration off / context isolation on', () => {
    const prefs: MutableWebPreferences = {
      preload: '/evil/preload.js',
      preloadURL: 'file:///evil/preload.js',
      nodeIntegration: true,
      nodeIntegrationInSubFrames: true,
      contextIsolation: false,
    };
    applySafeWebviewPreferences(prefs);
    expect(prefs.preload).toBeUndefined();
    expect(prefs.preloadURL).toBeUndefined();
    expect(prefs.nodeIntegration).toBe(false);
    expect(prefs.nodeIntegrationInSubFrames).toBe(false);
    expect(prefs.contextIsolation).toBe(true);
  });

  it('sets safe defaults even when the tag requested nothing', () => {
    const prefs: MutableWebPreferences = {};
    applySafeWebviewPreferences(prefs);
    expect(prefs.nodeIntegration).toBe(false);
    expect(prefs.contextIsolation).toBe(true);
  });
});

describe('isWebviewSrcAllowed', () => {
  it('allows http/https browsing and hub/plugin origins', () => {
    for (const src of [
      'https://google.com',
      'http://127.0.0.1:7895/plugins/ui/foo/',
      'http://localhost:5173',
      'https://example.com/path?token=abc',
    ]) {
      expect(isWebviewSrcAllowed(src), src).toBe(true);
    }
  });

  it('allows about:blank and an empty src (attaches, then loadURL()s)', () => {
    expect(isWebviewSrcAllowed('about:blank')).toBe(true);
    expect(isWebviewSrcAllowed('')).toBe(true);
    expect(isWebviewSrcAllowed(undefined)).toBe(true);
  });

  it('blocks file:// and other local-resource schemes', () => {
    for (const src of [
      'file:///etc/passwd',
      'file:///home/user/.ssh/id_rsa',
      'chrome://settings',
      'devtools://devtools/bundled/inspector.html',
      'data:text/html,<script>alert(1)</script>',
    ]) {
      expect(isWebviewSrcAllowed(src), src).toBe(false);
    }
  });

  it('blocks non-blank about: URLs (only about:blank is allowed)', () => {
    for (const src of [
      'about:version',
      'about:config',
      'about:srcdoc',
      'about:blank#x',
      'about:blank?y',
    ]) {
      expect(isWebviewSrcAllowed(src), src).toBe(false);
    }
  });

  it('fails closed on an unparseable src', () => {
    expect(isWebviewSrcAllowed('http://[::bad')).toBe(false);
  });
});

/**
 * The predicate above was always right; the WIRING wasn't. BrowserPane navigates
 * by calling webview.loadURL() from the renderer, and `will-navigate` doesn't
 * fire for embedder-initiated navigations — so the only guard that ran was the
 * attach-time src check, and typing `file:///etc/passwd` into the browser bar
 * loaded it. These cases pin the path that actually fires.
 */
describe('installWebviewNavigationGuard', () => {
  function fakeGuest() {
    const guest = new EventEmitter() as EventEmitter & {
      stop: ReturnType<typeof vi.fn>;
      loadURL: ReturnType<typeof vi.fn>;
    };
    guest.stop = vi.fn();
    guest.loadURL = vi.fn();
    return guest;
  }

  /** What Electron emits for a loadURL() — no will-navigate, just this. */
  function loadUrl(guest: EventEmitter, url: string, isMainFrame = true) {
    guest.emit('did-start-navigation', { url, isMainFrame, isSameDocument: false });
  }

  beforeEach(() => {
    vi.spyOn(console, 'warn').mockImplementation(() => {});
  });

  it('stops a file:// loadURL — the navigation will-navigate never sees', () => {
    const guest = fakeGuest();
    installWebviewNavigationGuard(guest);

    loadUrl(guest, 'file:///etc/passwd');

    expect(guest.stop).toHaveBeenCalledTimes(1);
    expect(guest.loadURL).toHaveBeenCalledWith('about:blank');
  });

  it('stops other local-resource schemes reached the same way', () => {
    for (const url of [
      'chrome://settings',
      'devtools://devtools/bundled/inspector.html',
      'data:text/html,<script>alert(1)</script>',
      'about:config',
    ]) {
      const guest = fakeGuest();
      installWebviewNavigationGuard(guest);
      loadUrl(guest, url);
      expect(guest.stop, url).toHaveBeenCalledTimes(1);
    }
  });

  it('leaves ordinary browsing alone', () => {
    const guest = fakeGuest();
    installWebviewNavigationGuard(guest);

    loadUrl(guest, 'https://example.com/page');
    loadUrl(guest, 'http://127.0.0.1:7895/plugins/ui/foo/');
    loadUrl(guest, 'about:blank');

    expect(guest.stop).not.toHaveBeenCalled();
    expect(guest.loadURL).not.toHaveBeenCalled();
  });

  it('ignores sub-frame navigations (only the top document is confined)', () => {
    const guest = fakeGuest();
    installWebviewNavigationGuard(guest);

    loadUrl(guest, 'data:text/html,x', false);

    expect(guest.stop).not.toHaveBeenCalled();
  });

  it('still cancels the in-page navigations will-navigate does report', () => {
    const guest = fakeGuest();
    installWebviewNavigationGuard(guest);

    const preventDefault = vi.fn();
    guest.emit('will-navigate', { preventDefault }, 'file:///home/user/.ssh/id_rsa');
    expect(preventDefault).toHaveBeenCalledTimes(1);

    preventDefault.mockClear();
    guest.emit('will-redirect', { preventDefault }, 'file:///etc/shadow');
    expect(preventDefault).toHaveBeenCalledTimes(1);

    preventDefault.mockClear();
    guest.emit('will-navigate', { preventDefault }, 'https://example.com');
    expect(preventDefault).not.toHaveBeenCalled();
  });
});
