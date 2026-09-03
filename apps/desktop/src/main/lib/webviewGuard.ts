/**
 * Attach-time hardening for <webview> tags.
 *
 * The main window enables `webviewTag` so two things can embed pages: BrowserPane
 * (arbitrary http(s) browsing) and plugin panes (loaded from the hub UI origin or
 * a 127.0.0.1 sidecar server). Left unguarded, renderer content could inject a
 * <webview> that turns on `nodeIntegration` or a `preload` script, gaining
 * main-process/native reach, or points `src` at `file://` to read the host
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
 *   - it must survive `pathConfinement.isSecretPath`, the same credential gate
 *     `assertPathAllowed` applies to every fs.* caller, so a credential or an
 *     agent-configuration file is refused whatever it is named. One of the
 *     roots is the whole home directory, so being inside a root was never the
 *     same question as being safe to render;
 *   - the target must EXIST and be a regular file, not a directory;
 *   - and its extension must be one a browser pane can actually render.
 *
 * What this does NOT do, deliberately: it does not touch `sandbox`. A file:
 * page still gets Chromium's default opaque file origin, so it cannot fetch/XHR
 * its neighbours; it can only pull the subresources (css/js/img) a local page
 * normally can, and `webSecurity` / `allowFileAccessFromFileUrls` are pinned
 * to that default on every attach (applySafeWebviewPreferences), so a guest
 * cannot request its way out of that origin via its own `webpreferences`
 * attribute.
 *
 * And what it does not claim: SUB-FRAME loads inside an allowed local page are
 * NOT guarded. `did-start-navigation` is filtered to the main frame, so an
 * allowed local page may iframe another local file, including one outside the
 * roots. That is DISPLAY-ONLY: the page cannot read the frame back, because the
 * two get distinct opaque file origins under Chromium's default
 * `allowFileAccessFromFileUrls: false`, and nothing here changes that. Confining
 * sub-frames would mean judging every image, stylesheet and script a local page
 * pulls in, which is the thing the file allowance exists to permit.
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
import { pathToFileURL } from 'url';
import { canonicalizePath, isSecretPath, pathWithinRoots } from './pathConfinement';

/** The mutable subset of Electron's webPreferences we override at attach time. */
export interface MutableWebPreferences {
  preload?: string;
  preloadURL?: string;
  nodeIntegration?: boolean;
  nodeIntegrationInSubFrames?: boolean;
  contextIsolation?: boolean;
  webSecurity?: boolean;
  allowFileAccessFromFileUrls?: boolean;
  [k: string]: unknown;
}

/**
 * Force the non-negotiable safe prefs regardless of what the <webview> tag asked
 * for: strip any preload, disable node integration (top frame and sub-frames),
 * require context isolation, and pin webSecurity on / allowFileAccessFromFileUrls
 * off. A malicious `<webview nodeintegration preload=…>` is thereby neutered
 * even if it reaches attach.
 *
 * The last two are DEFAULTS, not a behaviour change: Electron already ships
 * `webSecurity: true` and `allowFileAccessFromFileUrls: false` for a fresh
 * `<webview>`. They earn a line here because Electron 43.4.1 spreads a
 * `<webview webpreferences="...">` attribute onto this object AFTER its own
 * inheritance clamp, and that clamp covers contextIsolation, sandbox,
 * nodeIntegration (top and sub-frame), javascript and enableWebSQL but not
 * these two, so a guest page that can set its OWN
 * `<webview webpreferences="allowFileAccessFromFileUrls">` on a file: page
 * gets whole-filesystem XHR read, the thing the file: allowance's entire
 * design (see the module header) assumes stays off.
 */
export function applySafeWebviewPreferences(prefs: MutableWebPreferences): void {
  delete prefs.preload;
  delete prefs.preloadURL;
  prefs.nodeIntegration = false;
  prefs.nodeIntegrationInSubFrames = false;
  prefs.contextIsolation = true;
  prefs.webSecurity = true;
  prefs.allowFileAccessFromFileUrls = false;
}

