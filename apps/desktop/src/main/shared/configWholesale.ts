/**
 * The config subtrees the main process replaces WHOLESALE on save instead of
 * deep-merging.
 *
 * These are all user-owned MAPS whose keys can be deleted (`ui.customThemes`,
 * `claude.budgets`, `projects`). A deep merge can only ever add or overwrite
 * keys, so under one a removed entry comes straight back — the caller therefore
 * sends the entire surviving map and main takes it as the whole truth.
 *
 * That contract binds BOTH ends and they have to agree:
 *
 *   - configService.saveConfig must replace (not merge) each of these.
 *   - lib/configPatch must NOT diff into them. Its job is to trim a save down
 *     to what changed, and a trimmed map is a map that has lost every entry the
 *     caller didn't touch — which main then writes as the whole truth.
 *
 * They lived as two hand-kept lists, and they drifted: `projects` was added to
 * main's side only, so saving one project's icon shipped a one-entry map and
 * wiped every other project's identity. One list now, imported by both.
 */
export const WHOLESALE_CONFIG_PATHS: ReadonlySet<string> = new Set([
  'ui.customThemes',
  'claude.budgets',
  'projects',
]);
