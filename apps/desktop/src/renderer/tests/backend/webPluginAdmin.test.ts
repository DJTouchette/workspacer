import { afterEach, describe, expect, it, vi } from 'vitest';
import { createWebBackend } from '../../src/backend/webBackend';

vi.mock('../../src/backend/hubBusClient', () => ({
  HubBusClient: class {
    start() {}
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
    isConnected() {
      return false;
    }
  },
}));
afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});
const response = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status });
const api = () => createWebBackend('test-token', 'wss://selected.example/bus');

describe('web plugin administration', () => {
  it('uses the selected server and bearer for every operation, with desktop result shapes', async () => {
    const plugin = { id: 'example', name: 'Example' };
    const fetcher = vi
      .fn()
      .mockResolvedValueOnce(response(plugin))
      .mockResolvedValueOnce(response(plugin))
      .mockResolvedValueOnce(response([]))
      .mockResolvedValueOnce(response([plugin]))
      .mockResolvedValueOnce(response(plugin))
      .mockResolvedValueOnce(response({ ok: true }))
      .mockResolvedValueOnce(response(plugin));
    vi.stubGlobal('fetch', fetcher);
    const backend = api();
    expect(await backend.inspectPlugin('https://github.com/example/plugin')).toEqual({
      ok: true,
      plugin,
    });
    expect(await backend.installPlugin('https://github.com/example/plugin')).toEqual({
      ok: true,
      plugin,
    });
    expect(await backend.checkPluginUpdates()).toEqual({ ok: true, updates: [] });
    expect(await backend.listExamplePlugins()).toEqual([plugin]);
    expect(await backend.installExamplePlugin('example')).toEqual({ ok: true, plugin });
    expect(await backend.removePlugin('example')).toEqual({ ok: true });
    expect(await backend.setPluginEnabled('example', false)).toEqual({ ok: true, plugin });
    expect(fetcher.mock.calls.map(([url]) => url)).toEqual(
      ['inspect', 'install', 'updates', 'examples', 'examples/install', 'remove', 'setEnabled'].map(
        (route) => `https://selected.example/plugins/${route}`,
      ),
    );
    for (const [, options] of fetcher.mock.calls)
      expect(options.headers.Authorization).toBe('Bearer test-token');
    expect(JSON.parse(fetcher.mock.calls[6][1].body)).toEqual({ id: 'example', enabled: false });
  });

  it('does not retry owner denial or invent success on network and removal failures', async () => {
    const confirm = vi.spyOn(window, 'confirm');
    const fetcher = vi
      .fn()
      .mockResolvedValueOnce(response({ error: 'host authority required' }, 403))
      .mockRejectedValueOnce(new Error('offline'))
      .mockResolvedValueOnce(response({ ok: false }));
    vi.stubGlobal('fetch', fetcher);
    const backend = api();
    expect(await backend.installPlugin('source')).toEqual({
      ok: false,
      error: 'host authority required',
    });
    expect(confirm).not.toHaveBeenCalled();
    expect(await backend.checkPluginUpdates()).toEqual({ ok: false, error: 'offline' });
    expect(await backend.removePlugin('example')).toMatchObject({ ok: false });
    expect(fetcher).toHaveBeenCalledTimes(3);
  });

  it.each([false, true])('retries build consent only when accepted: %s', async (accepted) => {
    const argv = ['node', 'build script.js', '--production'];
    const fetcher = vi
      .fn()
      .mockResolvedValueOnce(response({ needsConsent: true, pluginId: 'example', argv }, 409))
      .mockResolvedValueOnce(response({ id: 'example' }));
    vi.stubGlobal('fetch', fetcher);
    const confirm = vi.spyOn(window, 'confirm').mockReturnValue(accepted);
    const result = await api().installPlugin('source');
    expect(confirm).toHaveBeenCalledWith(expect.stringContaining(JSON.stringify(argv)));
    expect(result.ok).toBe(accepted);
    expect(fetcher).toHaveBeenCalledTimes(accepted ? 2 : 1);
    expect(JSON.parse(fetcher.mock.calls[0][1].body)).toEqual({ url: 'source' });
    if (accepted)
      expect(JSON.parse(fetcher.mock.calls[1][1].body)).toEqual({
        url: 'source',
        allowInstallCommand: true,
        consentedArgv: argv,
      });
  });

  it('rejects malformed consent and distinguishes unavailable examples from an empty catalog', async () => {
    const fetcher = vi
      .fn()
      .mockResolvedValueOnce(response({ needsConsent: true, argv: [] }, 409))
      .mockResolvedValueOnce(new Response('unauthorized', { status: 401 }))
      .mockResolvedValueOnce(response([]));
    vi.stubGlobal('fetch', fetcher);
    const confirm = vi.spyOn(window, 'confirm');
    const backend = api();
    expect(await backend.installPlugin('source')).toMatchObject({ ok: false });
    expect(confirm).not.toHaveBeenCalled();
    await expect(backend.listExamplePlugins()).rejects.toThrow('HTTP 401');
    expect(await backend.listExamplePlugins()).toEqual([]);
  });
});
