/**
 * SECURITY.md #10: the will-attach-webview guard must force safe web prefs and
 * confine a webview's src to remote-browsing schemes. index.ts wires these pure
 * helpers into the Electron event; this suite pins the policy.
 */

import { describe, it, expect } from 'vitest';
import { applySafeWebviewPreferences, isWebviewSrcAllowed, type MutableWebPreferences } from './webviewGuard';

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

  it('fails closed on an unparseable src', () => {
    expect(isWebviewSrcAllowed('http://[::bad')).toBe(false);
  });
});
