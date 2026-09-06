/**
 * The containment guard behind a response card's `view_diff` action.
 *
 * A card's path is model-authored text. `FileLink`'s `resolveWithCwd` is a
 * string join and nothing more — correct for a path a TOOL CALL reported (the
 * agent already had that file open), wrong as the only check on a path a card
 * merely claims. So the claim is decided here, in main, where symlinks can be
 * read: `../` that follows a symlink applies to the LINK TARGET, and no
 * renderer-side string comparison can know that.
 *
 * This adds no new rule. It calls `lib/pathConfinement` — the desktop copy of
 * the cross-language containment predicate pinned by
 * `contracts/path-containment-cases.json` — with the OWNING pane's cwd as the
 * single allowed root, and then applies the same second gate every `fs.*`
 * caller gets (`isSecretPath`: credential basenames, `.git`, provider config).
 *
 * It returns the CANONICAL path, which is what the caller must hand downstream.
 * Re-passing the raw string is exactly the check-path/opened-path split
 * pathConfinement exists to close.
 */
import fs from 'fs';
import * as path from 'path';
import { canonicalizePath, canonicalRoot, isSecretPath, isWithin } from '../lib/pathConfinement';

export type HtmlCardPathResult = { ok: true; path: string } | { ok: false; error: string };

/**
 * Resolve `target` for a card owned by a pane whose cwd is `cwd`.
 *
 * `cwd` comes from the host, never from the card. A relative target is joined
 * to it TEXTUALLY first and canonicalized after, so the join cannot be used to
 * skip the walk; an absolute target is canonicalized as given and then has to
 * land inside the same root anyway.
 */
export function resolveHtmlCardPath(target: unknown, cwd: unknown): HtmlCardPathResult {
  if (typeof target !== 'string' || !target.trim()) return { ok: false, error: 'no file given' };
  if (typeof cwd !== 'string' || !cwd.trim())
    return { ok: false, error: 'this card has no project' };
  // No tilde expansion, here or anywhere else in the confinement story: '~' is
  // an ordinary filename, and a layer that expanded it would disagree with
  // every other copy of the rule.
  const root = canonicalRoot(cwd);
  if (!root) return { ok: false, error: 'this project directory could not be resolved' };

  const joined = path.isAbsolute(target) ? target : `${cwd}${path.sep}${target}`;
  let canonical: string;
  try {
    canonical = canonicalizePath(joined);
  } catch {
    return { ok: false, error: 'that path could not be resolved' };
  }
  if (!isWithin(canonical, root)) return { ok: false, error: 'that file is outside this project' };
  if (isSecretPath(canonical))
    return { ok: false, error: 'that file holds credentials or agent configuration' };
  // A card can name a file that was deleted since it was written. Say so rather
  // than opening an empty diff on a path that is gone.
  try {
    if (!fs.statSync(canonical).isFile()) return { ok: false, error: 'that path is not a file' };
  } catch {
    return { ok: false, error: 'that file no longer exists' };
  }
  return { ok: true, path: canonical };
}

/** Read through a pinned descriptor, then compare its inode with the contained
 * live path. No downstream pane re-resolves a basename or reopens this path. */
export function readHtmlCardFile(target: string, cwd: string): { path: string; text: string } {
  const resolved = resolveHtmlCardPath(target, cwd);
  if (!resolved.ok) throw new Error(resolved.error);
  const fd = fs.openSync(
    resolved.path,
    fs.constants.O_RDONLY | fs.constants.O_NOFOLLOW | fs.constants.O_NONBLOCK,
  );
  try {
    const stat = fs.fstatSync(fd);
    const checked = resolveHtmlCardPath(target, cwd);
    if (!checked.ok || checked.path !== resolved.path) throw new Error('File moved during read');
    const live = fs.statSync(checked.path);
    if (stat.dev !== live.dev || stat.ino !== live.ino || !stat.isFile())
      throw new Error('File changed during read');
    if (stat.size > 256 * 1024) throw new Error('Card diffs are limited to 256 KiB per file');
    const buffer = Buffer.alloc(256 * 1024 + 1);
    const count = fs.readSync(fd, buffer, 0, buffer.length, 0);
    if (count > 256 * 1024 || buffer.subarray(0, count).includes(0))
      throw new Error('File is too large or binary');
    return { path: resolved.path, text: buffer.subarray(0, count).toString('utf8') };
  } finally {
    fs.closeSync(fd);
  }
}
