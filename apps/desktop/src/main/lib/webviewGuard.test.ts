/**
 * The will-attach-webview guard must force safe web prefs and confine a
 * webview's src to remote-browsing schemes PLUS a bounded set of local files.
 * index.ts wires these pure helpers into the Electron events; this suite pins
 * the policy.
 *
 * The file: allowance is the part worth being paranoid about, so most of what
 * follows is escape attempts against a real temp-directory root rather than a
 * fake fs, because a policy about symlinks and `..` that is only tested against a
 * stub is a policy about the stub.
 */

import { describe, it, expect, vi, beforeEach, beforeAll, afterAll } from 'vitest';
import { EventEmitter } from 'events';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import { pathToFileURL } from 'url';
import {
  applySafeWebviewPreferences,
  checkPreviewFileUrl,
  checkWebviewSrc,
  installWebviewGuards,
  installWebviewNavigationGuard,
  isWebviewSrcAllowed,
  BROWSER_FILE_EXTENSIONS,
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

  /**
   * Electron 43.4.1 spreads a `<webview webpreferences="...">` attribute onto
   * this object AFTER the built-in inheritance clamp, and that clamp covers
   * contextIsolation/sandbox/nodeIntegration(*)/javascript/enableWebSQL but not
   * webSecurity or allowFileAccessFromFileUrls. A guest page that can set its
   * OWN `<webview webpreferences="allowFileAccessFromFileUrls">` therefore gets
   * whole-filesystem XHR read from a file: origin. Pinned here so this function
   * closes it regardless of what the tag or the clamp left in `prefs`.
   */
  it('pins webSecurity and allowFileAccessFromFileUrls, which the inheritance clamp does not cover', () => {
    const prefs: MutableWebPreferences = {
      webSecurity: false,
      allowFileAccessFromFileUrls: true,
    };
    applySafeWebviewPreferences(prefs);
    expect(prefs.webSecurity).toBe(true);
    expect(prefs.allowFileAccessFromFileUrls).toBe(false);
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

  it('blocks non-file local-resource schemes outright, roots or no roots', () => {
    for (const src of [
      'chrome://settings',
      'devtools://devtools/bundled/inspector.html',
      'data:text/html,<script>alert(1)</script>',
    ]) {
      expect(isWebviewSrcAllowed(src), src).toBe(false);
      expect(isWebviewSrcAllowed(src, ['/']), src).toBe(false);
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

  it('refuses every file: URL when no roots are supplied', () => {
    // The default is the OLD policy, so a caller that forgets the roots argument
    // gets a closed door rather than a wildcard.
    for (const src of ['file:///etc/passwd', 'file:///home/user/.ssh/id_rsa']) {
      expect(isWebviewSrcAllowed(src), src).toBe(false);
    }
  });
});

// ─── The bounded file: allowance ─────────────────────────────────────────────

/** A real directory tree, because the escapes under test are fs behaviours. */
let tmp: string;
let root: string;
let outside: string;
let roots: string[];

/** `file://` URL for an absolute path, encoding each segment like browserBus does. */
function fileUrl(p: string): string {
  return 'file://' + p.split('/').map(encodeURIComponent).join('/');
}

beforeAll(() => {
  tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'wks-webviewguard-'));
  root = path.join(tmp, 'user');
  outside = path.join(tmp, 'outside');
  fs.mkdirSync(path.join(root, 'design'), { recursive: true });
  fs.mkdirSync(outside, { recursive: true });
  // A sibling whose name has the root as a string prefix: the classic
  // /home/a admitting /home/ab bug.
  fs.mkdirSync(path.join(tmp, 'user2'), { recursive: true });

  fs.writeFileSync(path.join(root, 'design', 'index.html'), '<h1>ok</h1>');
  fs.writeFileSync(path.join(root, 'design', 'DESIGN.md'), '# nope');
  fs.writeFileSync(path.join(root, 'design', 'style.css'), 'body{}');
  fs.writeFileSync(path.join(root, 'deploy.sh'), '#!/bin/sh');
  fs.writeFileSync(path.join(root, '.env'), 'SECRET=1');
  fs.writeFileSync(path.join(root, 'key.pem'), 'PRIVATE');
  fs.writeFileSync(path.join(root, 'LICENSE'), 'MIT'); // no extension at all
  fs.writeFileSync(path.join(outside, 'evil.html'), '<h1>escaped</h1>');
  fs.writeFileSync(path.join(tmp, 'user2', 'a.html'), '<h1>neighbour</h1>');
  // A symlink that LIVES inside the root but POINTS outside it. A textual
  // `..`-collapse would see <root>/escape.html and allow it.
  fs.symlinkSync(path.join(outside, 'evil.html'), path.join(root, 'escape.html'));

  roots = [root];
});

afterAll(() => {
  fs.rmSync(tmp, { recursive: true, force: true });
});

describe('checkWebviewSrc: the file: allowance', () => {
  it('allows a renderable file inside a root', () => {
    const v = checkWebviewSrc(fileUrl(path.join(root, 'design', 'index.html')), roots);
    expect(v.allowed).toBe(true);
    expect(v.reason).toBeUndefined();
  });

  it('allows the same file through the localhost spelling of file://', () => {
    const p = path.join(root, 'design', 'index.html');
    const url = 'file://localhost' + p.split('/').map(encodeURIComponent).join('/');
    expect(isWebviewSrcAllowed(url, roots)).toBe(true);
  });

  it('allows every extension on the allowlist and nothing else', () => {
    for (const ext of BROWSER_FILE_EXTENSIONS) {
      const p = path.join(root, `probe.${ext}`);
      fs.writeFileSync(p, 'x');
      expect(isWebviewSrcAllowed(fileUrl(p), roots), ext).toBe(true);
      fs.rmSync(p);
    }
  });

  it('survives a space in the filename (encoded by fileUrlFromPath)', () => {
    const p = path.join(root, 'a report.html');
    fs.writeFileSync(p, 'x');
    expect(isWebviewSrcAllowed(fileUrl(p), roots)).toBe(true);
    fs.rmSync(p);
  });

  // ── Escape attempts. Each of these must be a denial. ──

  it("denies '..' traversal out of the root", () => {
    const v = checkWebviewSrc(fileUrl(root) + '/../outside/evil.html', roots);
    expect(v.allowed).toBe(false);
    expect(v.reason).toMatch(/outside/);
  });

  it('denies percent-encoded traversal (%2e%2e%2f)', () => {
    const v = checkWebviewSrc(fileUrl(root) + '/%2e%2e/outside/evil.html', roots);
    expect(v.allowed).toBe(false);
  });

  it('denies DOUBLE-encoded traversal without decoding it twice', () => {
    // One decode yields the literal component `%2e%2e`, a filename that does not
    // exist. It must NOT become a `..` on a second pass.
    const v = checkWebviewSrc(fileUrl(root) + '/%252e%252e/outside/evil.html', roots);
    expect(v.allowed).toBe(false);
    expect(v.reason).toMatch(/no such file/);
    expect(fs.existsSync(path.join(root, '%2e%2e'))).toBe(false);
  });

  it('denies a symlink that lives inside the root and points outside it', () => {
    const v = checkWebviewSrc(fileUrl(path.join(root, 'escape.html')), roots);
    expect(v.allowed).toBe(false);
    expect(v.reason).toMatch(/outside/);
  });

  it('denies a file: URL with a remote host', () => {
    const v = checkWebviewSrc('file://evil/share/payload.html', roots);
    expect(v.allowed).toBe(false);
    expect(v.reason).toMatch(/remote host/);
    // …and with a root that would otherwise contain the path.
    expect(
      isWebviewSrcAllowed('file://evil' + fileUrl(root).slice(7) + '/design/index.html', roots),
    ).toBe(false);
  });

  it('denies a prefix-collision sibling (/…/user does not admit /…/user2)', () => {
    const v = checkWebviewSrc(fileUrl(path.join(tmp, 'user2', 'a.html')), roots);
    expect(v.allowed).toBe(false);
    expect(v.reason).toMatch(/outside/);
  });

  it('denies a directory', () => {
    const v = checkWebviewSrc(fileUrl(path.join(root, 'design')), roots);
    expect(v.allowed).toBe(false);
    // A directory has no allowed extension either; whichever bar it hits first,
    // it must not be treated as a page.
    expect(v.reason).toBeTruthy();
  });

  it('denies a directory that happens to be named like a page', () => {
    const dir = path.join(root, 'site.html');
    fs.mkdirSync(dir);
    const v = checkWebviewSrc(fileUrl(dir), roots);
    expect(v.allowed).toBe(false);
    expect(v.reason).toMatch(/directory/);
    fs.rmdirSync(dir);
  });

  it('denies markdown, and says where it does open', () => {
    const v = checkWebviewSrc(fileUrl(path.join(root, 'design', 'DESIGN.md')), roots);
    expect(v.allowed).toBe(false);
    expect(v.reason).toMatch(/preview pane/);
  });

  it('denies disallowed extensions (.sh, .env, .pem) and extensionless files', () => {
    for (const name of ['deploy.sh', '.env', 'key.pem', 'LICENSE']) {
      const v = checkWebviewSrc(fileUrl(path.join(root, name)), roots);
      expect(v.allowed, name).toBe(false);
      expect(v.reason, name).toMatch(/does not render/);
    }
  });

  it('denies a path under a root that does not exist, and does not create it', () => {
    const missing = path.join(root, 'design', 'nope.html');
    const v = checkWebviewSrc(fileUrl(missing), roots);
    expect(v.allowed).toBe(false);
    expect(v.reason).toMatch(/no such file/);
    expect(fs.existsSync(missing)).toBe(false);
  });

  it('denies an empty root list even for a file that exists', () => {
    expect(isWebviewSrcAllowed(fileUrl(path.join(root, 'design', 'index.html')), [])).toBe(false);
    // An empty STRING root is not a wildcard either.
    expect(isWebviewSrcAllowed(fileUrl(path.join(root, 'design', 'index.html')), [''])).toBe(false);
  });

  it('honours a second root independently', () => {
    const p = path.join(outside, 'evil.html');
    expect(isWebviewSrcAllowed(fileUrl(p), roots)).toBe(false);
    expect(isWebviewSrcAllowed(fileUrl(p), [root, outside])).toBe(true);
  });
});

/**
 * "Inside a root" was never the same question as "safe to render". One of the
 * roots is the whole home directory, so the module header's claim that the pane
 * "cannot become a credential viewer" rested entirely on the extension list, and
 * json was on it. These pin BOTH halves of the repair: the credential gate that
 * runs before the extension gate, and an allowlist that no longer admits json.
 */
describe('checkWebviewSrc: credentials and agent configuration', () => {
  /** Files that live under a root and are exactly what the roots do not vet. */
  const SECRET_UNDER_ROOT = [
    // Denied by pathConfinement's shared list, regardless of extension.
    ['.claude/settings.json', /credentials or agent configuration/],
    ['.claude.json', /credentials or agent configuration/],
    // Denied by the extension gate now that json is off the allowlist. They are
    // NOT on the shared denial list, which is a TWIN of two Go copies pinned by
    // contracts/path-containment-cases.json, so this file is not the place to
    // add names to it. Both halves are needed; neither covers all four.
    ['.claude/.credentials.json', /does not render/],
    ['.docker/config.json', /does not render/],
  ] as const;

  beforeAll(() => {
    for (const [rel] of SECRET_UNDER_ROOT) {
      const p = path.join(root, rel);
      fs.mkdirSync(path.dirname(p), { recursive: true });
      fs.writeFileSync(p, '{"token":"hunter2"}');
    }
    // Extension-allowed files that the DENIAL list must refuse anyway. Each is a
    // place a program's behaviour is defined, not project data.
    for (const rel of ['.git/hooks/pre-commit.html', '.codex/x.html', '.claude/hooks/x.html']) {
      const p = path.join(root, rel);
      fs.mkdirSync(path.dirname(p), { recursive: true });
      fs.writeFileSync(p, '<h1>x</h1>');
    }
    fs.writeFileSync(path.join(root, 'package.json'), '{}');
    fs.writeFileSync(path.join(root, 'report.pdf'), '%PDF-1.4');
    fs.writeFileSync(path.join(root, 'README.md'), '# x');
  });

  it('refuses every one of the four json files an over-wide root exposes', () => {
    for (const [rel, reason] of SECRET_UNDER_ROOT) {
      const v = checkWebviewSrc(fileUrl(path.join(root, rel)), roots);
      expect(v.allowed, rel).toBe(false);
      expect(v.reason, rel).toMatch(reason);
    }
  });

  it('refuses a plain .json under a root, because json renders nothing anyone asked for', () => {
    const v = checkWebviewSrc(fileUrl(path.join(root, 'package.json')), roots);
    expect(v.allowed).toBe(false);
    expect(v.reason).toMatch(/does not render/);
  });

  it('refuses a .pdf: the internal viewer is a plugin, and plugins are off', () => {
    const v = checkWebviewSrc(fileUrl(path.join(root, 'report.pdf')), roots);
    expect(v.allowed).toBe(false);
    expect(v.reason).toMatch(/does not render/);
  });

  it('refuses the denial list REGARDLESS of extension, .html included', () => {
    for (const rel of ['.git/hooks/pre-commit.html', '.codex/x.html', '.claude/hooks/x.html']) {
      const v = checkWebviewSrc(fileUrl(path.join(root, rel)), roots);
      expect(v.allowed, rel).toBe(false);
      expect(v.reason, rel).toMatch(/credentials or agent configuration/);
    }
  });

  it('applies the same gate to the PREVIEW door, so .md is no way around it', () => {
    const p = path.join(root, '.codex', 'notes.md');
    fs.writeFileSync(p, '# secrets');
    const v = checkPreviewFileUrl(fileUrl(p), roots);
    expect(v.allowed).toBe(false);
    expect(v.reason).toMatch(/credentials or agent configuration/);
  });

  it('still allows an ordinary page beside all of that', () => {
    expect(isWebviewSrcAllowed(fileUrl(path.join(root, 'design', 'index.html')), roots)).toBe(true);
  });
});

/**
 * The markdown detour is the OTHER half of the same allowance, and it used to be
 * unconfined end to end: the renderer only asked whether the URL ended in `.md`,
 * and the read behind the preview pane applies no confinement of its own. So
 * `open_browser` on `file:///etc/ssl/README.md` rendered an out-of-root file,
 * and renaming anything to `.md` sidestepped the browser arm entirely.
 */
describe('checkPreviewFileUrl: the markdown detour is confined too', () => {
  it('allows an in-root markdown file', () => {
    const v = checkPreviewFileUrl(fileUrl(path.join(root, 'design', 'DESIGN.md')), roots);
    expect(v.allowed).toBe(true);
  });

  it('refuses an out-of-root markdown file, before any read', () => {
    fs.writeFileSync(path.join(outside, 'README.md'), '# escaped');
    const v = checkPreviewFileUrl(fileUrl(path.join(outside, 'README.md')), roots);
    expect(v.allowed).toBe(false);
    expect(v.reason).toMatch(/outside/);
  });

  it('refuses a %2e%2e traversal out of the root', () => {
    const v = checkPreviewFileUrl(fileUrl(root) + '/%2e%2e/outside/README.md', roots);
    expect(v.allowed).toBe(false);
    expect(v.reason).toMatch(/outside/);
  });

  it('refuses a NON-markdown file, so it cannot become a second browser door', () => {
    const v = checkPreviewFileUrl(fileUrl(path.join(root, 'design', 'index.html')), roots);
    expect(v.allowed).toBe(false);
    expect(v.reason).toMatch(/only opens markdown/);
  });

  it('refuses anything that is not a local file URL', () => {
    for (const u of ['https://example.com/README.md', 'about:blank', 'not a url']) {
      expect(checkPreviewFileUrl(u, roots).allowed, u).toBe(false);
    }
  });

  it('refuses a file URL with a remote host, and an empty root list', () => {
    expect(checkPreviewFileUrl('file://evil/share/README.md', roots).allowed).toBe(false);
    expect(checkPreviewFileUrl(fileUrl(path.join(root, 'design', 'DESIGN.md')), []).allowed).toBe(
      false,
    );
  });

  it('names the preview target on the browser door ONLY when it is in a root', () => {
    const inRoot = checkWebviewSrc(fileUrl(path.join(root, 'design', 'DESIGN.md')), roots);
    expect(inRoot.allowed).toBe(false);
    expect(inRoot.previewPath).toBe(path.join(root, 'design', 'DESIGN.md'));

    // Out of root: refused for WHERE it is, so there is nothing to offer. A
    // previewPath here would be a button that walks around the refusal.
    const outOfRoot = checkWebviewSrc(fileUrl(path.join(outside, 'README.md')), roots);
    expect(outOfRoot.allowed).toBe(false);
    expect(outOfRoot.previewPath).toBeUndefined();
  });
});

// ─── The navigation door ─────────────────────────────────────────────────────

/**
 * The predicate above was always right; the WIRING wasn't. BrowserPane navigates
 * by calling webview.loadURL() from the renderer, and `will-navigate` doesn't
 * fire for embedder-initiated navigations — so the only guard that ran was the
 * attach-time src check, and typing `file:///etc/passwd` into the browser bar
 * loaded it. These cases pin the path that actually fires.
 */
describe('installWebviewNavigationGuard', () => {
  type Fake = EventEmitter & {
    stop: ReturnType<typeof vi.fn>;
    loadURL: ReturnType<typeof vi.fn>;
    getURL: () => string;
    setWindowOpenHandler: ReturnType<typeof vi.fn>;
    openHandler?: (d: { url: string }) => { action: 'allow' | 'deny' };
  };

  function fakeGuest(currentUrl = ''): Fake {
    const guest = new EventEmitter() as Fake;
    guest.stop = vi.fn();
    guest.loadURL = vi.fn();
    guest.getURL = () => currentUrl;
    guest.setWindowOpenHandler = vi.fn((h) => {
      guest.openHandler = h;
    });
    return guest;
  }

  /** What Electron emits for a loadURL() — no will-navigate, just this. */
  function loadUrl(guest: EventEmitter, url: string, isMainFrame = true) {
    guest.emit('did-start-navigation', { url, isMainFrame, isSameDocument: false });
  }

  beforeEach(() => {
    vi.spyOn(console, 'warn').mockImplementation(() => {});
  });

  it('stops a file:// loadURL with no roots, the navigation will-navigate never sees', () => {
    const guest = fakeGuest();
    installWebviewNavigationGuard(guest, { allowedRoots: () => [] });

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
      installWebviewNavigationGuard(guest, { allowedRoots: () => roots });
      loadUrl(guest, url);
      expect(guest.stop, url).toHaveBeenCalledTimes(1);
    }
  });

  it('leaves ordinary browsing alone', () => {
    const guest = fakeGuest();
    installWebviewNavigationGuard(guest, { allowedRoots: () => roots });

    loadUrl(guest, 'https://example.com/page');
    loadUrl(guest, 'http://127.0.0.1:7895/plugins/ui/foo/');
    loadUrl(guest, 'about:blank');

    expect(guest.stop).not.toHaveBeenCalled();
    expect(guest.loadURL).not.toHaveBeenCalled();
  });

  it('ignores sub-frame navigations (only the top document is confined)', () => {
    const guest = fakeGuest();
    installWebviewNavigationGuard(guest, { allowedRoots: () => roots });

    loadUrl(guest, 'data:text/html,x', false);

    expect(guest.stop).not.toHaveBeenCalled();
  });

  it('still cancels the in-page navigations will-navigate does report', () => {
    const guest = fakeGuest();
    installWebviewNavigationGuard(guest, { allowedRoots: () => roots });

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

  // ── The origin rule that only matters once file: is allowed at all ──

  const inside = () => fileUrl(path.join(root, 'design', 'index.html'));
  const alsoInside = () => fileUrl(path.join(root, 'design', 'style.css'));

  it('blocks an http(s) page from navigating the pane to a local file', () => {
    const guest = fakeGuest('https://evil.example.com/');
    const blocked: string[] = [];
    installWebviewNavigationGuard(guest, {
      allowedRoots: () => roots,
      onBlocked: (i) => blocked.push(i.reason),
    });

    loadUrl(guest, inside()); // an ALLOWED file, refused because of who asked

    expect(guest.stop).toHaveBeenCalledTimes(1);
    expect(blocked[0]).toMatch(/web page may not open a local file/);
  });

  it('blocks the same thing through will-navigate (a link click on a web page)', () => {
    const guest = fakeGuest('http://example.com/');
    installWebviewNavigationGuard(guest, { allowedRoots: () => roots });
    const preventDefault = vi.fn();
    guest.emit('will-navigate', { preventDefault }, inside());
    expect(preventDefault).toHaveBeenCalledTimes(1);
  });

  it('allows file → file when the target is also inside a root', () => {
    const guest = fakeGuest(inside());
    installWebviewNavigationGuard(guest, { allowedRoots: () => roots });

    loadUrl(guest, alsoInside());

    expect(guest.stop).not.toHaveBeenCalled();
  });

  it('blocks file → file when the target is outside the roots', () => {
    const guest = fakeGuest(inside());
    installWebviewNavigationGuard(guest, { allowedRoots: () => roots });

    loadUrl(guest, fileUrl(path.join(outside, 'evil.html')));

    expect(guest.stop).toHaveBeenCalledTimes(1);
  });

  it('allows file → http(s), as before', () => {
    const guest = fakeGuest(inside());
    installWebviewNavigationGuard(guest, { allowedRoots: () => roots });

    loadUrl(guest, 'https://example.com/docs');

    expect(guest.stop).not.toHaveBeenCalled();
  });

  it('tracks the page it navigated to when the guest cannot report it', () => {
    const guest = fakeGuest();
    guest.getURL = () => ''; // an embedder-driven guest mid-flight
    installWebviewNavigationGuard(guest, { allowedRoots: () => roots });

    loadUrl(guest, 'https://example.com/'); // now ON the web
    loadUrl(guest, inside()); // …so this local file must be refused

    expect(guest.stop).toHaveBeenCalledTimes(1);
  });

  // ── window.open (allowpopups is on) ──

  it('denies a window.open of a disallowed file and allows an ordinary one', () => {
    const guest = fakeGuest(inside());
    const blocked: string[] = [];
    installWebviewNavigationGuard(guest, {
      allowedRoots: () => roots,
      onBlocked: (i) => blocked.push(i.url),
    });
    expect(guest.setWindowOpenHandler).toHaveBeenCalledTimes(1);

    expect(guest.openHandler!({ url: 'file:///etc/passwd' })).toEqual({ action: 'deny' });
    expect(guest.openHandler!({ url: 'https://example.com' })).toEqual({ action: 'allow' });
    expect(guest.openHandler!({ url: inside() })).toEqual({ action: 'allow' });
    expect(blocked).toEqual(['file:///etc/passwd']);
  });

  it('denies a window.open of a local file from a web page', () => {
    const guest = fakeGuest('https://evil.example.com/');
    installWebviewNavigationGuard(guest, { allowedRoots: () => roots });
    expect(guest.openHandler!({ url: inside() })).toEqual({ action: 'deny' });
  });

  // ── The source of a file: navigation is an ALLOW-list, not a deny-list ──
  //
  // Keying the rule on "is the current page http(s)" left two sources that are
  // not remote and are not allowed local pages either: `about:blank`, which
  // inherits its initiator's origin, and a guest with no committed document at
  // all. Both were proven ALLOW against the compiled guard before this.

  it('blocks about:blank from navigating to a local file, on both doors', () => {
    const guest = fakeGuest('about:blank');
    const blocked: string[] = [];
    installWebviewNavigationGuard(guest, {
      allowedRoots: () => roots,
      onBlocked: (i) => blocked.push(i.reason),
    });

    loadUrl(guest, inside());
    expect(guest.stop).toHaveBeenCalledTimes(1);
    expect(blocked[0]).toMatch(/may not open a local file/);

    expect(guest.openHandler!({ url: inside() })).toEqual({ action: 'deny' });
  });

  it('blocks a guest whose getURL() is empty from reaching a local file', () => {
    const guest = fakeGuest(''); // attached with no src, nothing committed yet
    installWebviewNavigationGuard(guest, { allowedRoots: () => roots });

    loadUrl(guest, inside());
    expect(guest.stop).toHaveBeenCalledTimes(1);
    expect(guest.openHandler!({ url: inside() })).toEqual({ action: 'deny' });
  });

  it('lets the attach load itself through, once, and only for that src', () => {
    const guest = fakeGuest(''); // the initial load has no committed page
    installWebviewNavigationGuard(guest, {
      allowedRoots: () => roots,
      approvedAttachSrcs: [inside()],
    });

    loadUrl(guest, inside());
    expect(guest.stop).not.toHaveBeenCalled();

    // Consumed. The guest is now ON that local page, so a second load of it is
    // allowed by the SOURCE rule rather than by the exemption; browsing away to
    // the web is what shows the exemption did not survive.
    loadUrl(guest, 'https://example.com/');
    loadUrl(guest, inside());
    expect(guest.stop).toHaveBeenCalledTimes(1);
  });

  it('does not extend the attach exemption to a DIFFERENT local file', () => {
    const guest = fakeGuest('');
    installWebviewNavigationGuard(guest, {
      allowedRoots: () => roots,
      approvedAttachSrcs: [inside()],
    });

    loadUrl(guest, alsoInside());
    expect(guest.stop).toHaveBeenCalledTimes(1);
  });

  it('matches the attach src through the spellings Chromium normalises away', () => {
    for (const spelling of [
      fileUrl(path.join(root, 'design')) + '/../design/index.html',
      fileUrl(path.join(root, 'design')) + '/%2e%2e/design/index.html',
      inside().replace(/^file:/, 'FILE:'),
    ]) {
      const guest = fakeGuest('');
      installWebviewNavigationGuard(guest, {
        allowedRoots: () => roots,
        approvedAttachSrcs: [spelling],
      });
      loadUrl(guest, inside()); // what Chromium actually reports
      expect(guest.stop, spelling).not.toHaveBeenCalled();
    }
  });

  it('names the guest a navigation refusal is about, so a pane can claim it', () => {
    const guest = fakeGuest('https://evil.example.com/');
    (guest as unknown as { id: number }).id = 77;
    const seen: Array<{ webContentsId?: number }> = [];
    installWebviewNavigationGuard(guest, {
      allowedRoots: () => roots,
      onBlocked: (i) => seen.push(i),
    });

    loadUrl(guest, inside());

    expect(seen).toHaveLength(1);
    expect(seen[0].webContentsId).toBe(77);
  });

  it('omits the id when the guest has none rather than sending a fake one', () => {
    const guest = fakeGuest('https://evil.example.com/');
    const seen: Array<{ webContentsId?: number }> = [];
    installWebviewNavigationGuard(guest, {
      allowedRoots: () => roots,
      onBlocked: (i) => seen.push(i),
    });

    loadUrl(guest, inside());

    expect(seen[0].webContentsId).toBeUndefined();
  });

  // ── The popups this pane exists to carry ──
  //
  // index.ts records that intercepting window.open via setWindowOpenHandler was
  // once reverted for "aborting unrelated navigations", in the same commit as
  // the Google/Microsoft sign-in user-agent spoofing. So this door judges file:
  // and nothing else, and the popup it admits is guarded instead.

  it('does not judge a non-file window.open at all, whatever the page is', () => {
    for (const from of ['https://accounts.google.com/', 'about:blank', '']) {
      const guest = fakeGuest(from);
      const blocked: string[] = [];
      installWebviewNavigationGuard(guest, {
        allowedRoots: () => roots,
        onBlocked: (i) => blocked.push(i.url),
      });
      for (const url of [
        'https://accounts.google.com/o/oauth2/auth?x=1',
        'https://login.microsoftonline.com/common/oauth2/authorize',
        'chrome://settings',
      ]) {
        expect(guest.openHandler!({ url }), `${from} -> ${url}`).toEqual({ action: 'allow' });
      }
      expect(blocked, from).toEqual([]);
    }
  });

  it('guards the popup it admits, so a non-file scheme is refused on its FIRST load', () => {
    const guest = fakeGuest('https://accounts.google.com/');
    installWebviewNavigationGuard(guest, { allowedRoots: () => roots });

    expect(guest.openHandler!({ url: 'chrome://settings' })).toEqual({ action: 'allow' });

    const popup = fakeGuest('');
    guest.emit('did-create-window', { webContents: popup });
    loadUrl(popup, 'chrome://settings');

    expect(popup.stop).toHaveBeenCalledTimes(1);
    expect(popup.loadURL).toHaveBeenCalledWith('about:blank');
  });

  it('guards the popup against reaching a local file, with no attach exemption', () => {
    const guest = fakeGuest(inside()); // an ALLOWED local page opens the popup
    installWebviewNavigationGuard(guest, { allowedRoots: () => roots });

    const popup = fakeGuest('');
    guest.emit('did-create-window', { webContents: popup });

    // The popup has committed nothing and nothing approved a src for it, so the
    // file: source rule refuses it even though the target is inside a root.
    loadUrl(popup, inside());
    expect(popup.stop).toHaveBeenCalledTimes(1);
  });

  it('lets an SSO popup browse, which is the whole point of admitting it', () => {
    const guest = fakeGuest('https://mail.google.com/');
    installWebviewNavigationGuard(guest, { allowedRoots: () => roots });

    const popup = fakeGuest('');
    guest.emit('did-create-window', { webContents: popup });
    loadUrl(popup, 'https://accounts.google.com/o/oauth2/auth');
    loadUrl(popup, 'https://accounts.google.com/signin/oauth/consent');

    expect(popup.stop).not.toHaveBeenCalled();
  });

  it('survives a did-create-window with no webContents', () => {
    const guest = fakeGuest('https://example.com/');
    installWebviewNavigationGuard(guest, { allowedRoots: () => roots });
    expect(() => guest.emit('did-create-window', undefined)).not.toThrow();
    expect(() => guest.emit('did-create-window', {})).not.toThrow();
  });

  it('survives a guest with no setWindowOpenHandler / getURL', () => {
    const bare = new EventEmitter() as EventEmitter & { stop: () => void; loadURL: () => void };
    bare.stop = vi.fn();
    bare.loadURL = vi.fn();
    expect(() => installWebviewNavigationGuard(bare, { allowedRoots: () => roots })).not.toThrow();
  });
});

// ─── Both doors, one policy ──────────────────────────────────────────────────

describe('installWebviewGuards', () => {
  function fakeHost() {
    const host = new EventEmitter();
    return host;
  }

  function attach(host: EventEmitter, src: string) {
    const event = { preventDefault: vi.fn() };
    const prefs: MutableWebPreferences = { nodeIntegration: true, preload: '/x.js' };
    host.emit('will-attach-webview', event, prefs, { src });
    return { blocked: event.preventDefault.mock.calls.length > 0, prefs };
  }

  beforeEach(() => {
    vi.spyOn(console, 'warn').mockImplementation(() => {});
  });

  it('forces safe prefs on every attach', () => {
    const host = fakeHost();
    installWebviewGuards(host, { allowedRoots: () => roots });
    const { prefs } = attach(host, 'https://example.com');
    expect(prefs.preload).toBeUndefined();
    expect(prefs.nodeIntegration).toBe(false);
    expect(prefs.contextIsolation).toBe(true);
  });

  it('overrides a webpreferences attribute that asks for webSecurity off and file XHR on', () => {
    const host = fakeHost();
    installWebviewGuards(host, { allowedRoots: () => roots });
    const event = { preventDefault: vi.fn() };
    const prefs: MutableWebPreferences = {
      webSecurity: false,
      allowFileAccessFromFileUrls: true,
    };
    host.emit('will-attach-webview', event, prefs, { src: 'https://example.com' });
    expect(prefs.webSecurity).toBe(true);
    expect(prefs.allowFileAccessFromFileUrls).toBe(false);
  });

  it('reports a refused attach with a reason and the phase', () => {
    const host = fakeHost();
    const seen: Array<{ url: string; reason: string; phase: string }> = [];
    installWebviewGuards(host, { allowedRoots: () => roots, onBlocked: (i) => seen.push(i) });

    const { blocked } = attach(host, 'file:///etc/passwd');

    expect(blocked).toBe(true);
    expect(seen).toHaveLength(1);
    expect(seen[0].url).toBe('file:///etc/passwd');
    expect(seen[0].phase).toBe('attach');
    expect(seen[0].reason).toBeTruthy();
  });

  /**
   * The pane's whole reason for existing: `<webview src="file:///.../x.html">`.
   * The attach door approves it, and the navigation door has to let the load
   * that attach started through even though nothing is committed yet. Wiring the
   * two through one call is what makes that possible without an exemption a page
   * could ask for itself.
   */
  it('lets an APPROVED file: attach load through the navigation door too', () => {
    const host = fakeHost();
    installWebviewGuards(host, { allowedRoots: () => roots });
    const url = fileUrl(path.join(root, 'design', 'index.html'));

    expect(attach(host, url).blocked).toBe(false);

    const guest = new EventEmitter() as EventEmitter & {
      stop: ReturnType<typeof vi.fn>;
      loadURL: ReturnType<typeof vi.fn>;
      getURL: () => string;
    };
    guest.stop = vi.fn();
    guest.loadURL = vi.fn();
    guest.getURL = () => ''; // nothing committed: this IS the attach load
    host.emit('did-attach-webview', {}, guest);
    guest.emit('did-start-navigation', { url, isMainFrame: true });

    expect(guest.stop).not.toHaveBeenCalled();
  });

  it('leaves no attach exemption behind when the attach was REFUSED', () => {
    const host = fakeHost();
    installWebviewGuards(host, { allowedRoots: () => roots });
    const allowed = fileUrl(path.join(root, 'design', 'index.html'));

    // A refused attach must not let the NEXT guest replay the previously
    // approved src as its own initial load.
    expect(attach(host, allowed).blocked).toBe(false);
    expect(attach(host, fileUrl(path.join(outside, 'evil.html'))).blocked).toBe(true);

    const guest = new EventEmitter() as EventEmitter & {
      stop: ReturnType<typeof vi.fn>;
      loadURL: ReturnType<typeof vi.fn>;
      getURL: () => string;
    };
    guest.stop = vi.fn();
    guest.loadURL = vi.fn();
    guest.getURL = () => '';
    host.emit('did-attach-webview', {}, guest);
    guest.emit('did-start-navigation', { url: allowed, isMainFrame: true });

    expect(guest.stop).toHaveBeenCalledTimes(1);
  });

  /**
   * The two doors were separately wired once, drifted, and shipped a
   * documented-but-absent block for three weeks (a72c0787 → 86912a14). This is
   * the assertion that they answer the SAME question: one roots supplier, and
   * identical verdicts for the same URL through the attach door and the
   * navigation door.
   */
  it('gives the attach door and the navigation door identical verdicts', () => {
    const cases = [
      'https://example.com',
      'http://127.0.0.1:7895/plugins/ui/foo/',
      'chrome://settings',
      'data:text/html,x',
      'about:config',
      'file:///etc/passwd',
      fileUrl(path.join(root, 'design', 'index.html')),
      fileUrl(path.join(root, 'design', 'DESIGN.md')),
      fileUrl(path.join(root, 'deploy.sh')),
      fileUrl(path.join(outside, 'evil.html')),
      fileUrl(root) + '/../outside/evil.html',
      'file://evil/share/payload.html',
    ];

    for (const url of cases) {
      const host = fakeHost();
      installWebviewGuards(host, { allowedRoots: () => roots });
      const attachBlocked = attach(host, url).blocked;

      // Same host, same options object: the guest the navigation guard is
      // installed on comes straight out of did-attach-webview.
      const guest = new EventEmitter() as EventEmitter & {
        stop: ReturnType<typeof vi.fn>;
        loadURL: ReturnType<typeof vi.fn>;
        getURL: () => string;
      };
      guest.stop = vi.fn();
      guest.loadURL = vi.fn();
      guest.getURL = () => ''; // no page yet, so the origin rule cannot fire
      host.emit('did-attach-webview', {}, guest);
      guest.emit('did-start-navigation', { url, isMainFrame: true });
      const navBlocked = guest.stop.mock.calls.length > 0;

      expect(navBlocked, `${url}: attach=${attachBlocked} nav=${navBlocked}`).toBe(attachBlocked);
    }
  });
});

/**
 * pathConfinement's caller contract: the walk resolves every symlink and every
 * `..` to decide a path is allowed, and the caller must hand the RESOLVED path
 * to the operation. Otherwise the guard checked `<root>/link/x.html` and
 * Chromium opened wherever `link` really points, which is a different file by
 * the time the check is over.
 */
describe('the attach door hands Chromium the CANONICAL path', () => {
  let linkedSrc: string;
  let canonicalSrc: string;

  beforeAll(() => {
    // A symlinked directory INSIDE the root, pointing at another directory
    // inside the same root: allowed either way, so the only thing under test is
    // which spelling is loaded.
    const link = path.join(root, 'shortcut');
    if (!fs.existsSync(link)) fs.symlinkSync(path.join(root, 'design'), link);
    linkedSrc = fileUrl(path.join(root, 'shortcut', 'index.html'));
    canonicalSrc = pathToFileURL(path.join(root, 'design', 'index.html')).href;
  });

  function attachWithParams(src: string) {
    const host = new EventEmitter();
    installWebviewGuards(host, { allowedRoots: () => roots });
    const event = { preventDefault: vi.fn() };
    const params: { src?: string } = { src };
    host.emit('will-attach-webview', event, {} as MutableWebPreferences, params);
    return { host, params, blocked: event.preventDefault.mock.calls.length > 0 };
  }

  it('rewrites an allowed file: src to its canonical spelling', () => {
    const { params, blocked } = attachWithParams(linkedSrc);
    expect(blocked).toBe(false);
    expect(params.src).toBe(canonicalSrc);
  });

  it('leaves an http(s) src exactly as it was', () => {
    const { params } = attachWithParams('https://example.com/a?b=1#c');
    expect(params.src).toBe('https://example.com/a?b=1#c');
  });

  it('leaves a REFUSED src alone rather than rewriting it', () => {
    const denied = fileUrl(path.join(outside, 'evil.html'));
    const { params, blocked } = attachWithParams(denied);
    expect(blocked).toBe(true);
    expect(params.src).toBe(denied);
  });

  it('recognises its own attach load under EITHER spelling', () => {
    for (const reported of [linkedSrc, canonicalSrc]) {
      const { host } = attachWithParams(linkedSrc);
      const guest = new EventEmitter() as EventEmitter & {
        stop: ReturnType<typeof vi.fn>;
        loadURL: ReturnType<typeof vi.fn>;
        getURL: () => string;
      };
      guest.stop = vi.fn();
      guest.loadURL = vi.fn();
      guest.getURL = () => '';
      host.emit('did-attach-webview', {}, guest);
      guest.emit('did-start-navigation', { url: reported, isMainFrame: true });
      expect(guest.stop, reported).not.toHaveBeenCalled();
    }
  });
});

/**
 * A source-level check on the WIRING, because the parity test above can only
 * prove the two functions agree, not that index.ts still calls them together.
 * The bug this whole file exists for was a second call site with its own policy.
 */
describe('index.ts wiring', () => {
  const src = fs.readFileSync(path.join(__dirname, '..', 'index.ts'), 'utf8');

  it('installs both doors through one call, against one roots supplier', () => {
    expect(src.match(/installWebviewGuards\(/g)).toHaveLength(1);
    expect(src).toMatch(/allowedRoots: webviewFileRoots/);
  });

  it('has no hand-rolled second webview door', () => {
    expect(src).not.toMatch(/on\('will-attach-webview'/);
    expect(src).not.toMatch(/on\('did-attach-webview'/);
  });
});