/**
 * Extensions a browser pane can render from disk. Kept small on purpose: this is
 * the set a page a person wants to LOOK at is written in, not "every file type".
 * Everything else (.sh, .env, .pem, .key, an extensionless file) is refused.
 *
 * `json` is NOT on this list, and its absence is load-bearing rather than an
 * oversight. The feature is "an agent wrote HTML and asked you to look at it";
 * no mockup is a .json. What a .json IS, under a root as wide as the home
 * directory, is `~/.claude/.credentials.json`, `~/.claude.json`,
 * `~/.claude/settings.json` and `~/.docker/config.json`. The extension gate is
 * not the only thing standing between the pane and those (see the credential
 * gate in checkFileSrc), but an allowance nothing needs is not worth defending
 * twice.
 *
 * `pdf` is off the list for a different reason: Chromium's internal PDF viewer
 * is a plugin, and `plugins` is FALSE by default on a `<webview>`'s
 * webPreferences and is not turned on anywhere in this app. With it off, a
 * `file:` PDF does not render, it DOWNLOADS, which is the same surprise the
 * markdown detour exists to avoid.
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
  'css',
  'js',
]);

/** Markdown is refused here and routed to the mdpreview pane instead, because
 *  Chromium DOWNLOADS `text/markdown` over file: rather than rendering it, so allowing it
 *  would trade a blank pane for a surprise download. See browserBus.ts. */
export const PREVIEW_FILE_EXTENSIONS = new Set(['md', 'markdown']);

/**
 * Which pane is asking.
 *
 * The two doors render DISJOINT extension sets: the browser refuses markdown
 * (Chromium downloads `text/markdown` over file:) and the preview renders
 * nothing else. Everything else about the check is the same rule on the same
 * roots: the host test, the per-component canonicalization, containment, the
 * credential gate, and existence. Passing a door rather than a bare extension
 * set is what keeps the markdown detour from being a SECOND, unconfined policy
 * written somewhere in the renderer.
 */
export type PaneDoor = 'browser' | 'preview';

/** What the renderer is told about a refusal, so a pane that is blank ON PURPOSE
 *  can say why. `previewPath` is present only for the markdown detour. */
export interface WebviewBlockedInfo {
  url: string;
  reason: string;
  phase: 'attach' | 'navigate';
  previewPath?: string;
  /**
   * The guest the refusal is ABOUT, so the pane that owns it can claim it by
   * identity instead of by matching a string.
   *
   * Every pane hears every refusal, and matching on the URL alone does not
   * work: Chromium reports the WHATWG-normalised href, while the pane holds the
   * spelling it was handed. `file:///home/x/../y.html`, a `%2e%2e` segment, an
   * uppercase `FILE:///`, `http://EXAMPLE.com` and an unencoded space all come
   * back different from what was sent, and the address-bar traversal (the case
   * the banner most needs to explain) is exactly a `..` URL. Absent on a
   * refused ATTACH: the guest does not exist yet, and there the URL is the only
   * identity anything has.
   */
  webContentsId?: number;
}

/** Why a src was refused. Carried to the renderer so the pane can say it. */
export interface SrcVerdict {
  allowed: boolean;
  /** Short, user-facing. Only set when `allowed` is false. */
  reason?: string;
  /** Set ONLY when the browser door refused an in-root markdown file: the
   *  CANONICAL path the preview pane should open instead. Absent on every other
   *  refusal, so a pane can offer "open the preview" without deciding for itself
   *  whether the target was inside a root. */
  previewPath?: string;
  /** The path this verdict was ABOUT, per component resolved. Set only when a
   *  `file:` URL was ALLOWED. pathConfinement's caller contract: every caller
   *  must hand the RETURNED canonical path to the operation, because the check
   *  and the open are otherwise two different paths. */
  canonicalPath?: string;
}

const ALLOWED: SrcVerdict = { allowed: true };
const deny = (reason: string): SrcVerdict => ({ allowed: false, reason });

