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
 * read — keyed by the project's absolute directory. Nothing new is written to
 * config for this; the roots are a projection of what is already there, so there
 * is no setting to leave unread.
 */

import * as os from 'os';
import { configService } from '../services/configService';

export function webviewFileRoots(): string[] {
  const roots = new Set<string>();
  const home = os.homedir();
  if (home) roots.add(home);
  let projects: Record<string, unknown> = {};
  try {
    projects = configService.getConfig().projects ?? {};
  } catch {
    // A config that cannot be read narrows the allowance to home; it must never
    // widen it, and it must never take the window down.
  }
  for (const dir of Object.keys(projects)) {
    if (dir.trim()) roots.add(dir);
  }
  return [...roots];
}
