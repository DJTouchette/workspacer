/**
 * The list of projects the app knows about, and the two facts it tracks about
 * each: whether it's pinned, and when it was last opened.
 *
 * Those two used to be `config.directories.favourites` and
 * `config.directories.recent` — two parallel arrays of paths, sitting beside a
 * `projects` map keyed by the same paths. "Favourite" is a boolean of a project
 * and "recent" is a timestamp of one, so keeping them apart meant dedup and
 * ordering logic in three places and no single answer to "what projects exist?"
 * (the Projects settings page had to union four separate sources to find out).
 *
 * MIGRATION: read both, write only the new shape. The legacy arrays are never
 * written again but are still honoured, so a config from an older build keeps
 * its pins and its ordering, and downgrading doesn't lose them either. An
 * explicit value in `projects` always wins over the legacy array — which is how
 * un-pinning a legacy favourite is expressible without rewriting the old key.
 */
import type { Config, ProjectIdentity } from '../hooks/useConfig';
import { projectKey } from './projectKey';

export interface KnownProject {
  /** The normalized directory — the key into `config.projects`. */
  dir: string;
  favourite: boolean;
  /** Epoch ms, or a synthesized ordering value for a legacy `recent` entry. */
  lastOpened: number;
}

/**
 * Legacy `recent` is an ORDER, not timestamps: index 0 is most recent. Map it
 * onto descending pseudo-timestamps below any real one, so a migrated list
 * keeps its order but any genuinely-touched project sorts above all of it.
 */
function legacyRank(index: number): number {
  return -(index + 1);
}

/** Every project the config knows about, most recently opened first, pinned
 *  ones first within that. */
export function listProjects(config: Partial<Config>): KnownProject[] {
  const projects = config.projects ?? {};
  const legacyFav = new Set((config.directories?.favourites ?? []).map(projectKey));
  const legacyRecent = (config.directories?.recent ?? []).map(projectKey);
  const legacyOrder = new Map(legacyRecent.map((d, i) => [d, legacyRank(i)]));

  const dirs = new Set<string>([
    ...Object.keys(projects).map(projectKey),
    ...legacyFav,
    ...legacyRecent,
    // A project known only because it carries scripts or a widget board is
    // still a project — it is a directory you configured something for.
    ...Object.keys(config.scripts ?? {}).map(projectKey),
    ...Object.keys(config.widgets ?? {}).map(projectKey),
  ]);

  const out: KnownProject[] = [];
  for (const dir of dirs) {
    if (!dir) continue;
    const entry: ProjectIdentity = projects[dir] ?? {};
    out.push({
      dir,
      // An explicit boolean wins; only its ABSENCE falls back to the legacy set,
      // so `favourite: false` can shadow a legacy pin.
      favourite: typeof entry.favourite === 'boolean' ? entry.favourite : legacyFav.has(dir),
      lastOpened:
        typeof entry.lastOpened === 'number'
          ? entry.lastOpened
          : (legacyOrder.get(dir) ?? -Infinity),
    });
  }
  out.sort((a, b) => {
    if (a.favourite !== b.favourite) return a.favourite ? -1 : 1;
    if (a.lastOpened !== b.lastOpened) return b.lastOpened - a.lastOpened;
    return a.dir.localeCompare(b.dir);
  });
  return out;
}

/** The pinned projects, most recent first. */
export function favouriteProjects(config: Partial<Config>): KnownProject[] {
  return listProjects(config).filter((p) => p.favourite);
}

/** Everything not pinned, most recently opened first. Projects that have never
 *  been opened and were never in `recent` sort last rather than vanishing. */
export function recentProjects(config: Partial<Config>): KnownProject[] {
  return listProjects(config).filter((p) => !p.favourite);
}

/**
 * The config patch that records a change to one project's entry, preserving
 * everything else about it.
 *
 * Returns the WHOLE `projects` map because configService replaces it wholesale
 * (a deep merge would resurrect a cleared entry) — so every caller must send
 * the complete truth, and building that is this module's job rather than each
 * call site's.
 */
export function patchProject(
  config: Partial<Config>,
  dir: string,
  patch: Partial<ProjectIdentity>,
): { projects: Record<string, ProjectIdentity> } {
  const key = projectKey(dir);
  const next: Record<string, ProjectIdentity> = { ...(config.projects ?? {}) };
  const merged: ProjectIdentity = { ...(next[key] ?? {}), ...patch };
  // Drop empties so an entry never persists as "all defaults", and drop the
  // entry entirely when nothing is left to say about it.
  for (const k of Object.keys(merged) as (keyof ProjectIdentity)[]) {
    const v = merged[k];
    if (v === undefined || v === null || v === '') delete merged[k];
    if (k === 'plugins' && v && !Object.keys(v as object).length) delete merged[k];
  }
  if (Object.keys(merged).length) next[key] = merged;
  else delete next[key];
  return { projects: next };
}

/**
 * Pin or unpin. Writes an explicit boolean rather than deleting the key: a
 * project that appears in the legacy `favourites` array needs `false` recorded
 * to actually come unpinned.
 */
export function setFavourite(
  config: Partial<Config>,
  dir: string,
  favourite: boolean,
): { projects: Record<string, ProjectIdentity> } {
  return patchProject(config, dir, { favourite });
}

/** Record that a project was just opened. */
export function touchProject(
  config: Partial<Config>,
  dir: string,
  now = Date.now(),
): { projects: Record<string, ProjectIdentity> } {
  return patchProject(config, dir, { lastOpened: now });
}

/** One plugin's per-project settings, merged over the plugin's own defaults by
 *  the caller. Absent when nothing has been configured for it here. */
export function projectPluginSettings(
  config: Partial<Config>,
  dir: string,
  pluginId: string,
): Record<string, unknown> {
  return (config.projects?.[projectKey(dir)]?.plugins ?? {})[pluginId] ?? {};
}

/** Write one plugin's per-project settings, leaving other plugins' alone. */
export function setProjectPluginSettings(
  config: Partial<Config>,
  dir: string,
  pluginId: string,
  values: Record<string, unknown>,
): { projects: Record<string, ProjectIdentity> } {
  const key = projectKey(dir);
  const existing = config.projects?.[key]?.plugins ?? {};
  const plugins = { ...existing };
  // An empty object means "nothing configured" — drop the namespace rather than
  // leaving `{"djtouchette.jira": {}}` behind in config.yaml forever.
  if (values && Object.keys(values).length) plugins[pluginId] = values;
  else delete plugins[pluginId];
  return patchProject(config, dir, { plugins });
}
