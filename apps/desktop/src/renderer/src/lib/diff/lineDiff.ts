/**
 * Line-level aligner for the GUI diff viewer. The Edit/MultiEdit tool gives us
 * two opaque blocks (old_string / new_string), not a git-style hunk, so to show
 * an interleaved "inline" diff or an aligned "side-by-side" split we first run a
 * Longest-Common-Subsequence pass to recover which lines are shared, removed, or
 * added. `parseDiff.ts` handles already-unified text; this handles two strings.
 */

export type DiffRowKind = 'context' | 'del' | 'add';

export interface InlineRow {
  kind: DiffRowKind;
  text: string;
  /** 1-based line number in the old file, or null for added lines. */
  oldNo: number | null;
  /** 1-based line number in the new file, or null for removed lines. */
  newNo: number | null;
}

export interface SplitSide {
  text: string;
  no: number;
}

export interface SplitRow {
  /** Removed/old line for this row, or null when the new side has no pair. */
  left: SplitSide | null;
  /** Added/new line for this row, or null when the old side has no pair. */
  right: SplitSide | null;
  /** False for unchanged context rows (both sides identical). */
  changed: boolean;
}

type Op = { type: 'eq' | 'del' | 'add'; text: string };

/**
 * Beyond this many DP cells (old × new lines) we skip the O(n·m) LCS and fall
 * back to a whole-block replace — keeps a giant paste from janking the UI.
 */
const MAX_CELLS = 1_000_000;

/** Ordered edit script aligning `oldLines` to `newLines` via LCS. */
function diffOps(oldLines: string[], newLines: string[]): Op[] {
  const n = oldLines.length;
  const m = newLines.length;
  if (n * m > MAX_CELLS) {
    return [
      ...oldLines.map((text): Op => ({ type: 'del', text })),
      ...newLines.map((text): Op => ({ type: 'add', text })),
    ];
  }

  // dp[i][j] = LCS length of oldLines[i:] and newLines[j:].
  const dp: Uint32Array[] = [];
  for (let i = 0; i <= n; i++) dp.push(new Uint32Array(m + 1));
  for (let i = n - 1; i >= 0; i--) {
    for (let j = m - 1; j >= 0; j--) {
      dp[i][j] = oldLines[i] === newLines[j]
        ? dp[i + 1][j + 1] + 1
        : Math.max(dp[i + 1][j], dp[i][j + 1]);
    }
  }

  const ops: Op[] = [];
  let i = 0;
  let j = 0;
  while (i < n && j < m) {
    if (oldLines[i] === newLines[j]) {
      ops.push({ type: 'eq', text: oldLines[i] });
      i++; j++;
    } else if (dp[i + 1][j] >= dp[i][j + 1]) {
      ops.push({ type: 'del', text: oldLines[i] });
      i++;
    } else {
      ops.push({ type: 'add', text: newLines[j] });
      j++;
    }
  }
  while (i < n) ops.push({ type: 'del', text: oldLines[i++] });
  while (j < m) ops.push({ type: 'add', text: newLines[j++] });
  return ops;
}

/** Interleaved unified rows: context, removals, and additions in source order. */
export function inlineRows(oldLines: string[], newLines: string[]): InlineRow[] {
  const rows: InlineRow[] = [];
  let oldNo = 1;
  let newNo = 1;
  for (const op of diffOps(oldLines, newLines)) {
    if (op.type === 'eq') rows.push({ kind: 'context', text: op.text, oldNo: oldNo++, newNo: newNo++ });
    else if (op.type === 'del') rows.push({ kind: 'del', text: op.text, oldNo: oldNo++, newNo: null });
    else rows.push({ kind: 'add', text: op.text, oldNo: null, newNo: newNo++ });
  }
  return rows;
}

/**
 * Paired rows for side-by-side. A run of removals/additions is zipped so the
 * first removal lines up with the first addition; the longer side spills into
 * rows with a null on the other column.
 */
export function splitRows(oldLines: string[], newLines: string[]): SplitRow[] {
  const rows: SplitRow[] = [];
  let oldNo = 1;
  let newNo = 1;
  let delBuf: SplitSide[] = [];
  let addBuf: SplitSide[] = [];
  const flush = () => {
    const max = Math.max(delBuf.length, addBuf.length);
    for (let k = 0; k < max; k++) {
      rows.push({ left: delBuf[k] ?? null, right: addBuf[k] ?? null, changed: true });
    }
    delBuf = [];
    addBuf = [];
  };
  for (const op of diffOps(oldLines, newLines)) {
    if (op.type === 'eq') {
      flush();
      rows.push({ left: { text: op.text, no: oldNo++ }, right: { text: op.text, no: newNo++ }, changed: false });
    } else if (op.type === 'del') {
      delBuf.push({ text: op.text, no: oldNo++ });
    } else {
      addBuf.push({ text: op.text, no: newNo++ });
    }
  }
  flush();
  return rows;
}
