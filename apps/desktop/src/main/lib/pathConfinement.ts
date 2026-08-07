/**
 * Filesystem path confinement — the desktop copy of the shared containment rule.
 *
 * Extracted out of services/hubCapabilities.ts so the cross-language contract
 * (contracts/path-containment-cases.json) can pin the predicate itself rather
 * than reaching it through a capability handler. Two other copies implement the
 * same algorithm: the Go brain (cmd/brain/fsguard.go), which is the DEFAULT
 * answerer for the fs.* and library.* methods under DELEGATE_CATALOG_TO_BRAIN,
 * and the bus's per-plugin grant confinement (internal/bus/policy.go). The three
 * copies must agree;
 * the fixture is what keeps them agreeing.
 *
 * The rule, in one line: canonicalize the caller's path per component, require
 * the result to sit at or inside one of the allowed roots, then refuse it anyway
 * if it is a credential.
 *
 * Two properties are load-bearing and easy to lose in a refactor:
 *
 *   1. NO TILDE EXPANSION, anywhere. '~' is an ordinary filename here. The brain
 *      used to expand it at its guard call sites while this side did not, so the
 *      same string was allowed by one provider and denied by the other.
 *
 *   2. CANONICALIZATION IS A PER-COMPONENT WALK, never a textual clean.
 *      path.resolve / path.normalize / filepath.Clean collapse 'link/..' before
 *      any symlink is read, so the guard would check <root>/x while the handler
 *      opened <somewhere-else>/x. Only fs.lstatSync and fs.readlinkSync are used
 *      below, and every caller must hand the RETURNED canonical path to the
 *      filesystem operation — check-path and opened-path cannot be allowed to
 *      differ.
 */

import * as fs from 'fs';
import * as path from 'path';
import { getConfigDir } from '../services/configService';

/** Cap on symlinks followed in one walk. The walk is hand-rolled (the platform
 *  realpath is what we are deliberately not using), so the ELOOP protection has
 *  to be hand-rolled too or a two-link cycle spins forever. */
const MAX_LINK_HOPS = 40;

const WIN32 = process.platform === 'win32';
/** Splitting alphabet. Windows accepts both slashes; POSIX only '/'. */
const SEP_RE = WIN32 ? /[\\/]+/ : /\/+/;

function isSepChar(ch: string): boolean {
  return ch === '/' || (WIN32 && ch === '\\');
}

interface SplitPath {
  /** '/' on POSIX; 'C:\' or '\\server\share\' on Windows. Always ends in a separator. */
  volume: string;
  /** The component region, split on separator runs, with '' and '.' discarded.
   *  '..' is NOT discarded — the walk handles it. */
  comps: string[];
}

function splitComponents(region: string): string[] {
  return region.split(SEP_RE).filter((c) => c !== '' && c !== '.');
}

/**
 * Split an ABSOLUTE path into its volume prefix and components, or null when it
 * is not absolute. A leading '~' makes a path relative, not special: nothing in
 * this module expands it.
 */
function splitAbsolute(s: string): SplitPath | null {
  if (WIN32) {
    const drive = /^([A-Za-z]:)[\\/]/.exec(s);
    if (drive) return { volume: `${drive[1]}\\`, comps: splitComponents(s.slice(2)) };
    const unc = /^[\\/]{2}([^\\/]+)[\\/]+([^\\/]+)(?:[\\/]|$)/.exec(s);
    if (unc) {
      return {
        volume: `\\\\${unc[1]}\\${unc[2]}\\`,
        comps: splitComponents(s.slice(unc[0].length)),
      };
    }
    return null;
  }
  if (!s.startsWith('/')) return null;
  return { volume: '/', comps: splitComponents(s.slice(1)) };
}

/** Append one component to a path this module built. Pure string arithmetic —
 *  no normalization, so it can be handed a component that is a '..' target of a
 *  symlink without collapsing anything. */
function appendComponent(base: string, name: string): string {
  return isSepChar(base.slice(-1)) ? base + name : base + path.sep + name;
}

/** The textual parent of an ALREADY-RESOLVED path, clamped at the volume. Safe
 *  precisely because `base` is symlink-free by construction, so its textual
 *  parent is its real parent — the property a whole-path clean destroys. */
