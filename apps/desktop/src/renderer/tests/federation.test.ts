import { describe, it, expect, afterEach, vi } from 'vitest';
import {
  filterPaneMenuForRemote,
  REMOTE_UNAVAILABLE_PANES,
  remoteDisabledTitle,
  hubOfflineLabel,
  fetchFederationPeers,
  createRemoteConversationSync,
  type RemoteConversationTarget,
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

describe('createRemoteConversationSync', () => {
  const target = (over: Partial<RemoteConversationTarget> = {}): RemoteConversationTarget => ({
    sessionId: 's1',
    hub: 'work',
    ...over,
  });
  /** Settle the factory's internal promise chain (several microtask turns). */
  const settle = () => new Promise((r) => setTimeout(r, 0));

  it('ignores local and tombstoned targets — a down link is never polled', async () => {
    const fetch = vi.fn(async () => ({ seq: 1, items: [] as unknown[] }));
    const sync = createRemoteConversationSync(fetch);
    sync.poke(null);
    sync.poke(undefined);
    sync.poke({ sessionId: 's1' }); // no hub = local session
    sync.poke(target({ hubOffline: true })); // tombstone
    await settle();
    expect(fetch).not.toHaveBeenCalled();
  });

  it('first poke fetches full; later pokes pass the last seen seq', async () => {
    const fetch = vi.fn(async () => ({ seq: 7, items: [] as unknown[] }));
    const sync = createRemoteConversationSync(fetch);
    sync.poke(target());
    await settle();
    expect(fetch).toHaveBeenCalledWith('s1', undefined);
    sync.poke(target());
    await settle();
    expect(fetch).toHaveBeenLastCalledWith('s1', 7);
    expect(fetch).toHaveBeenCalledTimes(2);
  });

  it('pokes during a fetch coalesce into exactly one trailing fetch', async () => {
    const resolvers: Array<(v: { seq: number; items: unknown[] } | null) => void> = [];
    const fetch = vi.fn(
      () =>
        new Promise<{ seq: number; items: unknown[] } | null>((r) => {
          resolvers.push(r);
        }),
    );
    const sync = createRemoteConversationSync(fetch);
    sync.poke(target());
    sync.poke(target());
    sync.poke(target());
    expect(fetch).toHaveBeenCalledTimes(1);
    resolvers[0]({ seq: 3, items: [] });
    await settle();
    expect(fetch).toHaveBeenCalledTimes(2); // one trailing fetch, not two
    expect(fetch).toHaveBeenLastCalledWith('s1', 3);
    resolvers[1](null);
    await settle();
    expect(fetch).toHaveBeenCalledTimes(2); // queue drained, nothing spurious
  });

  it('a null answer records no seq — the next poke fetches full again', async () => {
    const fetch = vi.fn(async () => null);
    const sync = createRemoteConversationSync(fetch);
    sync.poke(target());
    await settle();
    sync.poke(target());
    await settle();
    expect(fetch).toHaveBeenNthCalledWith(2, 's1', undefined);
  });

  it('a fetch that rejects is swallowed and does not wedge the single-flight gate', async () => {
    let calls = 0;
    const fetch = vi.fn(async () => {
      if (++calls === 1) throw new Error('link flap');
      return { seq: 2, items: [] as unknown[] };
    });
    const sync = createRemoteConversationSync(fetch);
    sync.poke(target());
    await settle();
    sync.poke(target());
    await settle();
    expect(fetch).toHaveBeenCalledTimes(2);
  });

  it('tracks seq per session — switching targets starts full', async () => {
    const fetch = vi.fn(async () => ({ seq: 9, items: [] as unknown[] }));
    const sync = createRemoteConversationSync(fetch);
    sync.poke(target());
    await settle();
    sync.poke(target({ sessionId: 's2' }));
    await settle();
    expect(fetch).toHaveBeenLastCalledWith('s2', undefined);
  });
});
