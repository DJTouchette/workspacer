// ── Long-paste collapsing ──
//
// Dumping a 400-line log into the composer buries the sentence you wrote around
// it and makes the box unusable. So a large paste is replaced by a marker —
// `[Pasted text #1 +412 lines]` — and the real text is held aside until send,
// when every marker is expanded back in place. The agent gets the full paste;
// the composer stays readable and editable around it.
//
// The marker is plain text on purpose: it survives arrow keys, selection, and
// backspace like any other input (delete it and the block goes with it), which
// a decorated widget in a <textarea> could not.

/** A paste at or above either threshold collapses. */
export const PASTE_COLLAPSE_LINES = 6;
export const PASTE_COLLAPSE_CHARS = 800;

export interface PastedBlock {
  id: number;
  text: string;
}

export function shouldCollapsePaste(text: string): boolean {
  return countLines(text) >= PASTE_COLLAPSE_LINES || text.length >= PASTE_COLLAPSE_CHARS;
}

function countLines(text: string): number {
  return text.split('\n').length;
}

/**
 * The marker shown in the composer. Measured in lines, except for a single
 * huge line (a minified blob, a base64 string) where "+1 lines" would be
 * useless and characters are the honest unit.
 */
export function pastePlaceholder(id: number, text: string): string {
  const lines = countLines(text);
  const measure = lines > 1 ? `+${lines} lines` : `+${text.length.toLocaleString()} chars`;
  return `[Pasted text #${id} ${measure}]`;
}

/** Matches any marker, capturing its id. The measure is not trusted on the way
 *  back — only the id identifies the block. */
const PLACEHOLDER_RE = /\[Pasted text #(\d+) \+[^\]]*\]/g;

/**
 * Swap every marker back for the text it stands for. Unknown ids are left
 * exactly as written — the user may simply have typed something that looks like
 * a marker, and inventing an expansion for it would be worse than passing it
 * through.
 */
export function expandPastedText(input: string, blocks: Map<number, string>): string {
  return input.replace(PLACEHOLDER_RE, (match, id: string) => blocks.get(Number(id)) ?? match);
}

/** Ids still referenced by the composer text. */
export function referencedBlockIds(input: string): Set<number> {
  const ids = new Set<number>();
  for (const m of input.matchAll(PLACEHOLDER_RE)) ids.add(Number(m[1]));
  return ids;
}

/**
 * Forget blocks whose marker is no longer in the composer (deleted, or cleared
 * by a send). Keeps the held text from outliving the draft that referenced it.
 */
export function pruneBlocks(input: string, blocks: Map<number, string>): void {
  const live = referencedBlockIds(input);
  for (const id of [...blocks.keys()]) {
    if (!live.has(id)) blocks.delete(id);
  }
}

/** Insert `insert` over [start, end) of `value` — the caret-aware splice a
 *  paste performs. Returns the new value and where the caret lands. */
export function spliceAtSelection(
  value: string,
  start: number,
  end: number,
  insert: string,
): { value: string; caret: number } {
  return {
    value: value.slice(0, start) + insert + value.slice(end),
    caret: start + insert.length,
  };
}