function parentOf(base: string, volume: string): string {
  if (base === volume) return volume;
  const idx = base.lastIndexOf(path.sep);
  if (idx < 0) return volume;
  const parent = base.slice(0, idx);
  if (parent.length < volume.length || parent === '') return volume;
  return parent;
}

/**
 * Canonicalize `target`: absolute, with '.', '..' and symlinks resolved by
 * walking one component at a time against the already-resolved prefix.
 *
 * Throws — and every caller treats a throw as a denial — when the target is
 * empty/whitespace-only, is not absolute, hits a symlink cycle, or produces any
 * lstat error other than ENOENT. ENOENT is the one continue-condition: a file
 * fs.write is about to create does not exist yet, and the remaining components
 * are appended (not switched to a lexical mode — a later '..' can pop back onto
 * a path that does exist, and the walk resumes resolving there).
 */
export function canonicalizePath(target: string): string {
  if (target.trim() === '') throw new Error('path is empty');
  // Everything past the emptiness test uses the ORIGINAL string: a filename may
  // legitimately begin or end with a space.
  const split = splitAbsolute(target);
  if (!split) throw new Error('path is not absolute');

  const { volume } = split;
  const queue = [...split.comps];
  let resolved = volume;
  let hops = 0;

  while (queue.length > 0) {
    const c = queue.shift() as string;
    if (c === '..') {
      resolved = parentOf(resolved, volume);
      continue;
    }
    const next = appendComponent(resolved, c);
    let st: fs.Stats;
    try {
      st = fs.lstatSync(next);
    } catch (err) {
      // ENOENT — and ONLY ENOENT — keeps walking. ENOTDIR, EACCES, EPERM,
      // ELOOP, ENAMETOOLONG and anything unrecognised fail the whole call:
      // swallowing them into "contained" is an escape, and swallowing them into
      // "not contained, keep looking" turns the guard into an existence oracle.
      if ((err as NodeJS.ErrnoException).code === 'ENOENT') {
        resolved = next;
        continue;
      }
      throw err;
    }
    if (st.isSymbolicLink()) {
      hops += 1;
      if (hops > MAX_LINK_HOPS) throw new Error('too many levels of symbolic links');
      const link = fs.readlinkSync(next); // any error here fails closed
      const linkSplit = splitAbsolute(link);
      if (linkSplit) {
        resolved = linkSplit.volume;
        queue.unshift(...linkSplit.comps);
      } else {
        // Relative link: interpreted against the directory that CONTAINS the
        // link, which is `resolved` — so `resolved` deliberately does not
        // advance to `next` in this branch.
        queue.unshift(...splitComponents(link));
      }
      continue;
    }
    resolved = next;
  }
  return resolved;
}

/**
 * Canonical form of an allowed root, or null to DISCARD it.
 *
 * A discarded root is skipped and nothing else: one stale session snapshot with
 * an empty cwd must not disable every other root, and must not silently become
 * the process working directory either (path.resolve('') returns exactly that).
 * A root that does not exist yet still canonicalizes — the config-dir stores are
 * created lazily and have to be comparable before anything has been saved.
 */
export function canonicalRoot(root: string): string | null {
  if (root.trim() === '') return null;
  if (!splitAbsolute(root)) return null; // relative, or a '~' that nobody expands
  try {
    return canonicalizePath(root);
  } catch {
    return null;
  }
}

/** Containment between two ALREADY-canonical paths. */
function containsCanonical(canonicalRootPath: string, canonicalTarget: string): boolean {
  if (canonicalTarget === canonicalRootPath) return true;
  // A root that canonicalized to a volume prefix ('/', 'C:\', '\\srv\share\')
  // already ends in a separator and contains everything below it. Appending a
  // second separator would produce '//' and match nothing — that is not
  // fail-closed, it is simply wrong containment.
  if (canonicalRootPath.endsWith(path.sep)) return canonicalTarget.startsWith(canonicalRootPath);
  // The separator is mandatory: without it, root '/srv/fo' contains '/srv/foo'.
  return canonicalTarget.startsWith(canonicalRootPath + path.sep);
}

