/**
 * The plugin catalogue over the hub's HTTP routes.
 *
 * `listHubPlugins` was a stub on the web backend (`[]` + a warnOnce), so `/app`
 * — and the desktop's remote-client mode, which is this same backend — knew of
 * no plugins at all: no palette entries, no pane-menu entries, no widgets. A
 * plugin pane could only be reached by restoring a layout document some desktop
 * client had written. The registry itself is not host-owned: main's IPC handler
 * is a thin proxy over GET /plugins + GET /plugins/tokens, and this client holds
 * the same bearer token, so it asks the hub directly.
 *
 * It also stamps the two bases the renderer must not guess (see
 * types/plugin.ts): the hub origin that serves `/plugins/ui/<id>/`, and a
 * sidecar's own base — the latter ONLY when the hub is this machine's loopback,
 * because `127.0.0.1:<port>` on a browser somewhere else is that browser's own
 * machine.
 */
import { beforeEach, afterEach, describe, expect, it, vi } from 'vitest';
import { createWebBackend } from '../../src/backend/webBackend';

vi.mock('../../src/backend/hubBusClient', () => ({
  HubBusClient: class {
    constructor(
      readonly token: string,
      readonly busUrl?: string,
    ) {}
    start() {}
    isConnected() {
      return true;
    }
    onStatus() {
      return () => {};
    }
    onReconnect() {
      return () => {};
    }
    subscribe() {
      return () => {};
    }
    call() {
      return Promise.resolve({});
    }
  },
}));

// Fresh objects per fetch: the backend stamps onto what the hub handed it, and
// a shared literal would carry one test's stamps into the next.
const manifests = () => [
  {
    id: 'acme.tracker',
    name: 'Tracker',
    apiVersion: '1',
    ui: 'ui',
    panes: [{ type: 'x', title: 'X' }],
  },
  {
    id: 'acme.ship',
    name: 'Ship',
    apiVersion: '1',
    server: { command: 'node', port: 9211 },
    panes: [{ type: 'y', title: 'Y' }],
  },
];

interface Recorded {
  url: string;
  init?: RequestInit;
}

function stubFetch(recorded: Recorded[], opts: { tokens?: Record<string, string> } = {}) {
  global.fetch = vi.fn(async (input: any, init?: RequestInit) => {
    const url = String(input);
    recorded.push({ url, init });
    if (url.endsWith('/plugins'))
      return { ok: true, status: 200, json: async () => manifests() } as any;
    if (url.endsWith('/plugins/tokens'))
      return { ok: true, status: 200, json: async () => opts.tokens ?? {} } as any;
    return { ok: false, status: 404, json: async () => ({}) } as any;
  }) as unknown as typeof fetch;
}

const originalFetch = global.fetch;
let recorded: Recorded[];

beforeEach(() => {
  recorded = [];
});
afterEach(() => {
  global.fetch = originalFetch;
});

describe('webBackend.listHubPlugins', () => {
  it('reads the hub catalogue with the client’s bearer token', async () => {
    stubFetch(recorded, { tokens: { 'acme.tracker': 'TOK-TRACKER' } });
    const api = createWebBackend('HOSTTOKEN');

    const list = (await api.listHubPlugins!()) as any[];
    expect(list.map((p) => p.id)).toEqual(['acme.tracker', 'acme.ship']);

    const plugins = recorded.find((r) => r.url.endsWith('/plugins'))!;
    expect((plugins.init?.headers as Record<string, string>).Authorization).toBe(
      'Bearer HOSTTOKEN',
    );
    // The per-plugin bus token is merged in, exactly as main does it.
    expect(list[0].busToken).toBe('TOK-TRACKER');
  });

  it('stamps the hub origin it is actually talking to on a hub-served plugin', async () => {
    stubFetch(recorded);
    // A browser on /app: no explicit bus URL, the page came from the hub.
    const api = createWebBackend('HOSTTOKEN');
    const list = (await api.listHubPlugins!()) as any[];
    expect(list[0].uiBase).toBe(location.origin);
  });

  it('derives the origin from the bus URL when one was given (remote-client mode)', async () => {
    stubFetch(recorded);
    const api = createWebBackend('HOSTTOKEN', 'wss://wks.example.dev/bus');
    const list = (await api.listHubPlugins!()) as any[];
    expect(list[0].uiBase).toBe('https://wks.example.dev');
    expect(recorded.some((r) => r.url === 'https://wks.example.dev/plugins')).toBe(true);
  });

  it('gives a sidecar a loopback base only when the hub IS this machine', async () => {
    stubFetch(recorded);
    const local = createWebBackend('HOSTTOKEN', 'ws://127.0.0.1:7895/bus');
    const localList = (await local.listHubPlugins!()) as any[];
    expect(localList[1].serverBase).toBe('http://127.0.0.1:9211');

    recorded = [];
    stubFetch(recorded);
    const remote = createWebBackend('HOSTTOKEN', 'wss://wks.example.dev/bus');
    const remoteList = (await remote.listHubPlugins!()) as any[];
    // The sidecar listens on the HUB's loopback. This browser's 127.0.0.1 is a
    // different machine entirely, so there is no base to offer.
    expect(remoteList[1].serverBase).toBeUndefined();
  });

  it('answers null (not []) when the hub is unreachable, so usePlugins retries', async () => {
    global.fetch = vi.fn(async () => {
      throw new Error('connection refused');
    }) as unknown as typeof fetch;
    const api = createWebBackend('HOSTTOKEN');
    await expect(api.listHubPlugins!()).resolves.toBeNull();
  });

  it('still lists plugins when the token route refuses (degraded, not empty)', async () => {
    global.fetch = vi.fn(async (input: any) => {
      const url = String(input);
      if (url.endsWith('/plugins')) return { ok: true, json: async () => manifests() } as any;
      return { ok: false, status: 403, json: async () => ({}) } as any;
    }) as unknown as typeof fetch;
    const api = createWebBackend('HOSTTOKEN');
    const list = (await api.listHubPlugins!()) as any[];
    expect(list.map((p) => p.id)).toEqual(['acme.tracker', 'acme.ship']);
    expect(list[0].busToken).toBeUndefined();
  });
});
