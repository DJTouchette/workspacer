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
import {
  applySafeWebviewPreferences,
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
