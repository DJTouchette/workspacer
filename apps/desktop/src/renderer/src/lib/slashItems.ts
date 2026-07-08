/**
 * Data + filtering for the composer's "/" command picker.
 *
 * When a Claude agent runs on the headless `stream` transport there is no TUI to
 * type `/foo` into and get Claude Code's own slash/skill menu — the text would
 * land as a literal prompt (see ComposerControls' `/model` note). So the GUI
 * composer surfaces its own picker: type `/`, get a filtered list of skills and
 * reusable prompts, pick one to drop its content into the composer. The items
 * come from the existing Library (`library.list`), so this needs no new backend.
 */

/** A single "/" picker entry — a projection of a LibraryItem down to what the
 *  picker renders and filters on. `id` maps back to the source LibraryItem. */
export interface SlashItem {
  id: string;
  /** The name shown after the leading "/", also the primary match target. */
  label: string;
  /** One-line description, shown under the label and also matched. */
  hint?: string;
  /** Source kind ('skill' | 'prompt' | …), rendered as a small badge. */
  kind?: string;
}

/**
 * Filter + rank picker items for `query` (the text after the leading "/").
 * Case-insensitive. Label prefix matches rank ahead of other label/hint
 * substring matches; an empty query returns the head of the list. Ranking is
 * stable within each tier, and the result is capped at `limit`.
 */
export function filterSlashItems(items: SlashItem[], query: string, limit = 8): SlashItem[] {
  const q = query.trim().toLowerCase();
  if (!q) return items.slice(0, limit);
  const prefix: SlashItem[] = [];
  const substr: SlashItem[] = [];
  for (const it of items) {
    const label = it.label.toLowerCase();
    if (label.startsWith(q)) prefix.push(it);
    else if (label.includes(q) || it.hint?.toLowerCase().includes(q)) substr.push(it);
  }
  return [...prefix, ...substr].slice(0, limit);
}
