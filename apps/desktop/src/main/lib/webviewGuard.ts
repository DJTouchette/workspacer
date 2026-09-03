/**
 * Attach-time hardening for <webview> tags.
 *
 * The main window enables `webviewTag` so two things can embed pages: BrowserPane
 * (arbitrary http(s) browsing) and plugin panes (loaded from the hub UI origin or
 * a 127.0.0.1 sidecar server). Left unguarded, renderer content could inject a
 * <webview> that turns on `nodeIntegration` or a `preload` script — gaining
 * main-process/native reach — or points `src` at `file://` to read the host
 * filesystem. The main process force-applies safe web preferences on every attach
 * and restricts the src (and later navigations) to remote-browsing schemes plus a
 * BOUNDED set of local files.
 *
 * ## Why file: is allowed at all, and how narrowly
 *
 * Agents write HTML the user is meant to look at (design mockups, generated
 * reports) and then call `open_browser` on it. Refusing every file: URL made that
 * a blank pane, silently. The allowance is deliberately the smallest thing that
 * makes it work:
 *
 *   - the URL must have no host (or the literal `localhost`), so `file://evil/x`
 *     (an SMB/UNC fetch off the box) is not a local file;
 *   - the path is percent-decoded ONCE and then canonicalized per component by
 *     `pathConfinement.canonicalizePath`, which reads every symlink instead of
 *     textually collapsing `..` (a textual clean would check `<root>/x` while
 *     Chromium opened wherever `link/..` really points);
 *   - the canonical result must sit inside one of `allowedRoots` by the same
 *     containment rule the fs.* capabilities use, so `/home/user2` is not
 *     admitted by the root `/home/user`;
 *   - the target must EXIST and be a regular file, not a directory;
 *   - and its extension must be one a browser pane can actually render.
 *
 * What this does NOT do, deliberately: it does not touch `webSecurity`,
 * `sandbox`, `contextIsolation` or `allowFileAccessFromFileUrls`. A file: page
 * still gets Chromium's default opaque file origin, so it cannot fetch/XHR its
 * neighbours; it can only pull the subresources (css/js/img) a local page
 * normally can.
 *
 * The roots themselves are supplied by the caller (index.ts) as one expression
 * feeding BOTH doors, so the attach check and the navigation check cannot drift
 * apart. That drift is exactly what 86912a14 was fixing.
 *
 * These are split out as pure functions so the policy is unit-testable without
 * standing up an Electron BrowserWindow.
 */

import * as fs from 'fs';
import * as path from 'path';
import { canonicalizePath, pathWithinRoots } from './pathConfinement';

/** The mutable subset of Electron's webPreferences we override at attach time. */
export interface MutableWebPreferences {
  preload?: string;
  preloadURL?: string;
  nodeIntegration?: boolean;
  nodeIntegrationInSubFrames?: boolean;
  contextIsolation?: boolean;
  [k: string]: unknown;
}

/**
 * Force the non-negotiable safe prefs regardless of what the <webview> tag asked
 * for: strip any preload, disable node integration (top frame and sub-frames),
 * and require context isolation. A malicious `<webview nodeintegration preload=…>`
 * is thereby neutered even if it reaches attach.
 */
export function applySafeWebviewPreferences(prefs: MutableWebPreferences): void {
  delete prefs.preload;
  delete prefs.preloadURL;
  prefs.nodeIntegration = false;
  prefs.nodeIntegrationInSubFrames = false;
  prefs.contextIsolation = true;
}

/**
 * Extensions a browser pane can render from disk. Kept small on purpose: this is
 * the set a page a person wants to LOOK at is written in, not "every file type".
 * Everything else (.sh, .env, .pem, .key, an extensionless file) is refused,
 * so a mis-aimed open_browser cannot become a credential viewer.
 */
export const BROWSER_FILE_EXTENSIONS = new Set([
  'html',
  'htm',
  'svg',
  'png',
  'jpg',
  'jpeg',
  'gif',
  'webp',
  'txt',
  'json',
  'css',
  'js',
  'pdf',
]);

