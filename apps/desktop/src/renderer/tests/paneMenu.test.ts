import { describe, it, expect } from 'vitest';
import { buildPaneMenu, DEFAULT_PANE_MENU } from '../src/lib/paneMenu';
import type { PluginPane } from '../src/types/plugin';

const mkPlugin = (type: string, title = type): PluginPane => ({
  pluginId: `p.${type}`,
  type,
  title,
  url: `http://127.0.0.1/${type}`,
  scope: 'both',
});

describe('buildPaneMenu', () => {
  it('unset menu = built-in defaults followed by every plugin pane', () => {
    const plugins = [mkPlugin('acme.tracker', 'Tracker'), mkPlugin('acme.notes', 'Notes')];
    const entries = buildPaneMenu(undefined, plugins);

    // Built-ins first, in DEFAULT_PANE_MENU order.
    expect(entries.slice(0, DEFAULT_PANE_MENU.length)).toEqual(
      DEFAULT_PANE_MENU.map((type) => ({
        kind: 'builtin',
        type,
        label: expect.any(String),
      })),
    );
    // Then all plugins, in order.
    expect(entries.slice(DEFAULT_PANE_MENU.length)).toEqual([
      { kind: 'plugin', pane: plugins[0], label: 'Tracker' },
      { kind: 'plugin', pane: plugins[1], label: 'Notes' },
    ]);
  });

  it('does not offer the retired "notes" built-in by default', () => {
    const entries = buildPaneMenu(undefined, []);
    expect(entries.some((e) => e.kind === 'builtin' && e.type === ('notes' as never))).toBe(false);
  });

  it('explicit list is honored verbatim, in order, mixing built-ins and plugins', () => {
    const plugins = [mkPlugin('acme.tracker', 'Tracker')];
    const entries = buildPaneMenu(['browser', 'acme.tracker', 'claude'], plugins);
    expect(entries).toEqual([
      { kind: 'builtin', type: 'browser', label: 'Browser' },
      { kind: 'plugin', pane: plugins[0], label: 'Tracker' },
      { kind: 'builtin', type: 'claude', label: 'Claude Code' },
    ]);
  });

  it('explicit list drops unknown ids (stale plugin, removed pane type)', () => {
    const entries = buildPaneMenu(['claude', 'notes', 'gone.plugin', 'terminal'], []);
    expect(entries.map((e) => (e.kind === 'builtin' ? e.type : e.pane.type))).toEqual([
      'claude',
      'terminal',
    ]);
  });

  it('an explicit empty list yields an empty menu (not the default)', () => {
    expect(buildPaneMenu([], [mkPlugin('acme.tracker')])).toEqual([]);
  });

  it('built-in takes precedence over a plugin whose type collides with a built-in id', () => {
    // A plugin declares a pane whose `type` collides with the built-in 'review' id.
    const plugins = [mkPlugin('review', 'Impostor Review')];
    const entries = buildPaneMenu(['review'], plugins);
    // Per the documented precedence, an id resolves to a built-in first, only
    // falling back to a plugin pane if it is not a built-in.
    expect(entries).toEqual([{ kind: 'builtin', type: 'review', label: 'Review' }]);
  });
});
