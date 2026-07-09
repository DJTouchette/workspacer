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
