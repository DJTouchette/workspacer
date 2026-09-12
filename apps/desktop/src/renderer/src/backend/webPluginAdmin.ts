import type { ElectronAPI } from '../types/electron';
import type { PluginManifest, PluginUpdateStatus } from '../types/plugin';

type PluginAdmin = Pick<
  ElectronAPI,
  | 'installPlugin'
  | 'inspectPlugin'
  | 'checkPluginUpdates'
  | 'listExamplePlugins'
  | 'installExamplePlugin'
  | 'removePlugin'
  | 'setPluginEnabled'
>;

/** Same guarded HTTP routes used by preload IPC. Authorization stays server-side. */
export function createWebPluginAdmin(base: string, token: string): PluginAdmin {
  async function request(route: string, body?: unknown, timeout = 30_000) {
    const response = await fetch(`${base}/plugins/${route}`, {
      method: body === undefined ? 'GET' : 'POST',
      headers: { Authorization: `Bearer ${token}`, 'Content-Type': 'application/json' },
      ...(body === undefined ? {} : { body: JSON.stringify(body) }),
      signal: AbortSignal.timeout(timeout),
    });
    const payload = await response.json().catch(() => null);
    return { response, payload };
  }
  function check({ response, payload }: Awaited<ReturnType<typeof request>>) {
    if (!response.ok)
      throw new Error(
        typeof payload?.error === 'string' ? payload.error : `HTTP ${response.status}`,
      );
    if (payload === null) throw new Error('The server returned an invalid plugin response');
    return payload;
  }
  async function plugin(route: string, body: unknown, timeout?: number) {
    try {
      return { ok: true, plugin: check(await request(route, body, timeout)) as PluginManifest };
    } catch (error) {
      return failed(error);
    }
  }
  function failed(error: unknown) {
    return { ok: false, error: error instanceof Error ? error.message : 'Plugin request failed' };
  }
  return {
    inspectPlugin: (url) => plugin('inspect', { url }),
    installPlugin: async (url) => {
      try {
        let result = await request('install', { url }, 360_000);
        if (result.response.status === 409 && result.payload?.needsConsent) {
          const { argv, pluginId } = result.payload;
          if (
            !Array.isArray(argv) ||
            !argv.length ||
            !argv.every((arg) => typeof arg === 'string')
          ) {
            return { ok: false, error: 'The server returned an invalid build command' };
          }
          // A build executes on the server. Show the exact argv, and only retry
          // with that argv after an explicit choice; never consent on inspection.
          if (
            !window.confirm(
              `Install ${pluginId ?? 'this plugin'}?\n\nSource: ${url}\n\nThis build command runs on the server with its user permissions:\n${JSON.stringify(argv)}\n\nContinue only if you trust this source.`,
            )
          ) {
            return { ok: false, error: 'Install cancelled' };
          }
          result = await request(
            'install',
            { url, allowInstallCommand: true, consentedArgv: argv },
            360_000,
          );
        }
        return { ok: true, plugin: check(result) as PluginManifest };
      } catch (error) {
        return failed(error);
      }
    },
    checkPluginUpdates: async () => {
      try {
        const updates = check(await request('updates', {}, 90_000));
        if (!Array.isArray(updates)) throw new Error('The server returned an invalid updates list');
        return { ok: true, updates: updates as PluginUpdateStatus[] };
      } catch (error) {
        return failed(error);
      }
    },
    listExamplePlugins: async () => {
      const examples = check(await request('examples', undefined, 5_000));
      if (!Array.isArray(examples)) throw new Error('The server returned an invalid examples list');
      return examples as PluginManifest[];
    },
    installExamplePlugin: (id) => plugin('examples/install', { id }, 360_000),
    removePlugin: async (id) => {
      try {
        const payload = check(await request('remove', { id }));
        if (payload.ok !== true) throw new Error('The server did not confirm plugin removal');
        return { ok: true };
      } catch (error) {
        return failed(error);
      }
    },
    setPluginEnabled: (id, enabled) => plugin('setEnabled', { id, enabled }),
  };
}
