/**
 * Cross-component bus for opening a URL (or local file) in Workspacer's own
 * in-app browser pane.
 *
 * A file affordance (e.g. FileLink's "Open in browser" on an .html file) calls
 * `requestOpenInBrowser`; App handles the event and opens a 'browser' pane
 * pointed at the target. Mirrors editorBus/previewBus, kept separate so the
 * concerns stay independent. This keeps HTML out of the OS default handler
 * (which may be an editor, not a browser).
 */

export const BROWSER_OPEN_EVENT = 'browser:open-url';

export interface BrowserOpenTarget {
  /** URL to load. For a local file, pass a `file://` URL. */
  url: string;
  /** Tab title — defaults to the URL. */
  title?: string;
}

export function requestOpenInBrowser(target: BrowserOpenTarget): void {
  window.dispatchEvent(new CustomEvent(BROWSER_OPEN_EVENT, { detail: target }));
}

/**
 * True when two URL strings name the same resource once WHATWG-normalised.
 *
 * Chromium reports the normalised href, so a byte comparison against the
 * spelling a pane was handed misses `file:///home/x/../y.html`, a `%2e%2e`
 * segment, an uppercase `FILE:///`, `http://EXAMPLE.com` and an unencoded space.
 * The twin of `sameUrl` in main/lib/webviewGuard.ts; both sides of the refusal
 * push have to agree about what "the same URL" means.
 */
export function sameUrl(a: string, b: string): boolean {
  if (a === b) return true;
  try {
    return new URL(a).href === new URL(b).href;
  } catch {
    return false;
  }
}

/** The filesystem path a `file://` URL names, or null when `url` is not a local
 *  file URL. The inverse of `fileUrlFromPath`, and the reason a caller can tell
 *  a `.md` target apart before handing it to a pane that cannot render one.
 *
 *  The Windows arm strips the leading slash from `/C:/x`. A POSIX path may
 *  legally look like that too; treating it as a drive letter is the lesser
 *  wrong, since the alternative mangles every real Windows path. */
export function pathFromFileUrl(url: string): string | null {
  let u: URL;
  try {
    u = new URL(url);
  } catch {
    return null;
  }
  if (u.protocol !== 'file:') return null;
  // Any authority beyond the (already-normalized) localhost is a remote fetch,
  // not a path on this machine.
  if (u.host !== '' && u.host !== 'localhost') return null;
  let decoded: string;
  try {
    decoded = decodeURIComponent(u.pathname);
  } catch {
    return null; // malformed percent-escape
  }
  if (!decoded) return null;
  return /^\/[a-zA-Z]:[\\/]/.test(decoded) ? decoded.slice(1) : decoded;
}

/**
 * The path behind a `file://` URL that points at markdown, or null.
 *
 * Chromium DOWNLOADS `text/markdown` over `file:` instead of rendering it, so a
 * markdown target must never reach the browser pane: every place that dispatches
 * a file: open routes it to the mdpreview pane through this.
 */
export function markdownPathFromFileUrl(url: string): string | null {
  const p = pathFromFileUrl(url);
  return p && /\.(md|markdown)$/i.test(p) ? p : null;
}

/** The verdict `previewFileAllowed` resolves. `canonicalPath` is present only
 *  when `allowed` is true: the CANONICAL path main resolved the URL to, which
 *  the caller must open verbatim (see previewFileAllowed's own doc). */
export interface PreviewFileVerdict {
  allowed: boolean;
  canonicalPath?: string;
}

/**
 * Whether main will allow a `file:` URL to open in the MARKDOWN PREVIEW pane,
 * and if so, the CANONICAL path it checked.
 *
 * `markdownPathFromFileUrl` above only asks whether a URL ends in `.md`; it has
 * never seen the allowed roots, and the `file:read` behind the preview pane
 * applies no confinement of its own. So the detour that keeps markdown OUT of
 * the browser pane was, by itself, a wider door than the pane it detours around:
 * `open_browser` on `file:///etc/ssl/README.md` rendered an out-of-root file,
 * and renaming anything to `.md` sidestepped the browser arm entirely.
 *
 * Every dispatch point asks this BEFORE it opens the preview, and opens the
 * `canonicalPath` this returns rather than its own parse of the URL: main
 * resolved every symlink and every `..` to decide the file was allowed, and
 * re-deriving a path from the renderer's own (unresolved) parse of `url` is the
 * check-path/opened-path split pathConfinement's caller contract exists to
 * close — a component that was an ordinary file at check time and a symlink out
 * of the roots by the time the pane reads it would otherwise render anyway.
 *
 * Fails CLOSED when the backend does not answer: a check we could not run is not
 * a check that passed.
 */
export async function previewFileAllowed(url: string): Promise<PreviewFileVerdict> {
  const check = window.electronAPI?.checkPreviewFile;
  if (!check) return { allowed: false };
  try {
    const v = await check(url);
    return v?.allowed === true && v.canonicalPath
      ? { allowed: true, canonicalPath: v.canonicalPath }
      : { allowed: false };
  } catch {
    return { allowed: false };
  }
}

/** Build a `file://` URL from an absolute filesystem path (Windows or POSIX). */
export function fileUrlFromPath(absPath: string): string {
  // Normalize Windows backslashes, ensure a leading slash, and encode spaces
  // and other reserved characters segment-by-segment (but keep the slashes).
  const normalized = absPath.replace(/\\/g, '/');
  const withSlash = normalized.startsWith('/') ? normalized : '/' + normalized;
  const encoded = withSlash
    .split('/')
    .map((seg) => encodeURIComponent(seg))
    .join('/');
  return 'file://' + encoded;
}
