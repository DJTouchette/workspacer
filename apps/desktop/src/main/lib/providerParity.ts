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

/** Compare by UTF-16 code unit — the JavaScript spelling of Go's `a < b`. */
export function byteCompare(a: string, b: string): number {
  return a < b ? -1 : a > b ? 1 : 0;
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
