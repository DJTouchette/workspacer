/**
 * Parser for `git diff` unified output → structured hunks the diff viewer can
 * virtualize. Also computes intraline (word-level) emphasis ranges for paired
 * removed/added lines, GitHub-style.
 */

export type LineKind = 'context' | 'add' | 'del';

export interface DiffLine {
  kind: LineKind;
  /** Line content without the leading +/-/space marker. */
  text: string;
  oldNo: number | null;
  newNo: number | null;
  /**
   * Intraline emphasis range [start, end) in `text` — the chars that actually
   * changed when this line pairs with one on the other side of the hunk.
   */
  emph?: [number, number];
  /** True when git flagged "\ No newline at end of file" after this line. */
  noNewline?: boolean;
}

export interface DiffHunk {
  /** Full `@@ -a,b +c,d @@ context` header line. */
  header: string;
  lines: DiffLine[];
}

export interface ParsedDiff {
  hunks: DiffHunk[];
  /** True when git reported a binary diff instead of hunks. */
  binary: boolean;
  additions: number;
  deletions: number;
  /** Longest line length in chars — used to size the horizontal scroll area. */
  maxLineLength: number;
}

const HUNK_RE = /^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/;

/** Common prefix/suffix lengths between two strings (non-overlapping). */
function commonAffix(a: string, b: string): { prefix: number; suffix: number } {
  const max = Math.min(a.length, b.length);
  let prefix = 0;
  while (prefix < max && a[prefix] === b[prefix]) prefix++;
  let suffix = 0;
  while (suffix < max - prefix && a[a.length - 1 - suffix] === b[b.length - 1 - suffix]) suffix++;
  return { prefix, suffix };
}

/**
 * Mark the changed region on paired del/add lines: a run of deletions
 * immediately followed by an equally-positioned run of additions pairs
 * first-with-first, and each pair gets its common prefix/suffix trimmed.
 * Pairs that share nothing are left unmarked — a full-line highlight on top
 * of the row tint is just noise.
 */
function markIntraline(lines: DiffLine[]): void {
  let i = 0;
  while (i < lines.length) {
    if (lines[i].kind !== 'del') {
      i++;
      continue;
    }
    const delStart = i;
    while (i < lines.length && lines[i].kind === 'del') i++;
    const addStart = i;
    while (i < lines.length && lines[i].kind === 'add') i++;
    const pairs = Math.min(addStart - delStart, i - addStart);
    for (let p = 0; p < pairs; p++) {
      const del = lines[delStart + p];
      const add = lines[addStart + p];
      const { prefix, suffix } = commonAffix(del.text, add.text);
      if (prefix === 0 && suffix === 0) continue; // nothing in common — skip
      if (prefix + suffix >= del.text.length && prefix + suffix >= add.text.length) continue;
      del.emph = [prefix, del.text.length - suffix];
      add.emph = [prefix, add.text.length - suffix];
    }
  }
}

export function parseUnifiedDiff(text: string): ParsedDiff {
  const hunks: DiffHunk[] = [];
  let binary = false;
  let additions = 0;
  let deletions = 0;
  let maxLineLength = 0;

  let current: DiffHunk | null = null;
  let oldNo = 0;
  let newNo = 0;

  for (const raw of text.split('\n')) {
    const line = raw.endsWith('\r') ? raw.slice(0, -1) : raw;

    const m = HUNK_RE.exec(line);
    if (m) {
      if (current) markIntraline(current.lines);
      current = { header: line, lines: [] };
      hunks.push(current);
      oldNo = parseInt(m[1], 10);
      newNo = parseInt(m[2], 10);
      maxLineLength = Math.max(maxLineLength, line.length);
      continue;
    }

    if (!current) {
      // Still in the preamble (diff --git / index / ---/+++ headers).
      if (line.startsWith('Binary files ') || line === 'GIT binary patch') binary = true;
      continue;
    }

    if (line.startsWith('\\')) {
      // "\ No newline at end of file" annotates the previous line.
      const prev = current.lines[current.lines.length - 1];
      if (prev) prev.noNewline = true;
      continue;
    }

    const marker = line[0];
    const content = line.slice(1);
    maxLineLength = Math.max(maxLineLength, content.length);

    if (marker === '+') {
      current.lines.push({ kind: 'add', text: content, oldNo: null, newNo: newNo++ });
      additions++;
    } else if (marker === '-') {
      current.lines.push({ kind: 'del', text: content, oldNo: oldNo++, newNo: null });
      deletions++;
    } else if (marker === ' ') {
      current.lines.push({ kind: 'context', text: content, oldNo: oldNo++, newNo: newNo++ });
    } else if (line === '') {
      // Trailing-newline artifact of split(), or a blank between file
      // sections — blank *context* lines always carry the space marker.
      continue;
    } else {
      // A new file header inside a multi-file diff ("diff --git ...") ends the
      // current hunk run; per-file requests never hit this, but stay safe.
      if (line.startsWith('diff ')) {
        markIntraline(current.lines);
        current = null;
      }
    }
  }
  if (current) markIntraline(current.lines);

  return { hunks, binary, additions, deletions, maxLineLength };
}
