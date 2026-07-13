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
