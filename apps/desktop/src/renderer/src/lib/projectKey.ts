/**
 * The stable config key for a workspace directory.
 *
 * config.yaml has two per-directory maps — `scripts` (the top-bar script
 * buttons) and `widgets` (a project's widget board) — and both must key the same
 * cwd identically, or a repo ends up with its scripts under one spelling and its
 * board under another. Windows hands us backslashes, agents can carry a trailing
 * separator, so both are normalized away.
 *
 * Note this is normalization, not canonicalization: it does not resolve symlinks
 * or `..`, so two genuinely different spellings of one directory still key
 * differently — the same limitation `scripts` has always had. Case is handled
 * separately, on lookup rather than on write: see resolveProjectKey.
 */
export function projectKey(cwd: string): string {
  return cwd.replace(/\\/g, '/').replace(/\/+$/, '');
}

/**
 * The key to read and write for `cwd` in a per-directory map, honouring an
 * existing entry that differs only by case.
 *
 * Windows paths are case-insensitive, and the app does not spell them
 * consistently — an agent cwd arrives as `c:/users/me/work/repo` while a
 * hand-written or picker-sourced path can be `C:/Users/me/work/repo`. Those are
 * one directory, so keying them separately silently splits a project's board in
 * two and the user's widgets appear to vanish.
 *
 * Lookup-time rather than write-time on purpose. Lowercasing inside projectKey
 * would be wrong on Linux and macOS, where `~/Repo` and `~/repo` really are
 * different directories, and it would orphan every `scripts` entry already
 * written with mixed case. Matching an existing key instead is correct on every
 * platform and cannot strand existing config: an exact hit always wins, a
 * case-only variant is adopted so writes land back on the entry that's already
 * there, and a genuinely new project gets its path as typed.
 */
export function resolveProjectKey(map: Record<string, unknown> | undefined, cwd: string): string {
  const key = projectKey(cwd);
  if (!map || Object.prototype.hasOwnProperty.call(map, key)) return key;
  // Case-insensitive fallback only where the filesystem is: doing this on a
  // case-sensitive OS would merge two directories that legitimately differ.
  if (!isCaseInsensitiveFS()) return key;
  const lowered = key.toLowerCase();
  for (const existing of Object.keys(map)) {
    if (existing.toLowerCase() === lowered) return existing;
  }
  return key;
}

/** Whether the host filesystem treats paths case-insensitively (Windows, macOS). */
function isCaseInsensitiveFS(): boolean {
  // navigator.platform is deprecated but is what the renderer reliably has;
  // userAgentData is not present on every Electron/Chromium build here.
  const p = (globalThis.navigator?.platform || '').toLowerCase();
  const ua = (globalThis.navigator?.userAgent || '').toLowerCase();
  return /win/.test(p) || /mac/.test(p) || /windows|mac os/.test(ua);
}
