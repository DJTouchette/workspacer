/**
 * File-name / slug utilities shared across libraryService, layoutService, and
 * sessionService. Each variant is parameterized so that on-disk filenames are
 * byte-identical to the originals — changing any option here will break
 * existing persisted files.
 *
 * Use the named wrappers below (not `slug` directly) at call sites.
 */

export interface SlugOpts {
  /**
   * Maximum byte-length of the returned string.
   * undefined = no limit (libraryService behaviour).
   */
  maxLen?: number;

  /**
   * Fallback returned when the slugified string is empty.
   * undefined = return '' on empty (sessionService behaviour).
   */
  fallback?: string;

  /**
   * Whether to strip leading/trailing hyphens from the result.
   * libraryService and layoutService do this; sessionService does not.
   */
  trimDashes?: boolean;

  /**
   * Which regex variant to use for allowed characters.
   *
   * 'library'  → /[^a-z0-9-_]+/g  (libraryService: collapses runs,
   *                                  -_ order matches original)
   * 'layout'   → /[^a-z0-9_-]/g   (layoutService/sessionService: single-char
   *                                  replace followed by /-+/g dedup)
   */
  charsetVariant?: 'library' | 'layout';
}

/**
 * Core slug implementation. Call the named wrappers below at production
 * call sites to guarantee byte-identical output.
 */
export function slug(input: string, opts: SlugOpts = {}): string {
  const { maxLen, fallback, trimDashes = false, charsetVariant = 'layout' } = opts;

  let out = (input || '').toLowerCase();

  if (charsetVariant === 'library') {
    // libraryService: collapse runs of bad chars into a single '-' in one pass
    out = out.replace(/[^a-z0-9-_]+/g, '-');
    if (trimDashes) out = out.replace(/^-+|-+$/g, '');
  } else {
    // layoutService / sessionService: single-char replace then deduplicate
    out = out.replace(/[^a-z0-9_-]/g, '-').replace(/-+/g, '-');
    if (trimDashes) out = out.replace(/^-|-$/g, '');
  }

  if (maxLen !== undefined) out = out.substring(0, maxLen);

  if (!out && fallback !== undefined) return fallback;
  return out;
}

// ── Named wrappers — one per call site ───────────────────────────────────────

/**
 * libraryService variant.
 * Rules: collapse bad-char runs → '-', trim leading/trailing dashes,
 * no max length, fallback = 'item'.
 */
export function slugLibrary(s: string): string {
  return slug(s, {
    charsetVariant: 'library',
    trimDashes: true,
    fallback: 'item',
  });
}

/**
 * layoutService variant.
 * Rules: single-char replace, dedup dashes, trim leading/trailing dashes,
 * max 64, fallback = 'layout'.
 */
export function slugLayout(name: string): string {
  return slug(name, {
    charsetVariant: 'layout',
    trimDashes: true,
    maxLen: 64,
    fallback: 'layout',
  });
}

/**
 * sessionService / sanitizeFilename variant.
 * Rules: single-char replace, dedup dashes, NO dash trimming, max 64,
 * no fallback (empty input → empty output).
 */
export function slugSession(name: string): string {
  return slug(name, {
    charsetVariant: 'layout',
    trimDashes: false,
    maxLen: 64,
  });
}
