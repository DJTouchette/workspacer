/**
 * Attach-time hardening for <webview> tags (SECURITY.md #10).
 *
 * The main window enables `webviewTag` so two things can embed pages: BrowserPane
 * (arbitrary http(s) browsing) and plugin panes (loaded from the hub UI origin or
 * a 127.0.0.1 sidecar server). Left unguarded, renderer content could inject a
 * <webview> that turns on `nodeIntegration` or a `preload` script — gaining
 * main-process/native reach — or points `src` at `file://` to read the host
 * filesystem. The main process force-applies safe web preferences on every attach
 * and restricts the src (and later navigations) to remote-browsing schemes.
 *
 * These are split out as pure functions so the policy is unit-testable without
 * standing up an Electron BrowserWindow.
 */

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
 * Whether a webview may attach with — or navigate to — `src`. Legitimate webviews
 * load http/https (arbitrary browsing, the 127.0.0.1 plugin sidecar servers, and
 * the hub UI origin) or `about:blank` (an empty shell that then `loadURL()`s). Any
 * other scheme — notably `file://`, plus `chrome:`, `devtools:`, `data:`, etc. —
 * is rejected so embedded content can never reach the host filesystem or a
 * privileged internal page. Fails closed on an unparseable src.
 */
export function isWebviewSrcAllowed(src: string | undefined): boolean {
  // An empty src attaches an about:blank shell that the pane drives via loadURL();
  // that later navigation is itself checked, so allow the empty attach.
  if (!src || src === 'about:blank') return true;
  let scheme: string;
  try {
    scheme = new URL(src).protocol;
  } catch {
    return false; // unparseable — fail closed
  }
  // Only http/https reach here; about:blank is already allowed above, and every
  // other about: URL (about:config, about:srcdoc, about:blank#x, …) is rejected.
  return scheme === 'http:' || scheme === 'https:';
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
}

/**
 * Confine a guest <webview> to the schemes isWebviewSrcAllowed permits, for the
 * lifetime of the webview.
 *
 * The subtlety is which event to hang this on. `will-navigate` is cancelable
 * but only fires for navigations the GUEST PAGE starts (a link click,
 * `window.location = …`). BrowserPane doesn't navigate that way — it calls
 * `webview.loadURL()` from the renderer, an embedder-initiated navigation that
 * `will-navigate` never sees. So the address bar was, in practice, unguarded:
 * SECURITY.md #10's claim that a typed `file://` URL is blocked was false as
 * shipped. `did-start-navigation` fires for every navigation including
 * loadURL — but it is NOT cancelable, so the block is stop() plus a bounce to
 * about:blank rather than preventDefault(). The cancelable pair stays wired as
 * well: catching a bad navigation before it starts is still better when the
 * event does fire.
 */
export function installWebviewNavigationGuard(guest: GuardableContents): void {
  const cancel = (e: { preventDefault(): void }, url: string) => {
    if (isWebviewSrcAllowed(url)) return;
    console.warn(`[main] blocking <webview> navigation to disallowed url: ${url}`);
    e.preventDefault();
  };
  guest.on('will-navigate', cancel);
  guest.on('will-redirect', cancel);
  guest.on('did-start-navigation', (details: { url: string; isMainFrame: boolean }) => {
    if (!details.isMainFrame) return;
    if (isWebviewSrcAllowed(details.url)) return;
    console.warn(`[main] stopping <webview> navigation to disallowed url: ${details.url}`);
    guest.stop();
    // Leave the guest on a blank page rather than whatever it was showing —
    // about:blank is allowed, so this doesn't re-enter the guard.
    guest.loadURL('about:blank');
  });
}
