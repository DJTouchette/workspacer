/**
 * The directories a <webview> may open a `file:` URL from.
 *
 * ONE definition, read fresh on every check by both doors of the webview guard
 * (see webviewGuard.installWebviewGuards). The set is deliberately small and
 * derived, never configured:
 *
 *   - the user's home directory, because that is where an agent writes the HTML
 *     it then asks the user to look at (reports, design mockups, coverage output);
 *   - every directory the user has a project registered for, because a project
 *     may legitimately live outside home (a mounted volume, /srv, D:\work).
 *
 * `config.projects` is the same map the sidebar, the Spawn dialog and the brain
 * read, keyed by the project's absolute directory. Nothing new is written to
 * config for this; the roots are a projection of what is already there, so there
 * is no setting to leave unread.
 *
 * ## What a root may not be
 *
 * A root is the whole width of the allowance, so a bad one is not a bad entry,
 * it is the absence of a policy. `config.projects` is a plain map: it is written
 * by the desktop AND by the Go brain, and `projects.add` is reachable from the
 * bus, so its keys are not all typed by a person at a dialog. Three shapes are
 * refused outright and logged once each:
 *
 *   - not absolute (a relative path, or a `~` nobody expands): it can confine
 *     nothing, and `pathWithinRoots` already answers false for it;
 *   - unresolvable: a component that is not a directory, a symlink cycle, an
 *     unreadable ancestor. A root we cannot canonicalize is a root we cannot
 *     prove anything about;
 *   - resolving to a VOLUME ROOT (`/`, `C:\`, `\\server\share\`). That is the
 *     one value for which "confined to this subtree" and "not confined at all"
 *     are the same sentence, so it can never be an honest scope. Detected by
 *     SHAPE (the path is its own parent) rather than by comparing to a literal,
 *     so the Windows forms are covered without spelling each of them out, and
 *     CANONICALLY, so a project directory that is a symlink to `/` is caught
 *     too. This is the rule `plugin/manager.go` applies to a plugin's declared
 *     path scopes (isVolumeRoot + the same log line); it earned its place there
 *     because a scope that resolved to `/` granted fs.write on
 *     ~/.claude/settings.json, and a webview root that resolves to `/` hands the
 *     browser pane every readable file on the host for the same reason.
 */

import * as os from 'os';
import * as path from 'path';
import { canonicalRoot } from './pathConfinement';
import { configService } from '../services/configService';

/** Roots already reported as unusable. `webviewFileRoots()` runs on EVERY check
 *  (an attach, every navigation, every window.open), so an unguarded log line
 *  here is a log flood, not a warning. */
const warned = new Set<string>();

function warnOnce(root: string, why: string): void {
  if (warned.has(root)) return;
  warned.add(root);
  console.warn(`[main] SECURITY: dropping webview file root ${JSON.stringify(root)}: ${why}`);
}

/**
 * The canonical form of a root that may be used, or null to DISCARD it.
 *
 * Exported for the tests, which is the only way to assert the volume-root arm
 * directly: reached through `webviewFileRoots` it is indistinguishable from a
 * key that was simply absent.
 */
export function usableRoot(dir: string): string | null {
  if (dir.trim() === '') return null; // blank keys are not worth a log line
  const canonical = canonicalRoot(dir);
  if (canonical === null) {
    warnOnce(dir, 'it is not an absolute path, or it does not canonicalize');
    return null;
  }
  if (path.dirname(canonical) === canonical) {
    warnOnce(
      dir,
      `it resolves to the volume root ${JSON.stringify(canonical)}, which contains every path on the host; that is not a scope`,
    );
    return null;
  }
  return canonical;
}

export function webviewFileRoots(): string[] {
  const roots = new Set<string>();
  const home = os.homedir();
  // Home goes through the same gate as everything else. A HOME of '/' is not a
  // reason to hand the pane the filesystem.
  if (home && usableRoot(home)) roots.add(home);
  let projects: Record<string, unknown> = {};
  try {
    projects = configService.getConfig().projects ?? {};
  } catch {
    // A config that cannot be read narrows the allowance to home; it must never
    // widen it, and it must never take the window down.
  }
  for (const dir of Object.keys(projects)) {
    // The RAW key is kept, not the canonical form: `pathWithinRoots` canonicalizes
    // each root itself on every check, so storing the resolved string here would
    // freeze a link that may legitimately be re-pointed while the app runs.
    if (usableRoot(dir)) roots.add(dir);
  }
  return [...roots];
}