/** Markdown is refused here and routed to the mdpreview pane instead, because
 *  Chromium DOWNLOADS `text/markdown` over file: rather than rendering it, so allowing it
 *  would trade a blank pane for a surprise download. See browserBus.ts. */
export const PREVIEW_FILE_EXTENSIONS = new Set(['md', 'markdown']);

/** Why a src was refused. Carried to the renderer so the pane can say it. */
export interface SrcVerdict {
  allowed: boolean;
  /** Short, user-facing. Only set when `allowed` is false. */
  reason?: string;
}

const ALLOWED: SrcVerdict = { allowed: true };
const deny = (reason: string): SrcVerdict => ({ allowed: false, reason });

/** True for the two schemes that browse the network. */
function isRemoteScheme(url: string): boolean {
  try {
    const p = new URL(url).protocol;
    return p === 'http:' || p === 'https:';
  } catch {
    return false;
  }
}

/** True when `url` parses as a file: URL (whether or not it is ALLOWED). */
export function isFileUrl(url: string | undefined): boolean {
  if (!url) return false;
  try {
    return new URL(url).protocol === 'file:';
  } catch {
    return false;
  }
}

/** Lowercase extension without the dot, or '' when there is none. */
function extensionOf(p: string): string {
  const ext = path.extname(p);
  return ext.startsWith('.') ? ext.slice(1).toLowerCase() : '';
}

/**
 * The filesystem path a file: URL names, or null when it names none.
 *
 * `pathname` is decoded ONCE. A double-encoded traversal (`%252e%252e%2f`) therefore
 * decodes to the literal component `%2e%2e/`, which is a filename that does not
 * exist rather than a `..`. It is denied downstream by the existence check, not
 * laundered into a traversal by a second decode.
 */
function filePathOf(u: URL): string | null {
  let decoded: string;
  try {
    decoded = decodeURIComponent(u.pathname);
  } catch {
    return null; // malformed percent-escape: fail closed
  }
  if (decoded === '') return null;
  // Windows spells a local path `file:///C:/x/y.html`, whose pathname carries a
  // leading slash the filesystem does not want. UNC (`file://server/share`) has
  // a host and is refused before this is reached.
  if (process.platform === 'win32' && /^\/[a-zA-Z]:[\\/]/.test(decoded)) return decoded.slice(1);
  return decoded;
}

/**
 * Whether a webview may attach with (or navigate to) a `file:` `src`, and why
 * not when it may not. Split out of `checkWebviewSrc` only for readability; it is
 * never reached with a non-file URL.
 */
function checkFileSrc(u: URL, allowedRoots: string[]): SrcVerdict {
  // `file://localhost/x` is the same document as `file:///x` and WHATWG already
  // normalizes the host away; any OTHER authority means a remote fetch (SMB/UNC),
  // which is not a local file no matter what the path says.
  if (u.host !== '' && u.host !== 'localhost') {
    return deny('a file URL with a remote host is not a local file');
  }
  const raw = filePathOf(u);
  if (raw === null) return deny('this file URL names no path');

  let canonical: string;
  try {
    canonical = canonicalizePath(raw);
  } catch {
    // Not absolute, a symlink cycle, or an lstat error that is not ENOENT.
    return deny('this path could not be resolved');
  }
  // Containment BEFORE existence, so a refusal outside the roots never doubles as
  // an existence oracle for paths the user was never allowed to ask about.
  if (!pathWithinRoots(allowedRoots, canonical)) {
    return deny('this file is outside your home and project directories');
  }

  const ext = extensionOf(canonical);
  if (PREVIEW_FILE_EXTENSIONS.has(ext)) {
    return deny('markdown files open in the preview pane, not the browser');
  }
  if (!BROWSER_FILE_EXTENSIONS.has(ext)) {
    return deny(`the browser pane does not render ${ext ? '.' + ext : 'extensionless'} files`);
  }

  let st: fs.Stats;
  try {
    st = fs.statSync(canonical);
  } catch {
    return deny('no such file');
  }
  if (st.isDirectory()) return deny('this path is a directory, not a file');
  if (!st.isFile()) return deny('this path is not a regular file');

  return ALLOWED;
}