/**
 * True when two URL strings name the same resource once WHATWG-normalised.
 *
 * Chromium reports the NORMALISED href, so a byte comparison against the
 * spelling a `<webview>` tag asked for misses `file:///a/../b.html`, a `%2e%2e`
 * segment, an uppercase `FILE:///` scheme and an unencoded space. Fails closed
 * on anything unparseable: two URLs we cannot compare are not the same URL.
 */
export function sameUrl(a: string, b: string): boolean {
  if (a === b) return true;
  try {
    return new URL(a).href === new URL(b).href;
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
function checkFileSrc(u: URL, allowedRoots: string[], door: PaneDoor): SrcVerdict {
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

  // The credential gate, BEFORE the extension gate, so it holds regardless of
  // what the file is called. This is the same second predicate assertPathAllowed
  // applies to every fs.* caller: a credential basename, a `.git` subtree
  // (`.git/config` is a place to define a program git then runs), git's per-user
  // config, a provider CLI's hooks/permissions/MCP files, the workspacer config
  // dir outside its library/layouts/sessions carve-outs, and the hub's own state
  // dir. A root is only as narrow as the directories the user works in, and the
  // home directory is one of the roots, so "inside a root" was never the same
  // question as "safe to render".
  //
  // Not all four of the .json files above are on that list: it is a shared
  // TWIN of two Go copies pinned by contracts/path-containment-cases.json, so
  // it is not the place to add names. `~/.claude.json` and
  // `~/.claude/settings.json` are refused here; `~/.claude/.credentials.json`
  // and `~/.docker/config.json` are refused by the extension gate below now
  // that json is off the allowlist. Both halves are needed.
  if (isSecretPath(canonical)) {
    return deny('this file holds credentials or agent configuration');
  }

  const ext = extensionOf(canonical);
  if (door === 'preview') {
    if (!PREVIEW_FILE_EXTENSIONS.has(ext)) {
      return deny('the preview pane only opens markdown files');
    }
  } else {
    if (PREVIEW_FILE_EXTENSIONS.has(ext)) {
      return {
        allowed: false,
        reason: 'markdown files open in the preview pane, not the browser',
        // Reached only AFTER containment passed, so this path is inside a root
        // by construction and offering it is not a second, wider door.
        previewPath: canonical,
      };
    }
    if (!BROWSER_FILE_EXTENSIONS.has(ext)) {
      return deny(`the browser pane does not render ${ext ? '.' + ext : 'extensionless'} files`);
    }
  }

  let st: fs.Stats;
  try {
    st = fs.statSync(canonical);
  } catch {
    return deny('no such file');
  }
  if (st.isDirectory()) return deny('this path is a directory, not a file');
  if (!st.isFile()) return deny('this path is not a regular file');

  return { allowed: true, canonicalPath: canonical };
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
export function checkWebviewSrc(
  src: string | undefined,
  allowedRoots: string[] = [],
  door: PaneDoor = 'browser',
): SrcVerdict {
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
      return checkFileSrc(url, allowedRoots, door);
    } catch {
      return deny('this path could not be checked'); // fail closed on anything unexpected
    }
  }
  return deny(`the ${url.protocol.replace(/:$/, '')} scheme cannot open in a pane`);
}

/**
 * Whether a `file:` URL may be opened in the MARKDOWN PREVIEW pane.
 *
 * The preview pane is the other half of the same allowance: the browser door
 * refuses markdown and names the preview as where it does open, so without this
 * the detour was the widest door in the feature. `markdownPathFromFileUrl` in
 * the renderer only asks whether a URL ENDS IN `.md`; it has never seen the
 * roots, and `IPC.FILE_READ` behind it applies no confinement of its own. So
 * `open_browser` on `file:///etc/ssl/README.md` rendered an out-of-root file,
 * and renaming any unreadable file to `.md` sidestepped the browser arm
 * entirely.
 *
 * This is the SAME predicate on the SAME roots, with only the renderable
 * extension set swapped, so the two doors cannot drift into different opinions
 * about where a file may live. Non-file URLs are refused outright: an http(s)
 * page is not a thing the preview pane opens.
 */
export function checkPreviewFileUrl(url: string, allowedRoots: string[] = []): SrcVerdict {
  if (!isFileUrl(url)) return deny('the preview pane only opens local files');
  return checkWebviewSrc(url, allowedRoots, 'preview');
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
  /** Present on a real WebContents. Carried into a refusal so the pane that owns
   *  this guest can claim it without string-matching a URL. */
  id?: number;
  /** Present on a real WebContents; guards `allowpopups` window opens. */
  setWindowOpenHandler?(handler: (details: { url: string }) => { action: 'allow' | 'deny' }): void;
}

/** What the guard needs from its host, and what it reports back. */
export interface WebviewGuardOptions {
  /** Absolute directories a file: URL may live under. Called on EVERY check so a
   *  project added after boot is honoured without a restart. */
  allowedRoots: () => string[];
  /** Told about every refusal, so the UI can show it instead of a blank pane. */
  onBlocked?: (info: WebviewBlockedInfo) => void;
  /**
   * Every spelling of the src the ATTACH door already approved for this guest.
   *
   * The first load a guest performs has NO committed page to judge it by, and
   * the rule below refuses a file: navigation from anything that is not itself
   * an allowed file: page. Without this, the pane's whole reason for existing
   * (`<webview src="file:///.../mockup.html">`) would be refused by its own
   * second door. It is a one-shot: consumed by the first navigation the guest
   * reports, so a page cannot replay it afterwards.
   */
  approvedAttachSrcs?: string[];
}

/**
 * Confine a guest <webview> to what `checkWebviewSrc` permits, for the lifetime of
 * the webview, plus one rule that only makes sense once file: is allowed at all:
 *
 *   **only a local page may navigate the pane to a local file.** An allowed
 *   file: target is refused unless the page doing the navigating is ITSELF an
 *   allowed file: page. Otherwise any web page the user visits could redirect
 *   the pane onto a file it guessed the path of and read it back through the
 *   same document. file: to file: is allowed when the TARGET also passes;
 *   file: to http(s) is allowed as before.
 *
 *   The rule is stated as an allow-list on the SOURCE, not a deny-list of
 *   remote schemes, because the deny-list shape leaked twice. `about:blank` is
 *   not http(s) but INHERITS its initiator's origin, so an https page could
 *   route through `window.open('about:blank')` and navigate the result to
 *   `file:`; and a guest whose `getURL()` is still `''` (no committed document)
 *   is not remote either, so an empty-string source was a standing bypass on
 *   both the navigation door and the window-open door. Neither is an allowed
 *   file: URL, so both are refused now. The one source that legitimately has no
 *   committed page is the attach load itself, and that is what
 *   `approvedAttachSrcs` is for: the attach door has already checked it.
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
  let lastKnownUrl = '';
  const currentUrl = (): string => {
    try {
      return guest.getURL?.() || lastKnownUrl;
    } catch {
      return lastKnownUrl;
    }
  };

  /** The attach src(es) this guest's FIRST load may be, still unconsumed. Only
   *  file: spellings are kept: every other scheme is judged by the src policy
   *  alone and needs no exemption. */
  let pendingAttachSrcs = (opts.approvedAttachSrcs ?? []).filter((s) => isFileUrl(s));

  /** Whether the page currently committed in the guest may open a local file.
   *  ONLY an allowed file: page may. A guest with no committed document
   *  (`getURL()` is `''`) and an `about:blank` shell both answer false. */
  const sourceMayOpenALocalFile = (): boolean => {
    const from = currentUrl();
    if (!isFileUrl(from)) return false;
    return checkWebviewSrc(from, opts.allowedRoots()).allowed;
  };

  /** The full decision for one navigation: the src policy, then the origin rule. */
  const verdictFor = (url: string): SrcVerdict => {
    const v = checkWebviewSrc(url, opts.allowedRoots());
    if (!v.allowed) return v;
    if (!isFileUrl(url)) return v;
    if (pendingAttachSrcs.some((s) => sameUrl(url, s))) return v;
    if (!sourceMayOpenALocalFile()) {
      return deny('a web page may not open a local file in this pane');
    }
    return v;
  };

  const report = (url: string, v: SrcVerdict) => {
    opts.onBlocked?.({
      url,
      reason: v.reason as string,
      phase: 'navigate',
      previewPath: v.previewPath,
      webContentsId: typeof guest.id === 'number' ? guest.id : undefined,
    });
  };

  const cancel = (e: { preventDefault(): void }, url: string) => {
    const v = verdictFor(url);
    if (v.allowed) return;
    console.warn(`[main] blocking <webview> navigation to disallowed url: ${url}`);
    report(url, v);
    e.preventDefault();
  };
  guest.on('will-navigate', cancel);
  guest.on('will-redirect', cancel);
  // Deliberately NOT rewritten to the canonical path here, unlike the attach
  // door. did-start-navigation fires for a navigation Chromium has ALREADY
  // begun: there is nothing left to hand it. Substituting a URL at this point
  // would mean cancelling a load and starting a different one, which is a
  // navigation the pane never asked for and a history entry the user did not
  // make. The residual gap is a TOCTOU one (a component swapped between the
  // check and the open), the same one every path guard in this codebase carries,
  // and it is not widened by leaving the spelling alone.
  guest.on('did-start-navigation', (details: { url: string; isMainFrame: boolean }) => {
    if (!details.isMainFrame) return;
    const v = verdictFor(details.url);
    // The attach load has started, so the exemption has done its one job. Every
    // later navigation is judged by the committed page alone.
    pendingAttachSrcs = [];
    if (v.allowed) {
      lastKnownUrl = details.url;
      return;
    }
    console.warn(`[main] stopping <webview> navigation to disallowed url: ${details.url}`);
    report(details.url, v);
    guest.stop();
    // Leave the guest on a blank page rather than whatever it was showing —
    // about:blank is allowed, so this doesn't re-enter the guard.
    guest.loadURL('about:blank');
  });
  // `allowpopups` is on, so a page can ask for a NEW window; without this a
  // `window.open('file:///…')` opened outside the pane, past both other doors.
  //
  // This door judges `file:` URLS AND NOTHING ELSE. Every other scheme gets
  // Electron's native default, which is what an absent handler would do, and
  // that is deliberate: the pane exists in large part to carry "Sign in with
  // Google/Microsoft", those popups are ordinary https windows, and a handler
  // that judged them is exactly what was reverted in d2537bcc for "aborting
  // unrelated navigations" (see the note in index.ts). Nothing is lost by
  // narrowing it, because a popup is not unguarded afterwards: the guard below
  // is installed on the window this handler admits, and its
  // `did-start-navigation` applies the FULL src policy to the popup's first
  // load. A `window.open('chrome://…')` is admitted here and stopped there.
  //
  // Electron's window-open handler has no URL slot: it answers allow or deny,
  // and `overrideBrowserWindowOptions` overrides window options, not the target.
  // So the canonical rewrite the attach door performs cannot be repeated here.
  // The popup guard installed below re-derives its own verdict from whatever
  // Chromium actually navigates to, which is the closest equivalent available.
  guest.setWindowOpenHandler?.(({ url }) => {
    if (!isFileUrl(url)) return { action: 'allow' };
    const v = verdictFor(url);
    if (v.allowed) return { action: 'allow' };
    console.warn(`[main] blocking <webview> window.open of disallowed url: ${url}`);
    report(url, v);
    return { action: 'deny' };
  });
  // A popup the pane opens becomes a plain BrowserWindow. `installWebviewGuards`
  // is wired to the MAIN window's webContents and its `did-attach-webview` never
  // fires for one, so an admitted popup was the one surface in this feature with
  // no navigation guard on it at all: a page could open a window and navigate
  // THAT to a local file. It gets the same guard and no attach exemption,
  // because nothing has approved a src for it.
  guest.on('did-create-window', (win: { webContents?: GuardableContents } | undefined) => {
    const popup = win?.webContents;
    if (!popup) return;
    installWebviewNavigationGuard(popup, {
      allowedRoots: opts.allowedRoots,
      onBlocked: opts.onBlocked,
    });
  });
}

