import { spawn } from 'child_process';
import { createInterface } from 'readline';
import * as path from 'path';
import * as fs from 'fs';
export interface CodexRouting {
  id: string;
  baseUrl?: string;
}

/** Resolve npm's Windows shim without invoking a shell with config arguments. */
export function codexProbeCommand(bin: string): string[] {
  if (!/\.cmd$/i.test(bin)) return [bin];
  const dir = path.dirname(bin);
  const script = path.join(dir, 'node_modules', '@openai', 'codex', 'bin', 'codex.js');
  if (!fs.existsSync(script))
    throw new Error('Codex integration requires a native binary or standard npm installation');
  const node = path.join(dir, 'node.exe');
  return [fs.existsSync(node) ? node : 'node', script];
}

function routingConfigArgs(args: string[]): string[] {
  const result: string[] = [];
  for (let i = 0; i < args.length; i++) {
    const arg = args[i];
    if (arg === '-p' || arg === '--profile' || arg.startsWith('--profile=') || /^-p.+/.test(arg)) {
      throw new Error(
        'Codex launch integrations currently require routing in the base config, without a native preset',
      );
    }
    if (arg === '-c' || arg === '--config') {
      if (!args[i + 1]) throw new Error('Missing Codex config override value');
      result.push(arg, args[++i]);
    } else if (arg.startsWith('--config=') || /^-c.+/.test(arg)) result.push(arg);
  }
  return result;
}

/** Extract only routing metadata. Never pass the raw config (which can contain secrets) to plugins. */
export function codexProviderFromConfig(
  config: any,
  env: Record<string, string> = {},
): CodexRouting {
  if (!config || typeof config !== 'object' || Array.isArray(config))
    throw new Error('Invalid Codex configuration response');
  // config/read does not promise to flatten legacy named profiles. Do not silently
  // route the wrong provider when that older configuration form is in use.
  if (config.profile)
    throw new Error(
      'Launch integrations require Codex routing in the base config, without a default named profile',
    );
  const id = config.model_provider ?? 'openai';
  if (typeof id !== 'string' || !id || id.length > 200)
    throw new Error('Invalid Codex model provider');
  const baseUrl =
    id === 'openai'
      ? (config.openai_base_url ??
        env.OPENAI_BASE_URL ??
        process.env.OPENAI_BASE_URL ??
        config.model_providers?.openai?.base_url)
      : config.model_providers?.[id]?.base_url;
  if (id !== 'openai' && !baseUrl)
    throw new Error('Selected Codex provider needs an explicit base_url for launch integrations');
  if (baseUrl != null) {
    let url: URL;
    try {
      url = new URL(baseUrl);
    } catch {
      throw new Error('Invalid Codex provider base URL');
    }
    if (
      !['https:', 'http:'].includes(url.protocol) ||
      url.username ||
      url.password ||
      url.search ||
      url.hash
    ) {
      throw new Error(
        'Codex provider base URL must be HTTP(S), without credentials, query, or fragment',
      );
    }
    return { id, baseUrl: url.toString().replace(/\/$/, '') };
  }
  return { id };
}

/** Ask the installed CLI to resolve its config layers, without starting a thread or turn. */
export function readCodexProvider(
  command: string[],
  cwd: string,
  env: Record<string, string> = {},
  extraArgs: string[] = [],
): Promise<CodexRouting> {
  return new Promise((resolve, reject) => {
    const child = spawn(
      command[0],
      [...command.slice(1), 'app-server', '--listen', 'stdio://', ...routingConfigArgs(extraArgs)],
      {
        cwd,
        env: { ...process.env, ...env },
        stdio: ['pipe', 'pipe', 'ignore'],
        windowsHide: true,
      },
    );
    const lines = createInterface({ input: child.stdout });
    let done = false;
    const finish = (error?: Error, provider?: CodexRouting) => {
      if (done) return;
      done = true;
      clearTimeout(timer);
      lines.close();
      child.stdin.destroy();
      child.kill();
      if (error) reject(error);
      else resolve(provider!);
    };
    const timer = setTimeout(
      () => finish(new Error('Timed out reading Codex routing configuration')),
      8000,
    );
    child.on('error', () =>
      finish(new Error('Could not start Codex to read routing configuration')),
    );
    child.on('exit', () =>
      finish(new Error('Codex exited before returning routing configuration')),
    );
    child.stdin.on('error', () => finish(new Error('Could not communicate with Codex')));
    const send = (message: unknown) => child.stdin.write(JSON.stringify(message) + '\n');
    lines.on('line', (line) => {
      let message: any;
      try {
        message = JSON.parse(line);
      } catch {
        return;
      }
      if (message.id !== 1 && message.id !== 2) return;
      if (message.error) {
        finish(new Error('Codex could not read configuration; check it with codex app-server'));
        return;
      }
      if (message.id === 1) {
        send({ method: 'initialized', params: {} });
        send({ id: 2, method: 'config/read', params: { includeLayers: false, cwd } });
      } else {
        try {
          finish(undefined, codexProviderFromConfig(message.result?.config, env));
        } catch (err) {
          finish(err as Error);
        }
      }
    });
    send({
      id: 1,
      method: 'initialize',
      params: { clientInfo: { name: 'workspacer_launch_routing', version: '1.0.0' } },
    });
  });
}
