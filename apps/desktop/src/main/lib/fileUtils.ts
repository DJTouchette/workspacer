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
 * Lowercase A-Z and leave every other character alone.
 *
 * `String.prototype.toLowerCase` is NOT interchangeable with Go's
 * `strings.ToLower`, and this is the one place in the repo where the difference
 * decides a FILENAME. JS applies the full Unicode SPECIAL CASING map, which can
 * make a string LONGER; Go does a per-rune simple fold. U+0130 (İ) becomes
 * 'i' + U+0307 COMBINING DOT ABOVE here — and the combining mark is then a bad
 * character and becomes a '-' — while Go produces a single 'i'. So a layout
 * named 'aİb' was written as ai-b.yaml by this side and aib.yaml by the brain,
 * into the same store: the item was invisible to the other provider's list, and
 * remove() re-slugged and unlinked a filename that was never written, so it
 * could not be deleted either. (Which side answers depends on
 * DELEGATE_CATALOG_TO_BRAIN, so this was live in the default configuration.)
 *
 * Every character the two implementations can disagree about is non-ASCII, and
 * every non-ASCII character is replaced by '-' below anyway, so narrowing the
 * fold to ASCII costs nothing a caller could want and removes the whole class —
 * including the cases nobody has enumerated yet. The twin is cmd/brain/slug.go
 * `lowerASCII`; contracts/filename-slug-cases.json holds both to it.
 */
function lowerAscii(input: string): string {
  let out = '';
  for (let i = 0; i < input.length; i++) {
    const code = input.charCodeAt(i);
    out += code >= 65 && code <= 90 ? String.fromCharCode(code + 32) : input[i];
  }
  return out;
}

/**
 * Core slug implementation. Call the named wrappers below at production
 * call sites to guarantee byte-identical output.
 */
export function slug(input: string, opts: SlugOpts = {}): string {
  const { maxLen, fallback, trimDashes = false, charsetVariant = 'layout' } = opts;

  let out = lowerAscii(input || '');

  const trimRe = charsetVariant === 'library' ? /^-+|-+$/g : /^-|-$/g;
  if (charsetVariant === 'library') {
    // libraryService: collapse runs of bad chars into a single '-' in one pass
    out = out.replace(/[^a-z0-9-_]+/g, '-');
  } else {
    // layoutService / sessionService: single-char replace then deduplicate
    out = out.replace(/[^a-z0-9_-]/g, '-').replace(/-+/g, '-');
  }
  if (trimDashes) out = out.replace(trimRe, '');

  if (maxLen !== undefined) out = out.substring(0, maxLen);

  // Re-trim after truncation: a cut can land on a '-' boundary and reintroduce
  // a trailing dash. Without this, slug() is not idempotent
  // (slug(slug(x)) !== slug(x)), which breaks any consumer that re-slugs a
  // stored id — e.g. layoutService.remove re-slugged the id and unlinked a
  // filename that didn't match what save() wrote, so the layout was undeletable.
  if (trimDashes) out = out.replace(trimRe, '');

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
