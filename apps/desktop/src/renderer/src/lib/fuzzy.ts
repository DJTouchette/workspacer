/**
 * Tiny fzf-style fuzzy matcher. A query matches when its characters appear in
 * the text in order (subsequence); the score rewards word-boundary hits,
 * consecutive runs, and exact substrings, and gently penalizes gaps and long
 * targets so tighter matches rank first.
 *
 * `fuzzyScore` returns `-Infinity` for a non-match, so callers can filter with
 * `score > -Infinity` and sort descending for ranked results.
 */

const WORD_SEPARATORS = new Set([' ', '-', '_', '/', '.', ':']);

export function fuzzyScore(query: string, text: string): number {
  const q = query.toLowerCase();
  const t = text.toLowerCase();
  if (!q) return 0;
  if (!t) return -Infinity;

  let score = 0;
  let ti = 0;
  let prevMatch = -2;
  for (let qi = 0; qi < q.length; qi++) {
    const found = t.indexOf(q[qi], ti);
    if (found === -1) return -Infinity;
    // Word-boundary bonus: start of text or right after a separator.
    if (found === 0 || WORD_SEPARATORS.has(t[found - 1])) score += 3;
    // Consecutive-run bonus.
    if (found === prevMatch + 1) score += 2;
    // Gap penalty — skipped characters cost a little.
    score -= (found - ti) * 0.1;
    prevMatch = found;
    ti = found + 1;
  }
  // Exact-substring bonus so literal matches always beat scattered ones.
  if (t.includes(q)) score += q.length * 2;
  // Slightly prefer shorter targets (less unmatched tail).
  score -= (t.length - q.length) * 0.02;
  return score;
}

/** Best score of `query` across several candidate strings (label + keywords,
 *  aliases, …). `-Infinity` when none match. */
export function fuzzyScoreAny(query: string, candidates: string[]): number {
  let best = -Infinity;
  for (const c of candidates) {
    const s = fuzzyScore(query, c);
    if (s > best) best = s;
  }
  return best;
}