/** True when an already-canonicalized `canonicalTarget` sits at or inside `root`.
 *  A root that cannot be canonicalized is discarded, i.e. contains nothing. */
export function isWithin(canonicalTarget: string, root: string): boolean {
  const cr = canonicalRoot(root);
  if (cr === null) return false;
  return containsCanonical(cr, canonicalTarget);
}

/** True when an already-canonicalized `canonicalTarget` sits at or inside one of
 *  `roots`. An empty (or entirely discarded) allow-list means NOTHING is
 *  allowed; it never means unrestricted. */
export function pathWithinRoots(roots: string[], canonicalTarget: string): boolean {
  return roots.some((r) => isWithin(canonicalTarget, r));
}

/** The config-dir subtrees the web/remote UI legitimately reads and writes.
 *  Mirrors configStoreRoots() in the Go brain (cmd/brain/fsguard.go) — the brain
 *  is the DEFAULT answerer for fs.*, so the two lists have to be the same list. */
export function configStoreRoots(): string[] {
  const cfg = getConfigDir();
  return ['library', 'layouts', 'sessions'].map((store) => path.join(cfg, store));
}

/** Credential files denied by name wherever they resolve — a root is only as
 *  narrow as the cwds an agent runs in, and `workspacer plugin dev <dir>` puts a
 *  .bus-token inside an ordinary project. Same list as the brain's. */
export const SECRET_BASENAMES = new Set(['.bus-token', '.settings.json']);

/** Last component of an already-canonical path; '' for a bare volume prefix. */
function canonicalBasename(canonicalTarget: string): string {
  return canonicalTarget.slice(canonicalTarget.lastIndexOf(path.sep) + 1);
}

/**
 * Second gate, applied to every guarded path after the roots check — reads AND
 * writes, because handing a token out is a privilege promotion and overwriting
 * one is a denial of service on the whole bus.
 *
 * Narrowing the config root is not enough on its own: an agent cwd is a root
 * too, so a user who spawns an agent in `$HOME` (or `~/.config`) re-admits the
 * entire config dir through THAT root. Anything landing in the config dir
 * outside library/ layouts/ sessions/ is therefore refused here regardless of
 * which root allowed it — config.yaml, remote-token, tokens.json,
 * remote-server.json, vapid.json, workspacer.db, the Electron cookie jar,
 * plugins/** and the dir itself. The Go twin (fsguard.go) denies the same whole
 * remainder; the two are supposed to stay word for word.
 *
 * Order matters: the basename check runs FIRST and unconditionally, so a
 * credential name inside a store carve-out is still refused.
 */
export function isSecretPath(canonicalTarget: string): boolean {
  if (SECRET_BASENAMES.has(canonicalBasename(canonicalTarget))) return true;
  const cfg = canonicalRoot(getConfigDir());
  // An unverifiable config dir means we cannot prove the target is outside it.
  if (cfg === null) return true;
  if (!containsCanonical(cfg, canonicalTarget)) return false;
  for (const store of configStoreRoots()) {
    const sr = canonicalRoot(store);
    if (sr !== null && containsCanonical(sr, canonicalTarget)) return false;
  }
  return true;
}

/**
 * Reject a call whose path escapes the allowed roots or lands on a credential
 * file; otherwise return the CANONICAL path, which the caller must be the one to
 * hand to the filesystem operation (re-passing the caller's raw string is the
 * check-path/opened-path split this whole module exists to close).
 *
 * One message for all three refusal reasons, matching the brain word for word:
 * it goes to a remote caller, and confirming where a denied path landed — or
 * that it hit something worth protecting — is a probe primitive.
 */
export function assertPathAllowed(cap: string, target: string, roots: string[]): string {
  const refuse = (): never => {
    throw new Error(`${cap}: path is outside the allowed workspace (agent cwds + config stores)`);
  };
  let canonical: string;
  try {
    // Exactly one resolution per request: the roots test and the secret gate
    // both read this value.
    canonical = canonicalizePath(target);
  } catch {
    return refuse(); // unverifiable → deny
  }
  if (!pathWithinRoots(roots, canonical)) return refuse();
  if (isSecretPath(canonical)) return refuse();
  return canonical;
}
