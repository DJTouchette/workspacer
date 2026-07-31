import { useEffect, useState } from 'react';

/**
 * Path → thumbnail data URL, shared by every surface that shows an image the
 * agent or the user attached: the composer's chips and the transcript's inline
 * thumbnails. One cache, kept across mounts, so attaching a screenshot and then
 * sending it decodes the file once rather than once per surface.
 *
 * Main does the decoding and downscaling (see main/services/imagePreview.ts)
 * because the renderer can't read host paths directly — in dev it's served over
 * http, where `file://` is blocked.
 */

/** `dataUrl: null` records a path we already failed on, so a missing or
 *  undecodable file isn't retried on every render. */
interface CachedPreview {
  dataUrl: string | null;
  at: number;
}
const previewCache = new Map<string, CachedPreview>();
const MAX_CACHED_PREVIEWS = 64;
/**
 * How long a decode is trusted. The cache is keyed by PATH, and paths get
 * rewritten — an agent regenerates /tmp/chart.png, you re-take a screenshot to
 * the same name — so without an expiry the transcript would show the first
 * version of that file for as long as the app stays open. Long enough that
 * scrolling a conversation doesn't re-decode anything; short enough that a
 * rewritten file corrects itself without a restart.
 */
const PREVIEW_TTL_MS = 5 * 60_000;

/** A cache entry that is still trusted, or undefined (miss or expired). */
function fresh(path: string): CachedPreview | undefined {
  const hit = previewCache.get(path);
  if (!hit) return undefined;
  if (Date.now() - hit.at > PREVIEW_TTL_MS) {
    previewCache.delete(path);
    return undefined;
  }
  return hit;
}

/** True once this path has an answer we still trust (a thumbnail OR a failure). */
export function previewSettled(path: string): boolean {
  return fresh(path) !== undefined;
}

function rememberPreview(path: string, dataUrl: string | null): void {
  // Oldest-first eviction — Map preserves insertion order.
  if (previewCache.size >= MAX_CACHED_PREVIEWS) {
    const oldest = previewCache.keys().next().value;
    if (oldest !== undefined) previewCache.delete(oldest);
  }
  previewCache.set(path, { dataUrl, at: Date.now() });
}

/** Only for tests, which need each case to start from a cold cache. */
export function __clearPreviewCache(): void {
  previewCache.clear();
}

export function cachedPreviews(paths: string[]): Record<string, string> {
  const out: Record<string, string> = {};
  for (const p of paths) {
    const hit = fresh(p)?.dataUrl;
    if (hit) out[p] = hit;
  }
  return out;
}

/**
 * Resolve thumbnails for a set of image paths. Returns path → data URL for
 * whatever resolved; a path that is missing, too big, or undecodable simply
 * never appears, so callers render nothing rather than a broken tile.
 */
export interface PreviewState {
  /** path → data URL, for whatever decoded. */
  previews: Record<string, string>;
  /** True once every path has an answer (a thumbnail or a recorded failure).
   *  Callers that fall back to something else must wait for this, or they'd
   *  flash the fallback for one frame on every successful load. */
  settled: boolean;
}

export function useImagePreviews(imagePaths: string[]): PreviewState {
  // Effect key: the identity of the image set, not the array reference (which
  // changes on every parent render). NUL-separated because a filename may
  // legally contain a newline, which would split one path into two.
  const key = imagePaths.join('\u0000');
  const [previews, setPreviews] = useState<Record<string, string>>(() =>
    cachedPreviews(imagePaths),
  );

  useEffect(() => {
    const paths = key ? key.split('\u0000') : [];
    let cancelled = false;
    // Paint whatever is already cached immediately; the rest fill in below.
    setPreviews(cachedPreviews(paths));

    const missing = paths.filter((p) => !previewSettled(p));
    if (missing.length === 0) return;

    const read = window.electronAPI?.readImagePreview;
    if (!read) {
      // No backend for previews (older web polyfill): chips stay iconic.
      missing.forEach((p) => rememberPreview(p, null));
      return;
    }
    void Promise.all(
      missing.map(async (p) => {
        try {
          const { dataUrl } = await read(p);
          rememberPreview(p, dataUrl);
        } catch {
          rememberPreview(p, null); // missing, too big, or undecodable
        }
      }),
    ).then(() => {
      if (!cancelled) setPreviews(cachedPreviews(paths));
    });

    return () => {
      cancelled = true;
    };
  }, [key]);

  return {
    previews,
    // Settled means "asked and answered", not "succeeded": a path the cache
    // holds as null (missing, too big, undecodable) is settled too.
    settled: imagePaths.every(previewSettled),
  };
}
