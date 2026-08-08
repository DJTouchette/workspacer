// The ASCII-whitespace trim set the caller-string seam uses on BOTH providers:
// space, tab, and the four ASCII vertical/form controls. It is spelled out — not
// delegated to `String.prototype.trim()` — because JS `.trim()` and Go's
// `strings.TrimSpace` are NOT the same function, and they disagree on exactly two
// code points that reach these paths as filenames and titles:
//
//   U+FEFF (ZWNBSP / BOM)  in ECMAScript's WhiteSpace production, NOT in Go's
//                          unicode.IsSpace (dropped from White_Space in 4.0.1) —
//                          `.trim()` strips it, strings.TrimSpace does not.
//   U+0085 (NEL)           unicode.IsSpace in Go, neither <USP> nor a JS
//                          LineTerminator — strings.TrimSpace strips it, `.trim()`
//                          does not.
//
// A BOM is exactly what a path pasted out of a Windows editor or a UTF-8-with-BOM
// file carries, so "$HOME default" versus "a filename the guard refuses" must not
// turn on which language's built-in happened to swallow the code point.
//
// TWIN: services/hub/cmd/brain/profiles.go `asciiWhitespace` (the Go constant the
// brain trims against). Same set as spawnCwd.ts TRIM_SET and searchService.ts
// TEXT_TRIM, factored here so the title/path blank checks share one predicate.
const NON_ASCII_WHITESPACE = /[^ \t\n\v\f\r]/;

// Leading/trailing ASCII-whitespace run, same set — the trim form of the check
// above, for callers that must AGREE with Go's `strings.Trim(s, asciiWhitespace)`
// twin (e.g. remoteTokens.ts normalizeScope vs authtoken.ParseScope). Delegating
// to `String.prototype.trim` reintroduces the exact BOM/NEL split this module
// exists to erase. Same spelling as spawnCwd.ts TRIM_SET.
const ASCII_WHITESPACE_EDGES = /^[ \t\n\v\f\r]+|[ \t\n\v\f\r]+$/g;

/** True when `s` is empty or contains only ASCII whitespace (see the set above). */
export function isAsciiBlank(s: string): boolean {
  return !NON_ASCII_WHITESPACE.test(s);
}

/** True when `s` holds at least one non-ASCII-whitespace character. */
export function hasNonBlankText(s: string): boolean {
  return NON_ASCII_WHITESPACE.test(s);
}

/**
 * Strip surrounding ASCII whitespace ONLY — never U+FEFF (BOM) or U+0085 (NEL),
 * which `String.prototype.trim` and Go's `strings.TrimSpace` disagree on. Use
 * this wherever a trimmed value must match a Go twin byte-for-byte.
 */
export function trimAsciiWhitespace(s: string): string {
  return s.replace(ASCII_WHITESPACE_EDGES, '');
}
