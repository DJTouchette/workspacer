import { describe, it, expect, afterEach } from 'vitest';
import {
  filterPaneMenuForRemote,
  REMOTE_UNAVAILABLE_PANES,
  remoteDisabledTitle,
  hubOfflineLabel,
  fetchFederationPeers,
} from '../src/lib/federation';
import { buildPaneMenu } from '../src/lib/paneMenu';
import type { PluginPane } from '../src/types/plugin';

const mkPlugin = (type: string, title = type): PluginPane => ({
  pluginId: `p.${type}`,
  type,
  title,
  url: `http://127.0.0.1/${type}`,
  scope: 'both',
});

describe('filterPaneMenuForRemote', () => {
  it('no hub = passthrough (same entries, untouched)', () => {
    const entries = buildPaneMenu(undefined, [mkPlugin('acme.tracker')]);
    expect(filterPaneMenuForRemote(entries, undefined)).toEqual(entries);
    expect(filterPaneMenuForRemote(entries, '')).toEqual(entries);
  });

  it('remote hub drops the cwd-bound built-ins from the default menu', () => {
    // Default menu is claude/terminal/browser/review — terminal + review are
    // local-machine surfaces (shell, git) and must go; chat + browser stay.
    const entries = filterPaneMenuForRemote(buildPaneMenu(undefined, []), 'aegis');
    expect(entries.map((e) => (e.kind === 'builtin' ? e.type : e.pane.type))).toEqual([
      'claude',
      'browser',
    ]);
  });

  it('drops an explicitly configured editor entry too', () => {
    // 'editor' is not in the default menu, but REMOTE_UNAVAILABLE_PANES gates
    // it wherever it appears (the pane menu is user-configurable).
    expect(REMOTE_UNAVAILABLE_PANES.has('editor')).toBe(true);
    const entries = filterPaneMenuForRemote(
      [
        { kind: 'builtin', type: 'editor', label: 'Editor' },
        { kind: 'builtin', type: 'claude', label: 'Claude Code' },
      ],
      'aegis',
    );
    expect(entries).toEqual([{ kind: 'builtin', type: 'claude', label: 'Claude Code' }]);
  });

  it('plugin panes survive the remote filter (only built-ins are cwd-gated)', () => {
    const plugins = [mkPlugin('acme.tracker', 'Tracker')];
    const entries = filterPaneMenuForRemote(buildPaneMenu(undefined, plugins), 'aegis');
    expect(entries.some((e) => e.kind === 'plugin' && e.pane.type === 'acme.tracker')).toBe(true);
  });
});

describe('remoteDisabledTitle', () => {
  it('names the hub in the hint', () => {
    expect(remoteDisabledTitle('aegis')).toBe('on aegis — not available for remote agents');
  });
});

describe('hubOfflineLabel', () => {
  const now = 1_000_000_000_000;
  it('no lastActivity = bare "hub offline"', () => {
    expect(hubOfflineLabel(undefined, now)).toBe('hub offline');
    expect(hubOfflineLabel(0, now)).toBe('hub offline');
  });
  it('formats the last-seen age compactly (m / h / d)', () => {
    expect(hubOfflineLabel(now - 5 * 60_000, now)).toBe('hub offline — last seen 5m ago');
    expect(hubOfflineLabel(now - 3 * 3_600_000, now)).toBe('hub offline — last seen 3h ago');
    expect(hubOfflineLabel(now - 2 * 86_400_000, now)).toBe('hub offline — last seen 2d ago');
  });
  it('never renders a negative age (clock skew clamps to 0s)', () => {
    expect(hubOfflineLabel(now + 60_000, now)).toBe('hub offline — last seen 0s ago');
  });
});

describe('fetchFederationPeers', () => {
  const win = window as { electronAPI?: unknown };
  const original = win.electronAPI;
  afterEach(() => {
    win.electronAPI = original;
  });

  it('returns the bridge list when present', async () => {
    const peers = [{ name: 'aegis', connected: true, lastSeen: 123 }];
    win.electronAPI = { federationPeers: async () => peers };
    expect(await fetchFederationPeers()).toEqual(peers);
  });

  it('returns [] when the bridge lacks the method (older preload / web mock)', async () => {
    win.electronAPI = {};
    expect(await fetchFederationPeers()).toEqual([]);
  });

  it('returns [] on a throwing or malformed bridge', async () => {
    win.electronAPI = {
      federationPeers: async () => {
        throw new Error('bus down');
      },
    };
    expect(await fetchFederationPeers()).toEqual([]);
    win.electronAPI = { federationPeers: async () => 'nonsense' };
    expect(await fetchFederationPeers()).toEqual([]);
  });
});