/** The host webContents side of the guard (the main BrowserWindow). */
export interface GuardHostContents {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  on(event: string, listener: (...args: any[]) => void): unknown;
}

/**
 * The canonical `file:` src to hand Chromium, keeping the ORIGINAL request's
 * query and fragment. `pathConfinement.canonicalizePath` resolves a filesystem
 * path, not a URL, so `verdict.canonicalPath` carries neither, and rewriting
 * straight from it dropped both: `open_browser` on `report.html#findings` lost
 * the anchor the moment the rewrite replaced the whole URL with the bare path.
 * `originalSrc` was already parsed once by `checkWebviewSrc` to reach here, so
 * the reparse below cannot fail in practice; it falls back to the bare
 * canonical URL rather than throw if it somehow does.
 */
function rewriteFileSrc(canonicalPath: string, originalSrc: string): string {
  const rewritten = pathToFileURL(canonicalPath);
  try {
    const original = new URL(originalSrc);
    rewritten.search = original.search;
    rewritten.hash = original.hash;
  } catch {
    // Unreached in practice; see doc comment above.
  }
  return rewritten.href;
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
  /**
   * FIFO queue of the exemption for each attach the attach door has APPROVED
   * but not yet paired with its guest. Electron fires will-/did-attach-webview
   * as a pair for one guest, but the PAIRS across different guests can
   * interleave (will A, will B, did A, did B) when two `<webview>`s attach
   * around the same time: a single mutable "most recently approved" value
   * handed pane 2's exemption to pane 1's guest (and left pane 2 with none),
   * bouncing BOTH to about:blank. did-attach-webview never fires for a
   * REFUSED attach (Chromium never creates that guest), so only an approved
   * attach enqueues here, and each did-attach-webview consumes exactly the
   * entry its own will-attach-webview pushed.
   */
  const pendingApprovals: string[][] = [];
  host.on(
    'will-attach-webview',
    (
      event: { preventDefault(): void },
      webPreferences: MutableWebPreferences,
      params: { src?: string },
    ) => {
      applySafeWebviewPreferences(webPreferences);
      const verdict = checkWebviewSrc(params.src, opts.allowedRoots());
      if (verdict.allowed) {
        const approvedSrcs: string[] = [];
        if (params.src) approvedSrcs.push(params.src);
        // Hand Chromium the CANONICAL path, not the spelling the tag asked
        // with. pathConfinement's caller contract: the walk resolved every
        // symlink and every `..` to decide this was allowed, and re-passing the
        // raw string is the check-path/opened-path split that whole module
        // exists to close. This is one of the two places the URL can still be
        // rewritten before the load starts.
        //
        // Both spellings stay in approvedSrcs. If a future Electron stops
        // honouring a mutation of `params`, the guest reports the original href
        // and the navigation door must still recognise its own attach load.
        if (verdict.canonicalPath) {
          const canonicalSrc = rewriteFileSrc(verdict.canonicalPath, params.src as string);
          params.src = canonicalSrc;
          approvedSrcs.push(canonicalSrc);
        }
        pendingApprovals.push(approvedSrcs);
        return;
      }
      console.warn(`[main] blocking <webview> attach with disallowed src: ${params.src}`);
      opts.onBlocked?.({
        url: params.src ?? '',
        reason: verdict.reason as string,
        phase: 'attach',
        previewPath: verdict.previewPath,
      });
      event.preventDefault();
    },
  );
  host.on('did-attach-webview', (_event: unknown, guest: GuardableContents) => {
    const approvedAttachSrcs = pendingApprovals.shift() ?? [];
    installWebviewNavigationGuard(guest, { ...opts, approvedAttachSrcs });
  });
}