/**
 * Whether a webview may attach with (or navigate to) `src`, and why not when it
 * may not. Legitimate webviews load http/https (arbitrary browsing, the 127.0.0.1
 * plugin sidecar servers, and the hub UI origin), `about:blank` (an empty shell
 * that then `loadURL()`s), or a `file:` URL that clears every bar in the module
 * header. Every other scheme (`chrome:`, `devtools:`, `data:`, …) is rejected so
 * embedded content can never reach a privileged internal page. Fails closed on an
 * unparseable src, and on ANY exception from the filesystem walk.
 *
 * `allowedRoots` defaults to EMPTY, which allows no file at all: a caller that
 * forgets to supply roots gets the old, closed policy rather than a wildcard.
 */
export function checkWebviewSrc(src: string | undefined, allowedRoots: string[] = []): SrcVerdict {
  // An empty src attaches an about:blank shell that the pane drives via loadURL();
  // that later navigation is itself checked, so allow the empty attach.
  if (!src || src === 'about:blank') return ALLOWED;
  let url: URL;
  try {
    url = new URL(src);
  } catch {
    return deny('this address could not be parsed'); // unparseable: fail closed
  }
  // Only http/https reach here; about:blank is already allowed above, and every
  // other about: URL (about:config, about:srcdoc, about:blank#x, …) is rejected.
  if (url.protocol === 'http:' || url.protocol === 'https:') return ALLOWED;
  if (url.protocol === 'file:') {
    try {
      return checkFileSrc(url, allowedRoots);
    } catch {
      return deny('this path could not be checked'); // fail closed on anything unexpected
    }
  }
  return deny(`the ${url.protocol.replace(/:$/, '')} scheme cannot open in a pane`);
}

/**
 * Boolean face of `checkWebviewSrc`, for call sites that only need the verdict.
 */
export function isWebviewSrcAllowed(src: string | undefined, allowedRoots: string[] = []): boolean {
  return checkWebviewSrc(src, allowedRoots).allowed;
}

/**
 * The bits of Electron's WebContents this guard needs, so the policy can be
 * exercised against a plain event emitter in a test.
 */
export interface GuardableContents {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  on(event: string, listener: (...args: any[]) => void): unknown;
  stop(): void;
  loadURL(url: string): unknown;
  /** Present on a real WebContents; used to learn what page is navigating. */
  getURL?(): string;
  /** Present on a real WebContents; guards `allowpopups` window opens. */
  setWindowOpenHandler?(handler: (details: { url: string }) => { action: 'allow' | 'deny' }): void;
}

/** What the guard needs from its host, and what it reports back. */
export interface WebviewGuardOptions {
  /** Absolute directories a file: URL may live under. Called on EVERY check so a
   *  project added after boot is honoured without a restart. */
  allowedRoots: () => string[];
  /** Told about every refusal, so the UI can show it instead of a blank pane. */
  onBlocked?: (info: { url: string; reason: string; phase: 'attach' | 'navigate' }) => void;
  /** The src the guest attached with: the page a first navigation comes FROM. */
  initialUrl?: string;
}

/**
 * Confine a guest <webview> to what `checkWebviewSrc` permits, for the lifetime of
 * the webview, plus one rule that only makes sense once file: is allowed at all:
 *
 *   **a remote page may never navigate the pane to a local file.** An allowed
 *   file: target is still refused when the page doing the navigating is on http(s).
 *   Otherwise any web page the user visits could redirect the pane onto a file it
 *   guessed the path of and read it back through the same document.
 *   file: → file: is allowed when the TARGET also passes; file: → http(s) is
 *   allowed as before.
 *
 * The subtlety is which event to hang this on. `will-navigate` is cancelable
 * but only fires for navigations the GUEST PAGE starts (a link click,
 * `window.location = …`). BrowserPane doesn't navigate that way — it calls
 * `webview.loadURL()` from the renderer, an embedder-initiated navigation that
 * `will-navigate` never sees. So the address bar was, in practice, unguarded —
 * a typed `file://` URL was documented as blocked and was not, as shipped.
 * `did-start-navigation` fires for every navigation including
 * loadURL — but it is NOT cancelable, so the block is stop() plus a bounce to
 * about:blank rather than preventDefault(). The cancelable pair stays wired as
 * well: catching a bad navigation before it starts is still better when the
 * event does fire.
 */
