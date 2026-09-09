/** Optional launch contributions from installed, trusted sidecar plugins. */
import { codexProbeCommand, readCodexProvider } from './codexRouting';

export interface LaunchContext {
  version: 1;
  agent: string;
  cwd: string;
  model?: string;
  resume: boolean;
  provider?: { id: string; baseUrl?: string };
}
export interface LaunchPatch {
  env: Record<string, string>;
  args: string[];
}

export function validateLaunchPatch(value: unknown): LaunchPatch {
  if (!value || typeof value !== 'object' || Array.isArray(value))
    throw new Error('Invalid launch integration response');
  const patch = value as Record<string, unknown>;
  if (Object.keys(patch).some((key) => key !== 'env' && key !== 'args'))
    throw new Error('Unsupported launch integration field');
  const env = patch.env === undefined ? {} : patch.env;
  const args = patch.args === undefined ? [] : patch.args;
  if (!env || typeof env !== 'object' || Array.isArray(env) || Object.keys(env).length > 64)
    throw new Error('Invalid launch environment');
  for (const [key, entry] of Object.entries(env)) {
    if (
      !/^[A-Za-z_][A-Za-z0-9_]{0,127}$/.test(key) ||
      ['__proto__', 'constructor', 'prototype'].includes(key) ||
      typeof entry !== 'string' ||
      entry.includes('\0') ||
      entry.length > 32768
    )
      throw new Error('Invalid launch environment entry');
  }
  if (
    !Array.isArray(args) ||
    args.length > 64 ||
    args.some((arg) => typeof arg !== 'string' || arg.includes('\0') || arg.length > 8192)
  )
    throw new Error('Invalid launch arguments');
  return { env: { ...env } as Record<string, string>, args: [...args] as string[] };
}

async function listIntegrations(): Promise<any[]> {
  const { getHubToken, hubHttpUrl } = await import('./hubDaemon');
  const token = getHubToken();
  const response = await fetch(`${hubHttpUrl()}/plugins`, {
    headers: token ? { Authorization: `Bearer ${token}` } : {},
    signal: AbortSignal.timeout(3000),
  });
  if (!response.ok) throw new Error('Could not read plugins from the hub');
  const list = await response.json();
  if (!Array.isArray(list)) throw new Error('Invalid plugin list');
  return list;
}

export async function prepareLaunchIntegration(
  id: string | null | undefined,
  context: Omit<LaunchContext, 'version' | 'provider'>,
  base: { env: Record<string, string>; args: string[]; bin?: string },
  dependencies = {
    list: listIntegrations,
    call: async (method: string, params: unknown): Promise<unknown> =>
      (await import('./hubClient')).callHub(method, params),
    routing: readCodexProvider,
  },
): Promise<LaunchPatch> {
  if (id === undefined || id === null) return { env: base.env, args: base.args };
  if (typeof id !== 'string' || !id.trim() || id !== id.trim() || id.length > 200)
    throw new Error('Invalid launch integration selection');
  try {
    if (!['claude', 'codex'].includes(context.agent))
      throw new Error('This host supports launch integrations for Claude and Codex only');
    const plugin = (await dependencies.list()).find((item) => item.id === id && !item.disabled);
    const contribution = plugin?.launchIntegration;
    if (
      contribution?.version !== 1 ||
      !contribution.agents?.includes(context.agent) ||
      !contribution.prepareMethod?.startsWith(`${id}.`) ||
      !plugin.provides?.includes(contribution.prepareMethod)
    ) {
      throw new Error(`Plugin is unavailable or does not support ${context.agent}`);
    }
    const provider =
      context.agent === 'codex'
        ? await dependencies.routing(codexProbeCommand(base.bin!), context.cwd, base.env, base.args)
        : undefined;
    // Never send a raw config, inherited environment, headers, or credentials to the plugin.
    const patch = validateLaunchPatch(
      await dependencies.call(contribution.prepareMethod, {
        version: 1,
        ...context,
        ...(provider ? { provider } : {}),
      }),
    );
    // Integration args are appended so a profile cannot silently restore its upstream route.
    return { env: { ...base.env, ...patch.env }, args: [...base.args, ...patch.args] };
  } catch (error) {
    throw new Error(
      `[WKS_LAUNCH_INTEGRATION] Launch integration ${id}: ${error instanceof Error ? error.message : 'preparation failed'}. Restore the plugin/service or select None for a new launch.`,
    );
  }
}
