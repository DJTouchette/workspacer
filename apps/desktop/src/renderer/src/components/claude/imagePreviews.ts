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

/** `null` records a path we already failed on, so a missing or undecodable
 *  file isn't retried on every render. */
const previewCache = new Map<string, string | null>();
const MAX_CACHED_PREVIEWS = 64;

function rememberPreview(path: string, dataUrl: string | null): void {
  // Oldest-first eviction — Map preserves insertion order.
  if (previewCache.size >= MAX_CACHED_PREVIEWS) {
    const oldest = previewCache.keys().next().value;
    if (oldest !== undefined) previewCache.delete(oldest);
  }
  previewCache.set(path, dataUrl);
}

/** Only for tests, which need each case to start from a cold cache. */
export function __clearPreviewCache(): void {
  previewCache.clear();
}

export function cachedPreviews(paths: string[]): Record<string, string> {
  const out: Record<string, string> = {};
  for (const p of paths) {
    const hit = previewCache.get(p);
    if (hit) out[p] = hit;
  }
  return out;
}

/**
 * Resolve thumbnails for a set of image paths. Returns path → data URL for
 * whatever resolved; a path that is missing, too big, or undecodable simply
 * never appears, so callers render nothing rather than a broken tile.
 */
export function useImagePreviews(imagePaths: string[]): Record<string, string> {
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

    const missing = paths.filter((p) => !previewCache.has(p));
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

  return previews;
}