export function installWebviewNavigationGuard(
  guest: GuardableContents,
  opts: WebviewGuardOptions = { allowedRoots: () => [] },
): void {
  /** Last URL we saw the guest settle on, when it cannot tell us itself. */
  let lastKnownUrl = opts.initialUrl ?? '';
  const currentUrl = (): string => {
    try {
      return guest.getURL?.() || lastKnownUrl;
    } catch {
      return lastKnownUrl;
    }
  };

  /** The full decision for one navigation: the src policy, then the origin rule. */
  const verdictFor = (url: string): SrcVerdict => {
    const v = checkWebviewSrc(url, opts.allowedRoots());
    if (!v.allowed) return v;
    if (isFileUrl(url) && isRemoteScheme(currentUrl())) {
      return deny('a web page may not open a local file in this pane');
    }
    return v;
  };

  const report = (url: string, reason: string) => {
    opts.onBlocked?.({ url, reason, phase: 'navigate' });
  };

  const cancel = (e: { preventDefault(): void }, url: string) => {
    const v = verdictFor(url);
    if (v.allowed) return;
    console.warn(`[main] blocking <webview> navigation to disallowed url: ${url}`);
    report(url, v.reason as string);
    e.preventDefault();
  };
  guest.on('will-navigate', cancel);
  guest.on('will-redirect', cancel);
  guest.on('did-start-navigation', (details: { url: string; isMainFrame: boolean }) => {
    if (!details.isMainFrame) return;
    const v = verdictFor(details.url);
    if (v.allowed) {
      lastKnownUrl = details.url;
      return;
    }
    console.warn(`[main] stopping <webview> navigation to disallowed url: ${details.url}`);
    report(details.url, v.reason as string);
    guest.stop();
    // Leave the guest on a blank page rather than whatever it was showing —
    // about:blank is allowed, so this doesn't re-enter the guard.
    guest.loadURL('about:blank');
  });
  // `allowpopups` is on, so a page can ask for a NEW window; without this a
  // `window.open('file:///…')` opened outside the pane, past both other doors.
  // Allowed URLs keep Electron's default behaviour.
  guest.setWindowOpenHandler?.(({ url }) => {
    const v = verdictFor(url);
    if (v.allowed) return { action: 'allow' };
    console.warn(`[main] blocking <webview> window.open of disallowed url: ${url}`);
    report(url, v.reason as string);
    return { action: 'deny' };
  });
}

/** The host webContents side of the guard (the main BrowserWindow). */
export interface GuardHostContents {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  on(event: string, listener: (...args: any[]) => void): unknown;
}

/**
 * Wire BOTH doors, the attach check and the per-guest navigation guard, from
 * one call, against ONE `allowedRoots` supplier.
 *
 * This exists so the two can no longer be given different policies by accident.
 * They were separately wired once, drifted, and shipped a documented-but-absent
 * block for three weeks (a72c0787 → 86912a14); a shared roots argument passed by
 * hand at two call sites would have reopened exactly that.
 */
export function installWebviewGuards(host: GuardHostContents, opts: WebviewGuardOptions): void {
  host.on(
    'will-attach-webview',
    (
      event: { preventDefault(): void },
      webPreferences: MutableWebPreferences,
      params: { src?: string },
    ) => {
      applySafeWebviewPreferences(webPreferences);
      const verdict = checkWebviewSrc(params.src, opts.allowedRoots());
      if (verdict.allowed) return;
      console.warn(`[main] blocking <webview> attach with disallowed src: ${params.src}`);
      opts.onBlocked?.({
        url: params.src ?? '',
        reason: verdict.reason as string,
        phase: 'attach',
      });
      event.preventDefault();
    },
  );
  host.on('did-attach-webview', (_event: unknown, guest: GuardableContents) => {
    installWebviewNavigationGuard(guest, opts);
  });
}
