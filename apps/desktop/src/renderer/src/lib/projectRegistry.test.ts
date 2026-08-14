/**
 * The projects registry, and the migration off `directories.recent/favourites`.
 *
 * The migration is read-both-write-new, so the cases that matter are the mixed
 * ones: a config that has only the old arrays, only the new map, or both with
 * the two disagreeing.
 */
import { describe, it, expect } from 'vitest';
import {
  listProjects,
  favouriteProjects,
  recentProjects,
  setFavourite,
  touchProject,
  patchProject,
  projectPluginSettings,
  setProjectPluginSettings,
} from './projectRegistry';

const legacy = {
  directories: { favourites: ['/w/pinned'], recent: ['/w/newest', '/w/older', '/w/pinned'] },
};

describe('listProjects — reading both shapes', () => {
  it('reads a config that only has the legacy arrays', () => {
    const list = listProjects(legacy);
    expect(list.map((p) => p.dir)).toEqual(['/w/pinned', '/w/newest', '/w/older']);
    expect(list[0].favourite).toBe(true);
  });

  it('preserves legacy `recent` ORDER, which is an order and not timestamps', () => {
    expect(recentProjects(legacy).map((p) => p.dir)).toEqual(['/w/newest', '/w/older']);
  });

  it('reads a config that only has the new map', () => {
    const list = listProjects({
      projects: {
        '/w/a': { favourite: true },
        '/w/b': { lastOpened: 200 },
        '/w/c': { lastOpened: 100 },
      },
    });
    expect(list.map((p) => p.dir)).toEqual(['/w/a', '/w/b', '/w/c']);
  });

  it('lets an explicit value SHADOW the legacy array', () => {
    // The whole reason favourite is a boolean and not just presence: unpinning
    // something the old array pinned has to be expressible without rewriting
    // the old key.
    const list = listProjects({ ...legacy, projects: { '/w/pinned': { favourite: false } } });
    expect(list.find((p) => p.dir === '/w/pinned')!.favourite).toBe(false);
    expect(
      favouriteProjects({ ...legacy, projects: { '/w/pinned': { favourite: false } } }),
    ).toEqual([]);
  });

  it('sorts a genuinely-touched project above every migrated one', () => {
    const list = listProjects({ ...legacy, projects: { '/w/older': { lastOpened: Date.now() } } });
    expect(list.map((p) => p.dir)).toEqual(['/w/pinned', '/w/older', '/w/newest']);
  });

  it('includes projects known only through scripts or a widget board', () => {
    // These are directories you configured something for; they belong in the
    // list even though nobody pinned or recently opened them.
    const dirs = listProjects({
      scripts: { '/w/has-scripts': [] },
      widgets: { '/w/has-board': [] },
    }).map((p) => p.dir);
    expect(dirs).toContain('/w/has-scripts');
    expect(dirs).toContain('/w/has-board');
  });

  it('normalizes keys, so one directory is never two rows', () => {
    const list = listProjects({
      directories: { favourites: ['/w/repo/'], recent: [] },
      projects: { '/w/repo': { label: 'Repo' } },
    });
    expect(list).toHaveLength(1);
    expect(list[0].favourite).toBe(true);
  });

  it('is empty for an empty config rather than throwing', () => {
    expect(listProjects({})).toEqual([]);
  });
});

describe('writes produce the whole map, because config replaces it wholesale', () => {
  it('pins without disturbing other entries or other fields', () => {
    const config = { projects: { '/w/a': { label: 'A' }, '/w/b': { icon: '🚀' } } };
    const { projects } = setFavourite(config, '/w/b', true);
    expect(projects['/w/a']).toEqual({ label: 'A' });
    expect(projects['/w/b']).toEqual({ icon: '🚀', favourite: true });
  });

  it('records `false` explicitly so a legacy pin can be undone', () => {
    const { projects } = setFavourite(legacy, '/w/pinned', false);
    expect(projects['/w/pinned']).toEqual({ favourite: false });
  });

  it('touch records a timestamp', () => {
    const { projects } = touchProject({}, '/w/a', 1234);
    expect(projects['/w/a']).toEqual({ lastOpened: 1234 });
  });

  it('drops an entry once nothing is left to say about it', () => {
    const config = { projects: { '/w/a': { label: 'A' } } };
    const { projects } = patchProject(config, '/w/a', { label: '' });
    expect(projects['/w/a']).toBeUndefined();
  });

  it('normalizes the key it writes', () => {
    const { projects } = touchProject({}, '/w/a/', 1);
    expect(Object.keys(projects)).toEqual(['/w/a']);
  });
});

describe('per-project plugin settings', () => {
  const cfg = {
    projects: {
      '/w/a': { plugins: { 'x.jira': { prefix: 'HVMS' }, 'x.ship': { repo: 'o/n' } } },
    },
  };

  it('reads one plugin’s namespace', () => {
    expect(projectPluginSettings(cfg, '/w/a', 'x.jira')).toEqual({ prefix: 'HVMS' });
    expect(projectPluginSettings(cfg, '/w/a', 'x.nope')).toEqual({});
    expect(projectPluginSettings({}, '/w/a', 'x.jira')).toEqual({});
  });

  it('writes one plugin without disturbing another', () => {
    const { projects } = setProjectPluginSettings(cfg, '/w/a', 'x.jira', { prefix: 'PLAT' });
    expect(projects['/w/a'].plugins).toEqual({
      'x.jira': { prefix: 'PLAT' },
      'x.ship': { repo: 'o/n' },
    });
  });

  it('removes the namespace when a plugin is cleared, rather than leaving {}', () => {
    const { projects } = setProjectPluginSettings(cfg, '/w/a', 'x.jira', {});
    expect(projects['/w/a'].plugins).toEqual({ 'x.ship': { repo: 'o/n' } });
  });

  it('drops the whole entry when the last plugin is cleared and nothing else is set', () => {
    const only = { projects: { '/w/a': { plugins: { 'x.jira': { prefix: 'H' } } } } };
    const { projects } = setProjectPluginSettings(only, '/w/a', 'x.jira', {});
    expect(projects['/w/a']).toBeUndefined();
  });
});
