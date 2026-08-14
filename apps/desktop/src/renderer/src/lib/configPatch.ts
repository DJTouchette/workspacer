/**
 * Trimming a config save down to what actually changed.
 *
 * Callers write `save({ ui: { ...config.ui, sidebarWidth: px } })` — a leaf edit
 * that re-sends every sibling key from the renderer's snapshot. The main process
 * deep-merges, so the spread was never needed; what it does instead is make the
 * save carry a *stale copy of everything else*. Anything written behind the
 * renderer's back since it booted (an agent's `claude.seenModels`, a custom
 * theme created from the phone client, another window's edit) is then overwritten
 * by the older value — silently, and worst for the keys main replaces wholesale
 * rather than merging (`ui.customThemes`, `claude.budgets`).
 *
 * So the seam drops any key whose value the renderer isn't actually changing. A
 * stale sibling equals the snapshot it was spread from, so it never leaves the
 * renderer; a real edit always differs, so it always does. Callers keep their
 * spreads and stop being able to clobber with them.
 */

/**
 * Subtrees the main process replaces WHOLESALE instead of deep-merging: what
 * the caller sends is the entire truth, because that is the only way to delete
 * an entry. The list is shared with configService rather than restated here:
 * when it WAS restated the two copies drifted, `projects` was trimmed on its
 * way out of the renderer, and saving one project's icon wiped every other
 * project's identity. See main/shared/configWholesale. Diffing into them would be wrong twice over: it would
 * strip entries the caller meant to re-send, and a pure deletion would produce
 * an empty diff, so the delete would never happen.
 */
import { WHOLESALE_CONFIG_PATHS as WHOLESALE_PATHS } from '../../../main/shared/configWholesale';

/** Plain object — not null, not an array, not a Date/Map/class instance. */
function isPlainObject(v: unknown): v is Record<string, unknown> {
  return typeof v === 'object' && v !== null && Object.getPrototypeOf(v) === Object.prototype;
}

/** Structural equality, by the same rules the config travels under (JSON/IPC). */
export function sameConfigValue(a: unknown, b: unknown): boolean {
  if (a === b) return true;
  if (Array.isArray(a) || Array.isArray(b)) {
    if (!Array.isArray(a) || !Array.isArray(b) || a.length !== b.length) return false;
    return a.every((v, i) => sameConfigValue(v, b[i]));
  }
  if (isPlainObject(a) && isPlainObject(b)) {
    const ak = Object.keys(a);
    const bk = Object.keys(b);
    if (ak.length !== bk.length) return false;
    return ak.every(
      (k) => Object.prototype.hasOwnProperty.call(b, k) && sameConfigValue(a[k], b[k]),
    );
  }
  return false;
}

/**
 * The subset of `partial` that differs from `current`, with the same nesting.
 * `{}` when nothing changed — the caller should skip the write entirely.
 *
 * Objects recurse (so one changed leaf sends one changed leaf). Arrays and
 * scalars are all-or-nothing: an array IS the value as far as the merge is
 * concerned, so a changed array is sent whole.
 */
export function minimalConfigPatch<T extends Record<string, unknown>>(
  // Deliberately loose on the left and generic on the right: a patch is nested-
  // partial by nature, and `Partial<T>` only loosens the TOP level — typing the
  // snapshot as T would force every caller (and every test) to hand over a
  // complete subtree just to change one leaf.
  current: Record<string, unknown> | undefined,
  partial: T,
  path = '',
): T {
  const out: Record<string, unknown> = {};
  for (const key of Object.keys(partial ?? {})) {
    const here = path ? `${path}.${key}` : key;
    const next = (partial as Record<string, unknown>)[key];
    const prev = current?.[key];
    // Wholesale subtrees and arrays are single values: send them whole, or not
    // at all. Recursing would silently turn a deletion into a no-op.
    if (isPlainObject(next) && isPlainObject(prev) && !WHOLESALE_PATHS.has(here)) {
      const inner = minimalConfigPatch(prev, next, here);
      if (Object.keys(inner).length > 0) out[key] = inner;
      continue;
    }
    if (!sameConfigValue(prev, next)) out[key] = next;
  }
  return out as T;
}
