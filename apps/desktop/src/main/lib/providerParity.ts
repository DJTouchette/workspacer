/**
 * Small primitives that exist so a list this side comes back the same as the
 * same list from the Go brain.
 *
 * Both providers answer the same bus methods — the brain by default under
 * DELEGATE_CATALOG_TO_BRAIN, this process when delegation is off — and every one
 * of these replaced a JavaScript idiom whose Go twin behaves differently:
 *
 *   `localeCompare`      → Go sorts strings by raw bytes, so layouts.list,
 *                          sessions.list, library.list, fs.listDir and
 *                          fs.listEntries came back in different orders
 *                          depending on which provider answered. Worse:
 *                          localeCompare is a METHOD, so a YAML scalar that is
 *                          not a string (`createdAt: 5`, or an unquoted ISO date,
 *                          which js-yaml 4 parses to a Date) threw inside the
 *                          comparator, and the function-level catch turned that
 *                          into an EMPTY LIST — every well-formed row vanished
 *                          too, while the brain listed them all.
 *   `String#replace`     → replaces the FIRST occurrence anywhere, not a suffix:
 *                          `'a.jsonl.b.jsonl'.replace('.jsonl','')` is
 *                          `'a.b.jsonl'` where Go's TrimSuffix gives
 *                          `'a.jsonl.b'`, so the two providers offered different
 *                          resume ids for one transcript — and
 *                          `.jsonlagent-x.jsonl` became `agent-x.jsonl`, which
 *                          then matched the subagent filter and dropped a row.
 */

/**
 * Compare by UTF-8 BYTE — the JavaScript spelling of Go's `a < b`.
 *
 * `a < b` on JavaScript strings is UTF-16 CODE UNIT order, which is not the same
 * relation. A code point above U+FFFF is a surrogate pair (0xD800-0xDBFF lead),
 * so it sorts BELOW every character in 0xE000-0xFFFF under `<` — while its UTF-8
 * encoding starts 0xF0-0xF4 and sorts ABOVE the 0xE0-0xEF of the BMP. One real
 * directory is enough to see it: for the entries `a`, U+1F600 + "-notes",
 * U+FDFD + "-x" and U+FF04 + "-budget", Go puts the emoji LAST and `<` puts it
 * SECOND. That hits fs.listDir, fs.listEntries, library.list titles,
 * layouts.list and sessions.list — every list whose ORDER is what the picker
 * shows and what "first" means in it.
 *
 * Comparing the UTF-8 encodings directly is the definition, not an
 * approximation (it is equivalent to comparing by code point); the buffers are
 * used because they are literally what Go compares.
 */
export function byteCompare(a: string, b: string): number {
  if (a === b) return 0;
  return Buffer.compare(Buffer.from(a, 'utf8'), Buffer.from(b, 'utf8'));
}

/** Go's `str(v any) string`: a string stays, anything else (number, Date, null,
 *  object — all of which a YAML scalar can be) becomes "". Coercing rather than
 *  stringifying is deliberate: it is what the Go twin does, so the two sides
 *  order an odd row identically instead of merely both surviving it. */
export function asString(v: unknown): string {
  return typeof v === 'string' ? v : '';
}

/** Go's `strings.TrimSuffix`: removes `suffix` only when the string ENDS with it. */
export function trimSuffix(s: string, suffix: string): string {
  return suffix && s.endsWith(suffix) ? s.slice(0, s.length - suffix.length) : s;
}

/** `trimSuffix` with an ASCII-case-insensitive match, the twin of the Go brain's
 *  `trimMDSuffix` and of the `/\.md$/i` this codebase already used for library
 *  ids. Split out so the two sides mint the same id for `readme.Md`. */
export function trimSuffixFold(s: string, suffix: string): string {
  return s.length >= suffix.length &&
    s.slice(s.length - suffix.length).toLowerCase() === suffix.toLowerCase()
    ? s.slice(0, s.length - suffix.length)
    : s;
}
