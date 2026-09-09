import { resolveCodexReadinessBinary } from './codexReadinessBinary';
/** Codex 0.153.4 readiness-only isolation. This does NOT relax the generic
 * completion adapter's requireNoTools guard. The catalog is deliberately local:
 * remote model metadata can otherwise add clock/async tools despite feature flags.
 * See docs/fleet-provider-readiness.md for the source and loopback wire proof. */
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import { spawn } from 'child_process';
import type { CompletionResult } from './directCompletion';
import { classifyCliFailure, CompletionFailure, runCli } from './directCompletion';

export const CODEX_PING_VERSION = 'codex-cli 0.153.4';
export const CODEX_PING_MODEL = 'gpt-5.4-mini';
export const CODEX_PING_CATALOG = {
  models: [
    {
      slug: CODEX_PING_MODEL,
      display_name: CODEX_PING_MODEL,
      description: null,
      supported_reasoning_levels: [{ effort: 'low', description: 'Low' }],
      default_reasoning_level: 'low',
      shell_type: 'disabled',
      visibility: 'list',
      supported_in_api: true,
      priority: 0,
      support_verbosity: false,
      truncation_policy: { mode: 'tokens', limit: 1000 },
      experimental_supported_tools: [],
      apply_patch_tool_type: null,
      model_messages: { instructions_template: 'Reply OK.', persistent_instructions: '' },
      include_skills_usage_instructions: false,
      include_apps_usage_instructions: false,
      include_plugin_usage_instructions: false,
      supports_search_tool: false,
      tool_mode: 'direct',
      multi_agent_version: 'disabled',
      node_repl_disabled: true,
      use_responses_lite: false,
      supports_reasoning_summary_parameter: true,
    },
  ],
};
const DISABLED_FEATURES = [
  'shell_tool',
  'view_image',
  'plugins',
  'remote_plugin',
  'apps',
  'hooks',
  'plugin_hooks',
  'remote_models',
  'memories',
  'external_agent_memory_import',
  'external_migration',
  'js_repl',
  'js_repl_tools_only',
  'image_generation',
  'browser_use',
  'computer_use',
  'deferred_executor',
  'token_budget',
  'current_time_reminder',
  'sleep_tool',
  'tool_suggest',
  'code_mode',
  'code_mode_only',
  'multi_agent',
  'multi_agent_v2',
  'search_tool',
  'tool_search',
  'request_permissions_tool',
  'standalone_web_search',
  'sqlite',
  'unbounded_connection_retries',
  'shell_snapshot',
  'shell_snapshot_v2',
];
const AUTH_FEATURES = [
  'auth_elicitation',
  'secret_auth_storage',
  'respect_system_proxy',
  'network_proxy',
  'psp',
  'use_agent_identity',
];
function classifyCodexFailure(text: string) {
  return /model[^\n]*(?:not supported|not available|does not exist)/i.test(text)
    ? ('unsupported-model' as const)
    : classifyCliFailure(text);
}
function overrides(values: Record<string, unknown>): string[] {
  return Object.entries(values).flatMap(([key, value]) => [
    '-c',
    `${key}=${JSON.stringify(value)}`,
  ]);
}
/** Shared with the installed-CLI loopback contract test. No secrets in argv. */
export function codexPingArgs(
  catalog: string,
  store: string,
  authFeatures: Record<string, boolean> = {},
  model = CODEX_PING_MODEL,
  effort = 'low',
): string[] {
  return [
    'exec',
    '--ignore-user-config',
    '--ignore-rules',
    '--ephemeral',
    '--skip-git-repo-check',
    '--json',
    '--model',
    model,
    ...overrides({
      ...Object.fromEntries(DISABLED_FEATURES.map((f) => [`features.${f}`, false])),
      'skills.include_instructions': false,
      'skills.bundled.enabled': false,
      'tools.update_plan.enabled': false,
      'tools.experimental_request_user_input.enabled': false,
      'agents.enabled': false,
      project_doc_max_bytes: 0,
      include_environment_context: false,
      include_permissions_instructions: false,
      include_apps_instructions: false,
      include_collaboration_mode_instructions: false,
      developer_instructions: '',
      web_search: 'disabled',
      'analytics.enabled': false,
      'feedback.enabled': false,
      model_reasoning_effort: effort,
      model_reasoning_summary: 'none',
      notify: [],
      ...Object.fromEntries(
        Object.entries(authFeatures).filter(
          ([key, value]) =>
            AUTH_FEATURES.some((name) => key === `features.${name}`) && typeof value === 'boolean',
        ),
      ),
      model_catalog_json: catalog,
      cli_auth_credentials_store: store,
      model_provider: 'workspacer_readiness',
      'model_providers.workspacer_readiness.name': 'OpenAI',
      'model_providers.workspacer_readiness.wire_api': 'responses',
      'model_providers.workspacer_readiness.http_headers.version': '0.153.4',
      'model_providers.workspacer_readiness.requires_openai_auth': true,
      'model_providers.workspacer_readiness.env_http_headers.OpenAI-Organization':
        'OPENAI_ORGANIZATION',
      'model_providers.workspacer_readiness.env_http_headers.OpenAI-Project': 'OPENAI_PROJECT',
      'model_providers.workspacer_readiness.request_max_retries': 0,
      'model_providers.workspacer_readiness.stream_max_retries': 0,
    }),
    '-',
  ];
}

/** Verify the launcher's intended route through its own read-only config API.
 * No thread is created. Read neither auth.json nor keyrings ourselves. Refuse
 * non-default routes/policies instead of letting --ignore-user-config change auth.
 * The metadata process has plugins/hooks/skill installation disabled before boot.
 */
async function authStoreForIsolatedPing(
  bin: string,
  signal: AbortSignal,
): Promise<{
  store: string | null;
  category: string;
  authFeatures: Record<string, boolean>;
  model: string | null;
  effort: string;
}> {
  return new Promise((resolve) => {
    if (signal.aborted) {
      resolve({ store: null, category: 'cancelled', authFeatures: {}, model: null, effort: 'low' });
      return;
    }
    const args = [
      'app-server',
      ...overrides({
        'features.plugins': false,
        'features.remote_plugin': false,
        'features.hooks': false,
        'features.plugin_hooks': false,
        'features.apps': false,
        'features.external_migration': false,
        'features.memories': false,
        'features.remote_control': false,
        'skills.bundled.enabled': false,
        'analytics.enabled': false,
        'feedback.enabled': false,
        'features.sqlite': false,
      }),
    ];
    const child = spawn(bin, args, {
      cwd: os.tmpdir(),
      stdio: ['pipe', 'pipe', 'pipe'],
      shell: false,
      windowsHide: true,
    });
    const authFeatures: Record<string, boolean> = {};
    let model: string | null = null,
      effort = 'low';
    let buffer = '',
      bytes = 0,
      settled = false,
      store: string | null = null;
    const finish = (value: string | null, category = 'metadata-unavailable') => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      signal.removeEventListener('abort', abort);
      child.kill('SIGKILL');
      resolve({ store: value, category, authFeatures, model, effort });
    };
    const abort = () => finish(null);
    const timer = setTimeout(() => finish(null, 'timeout'), 4000);
    signal.addEventListener('abort', abort, { once: true });
    if (signal.aborted) abort();
    child.stdin?.on('error', abort);
    child.on('error', abort);
    child.on('close', abort);
    const send = (value: unknown) => child.stdin?.write(JSON.stringify(value) + '\n');
    child.stderr?.on('data', (chunk: Buffer) => {
      bytes += chunk.length;
      if (bytes > 131072) abort();
    });
    child.stdout?.on('data', (chunk: Buffer) => {
      bytes += chunk.length;
      if (bytes > 131072) {
        abort();
        return;
      }
      buffer += chunk.toString('utf8');
      while (buffer.includes('\n') && !settled) {
        const end = buffer.indexOf('\n'),
          line = buffer.slice(0, end);
        buffer = buffer.slice(end + 1);
        let row: any;
        try {
          row = JSON.parse(line);
        } catch {
          abort();
          return;
        }
        if (row.error) {
          abort();
          return;
        }
        if (row.id === 1) {
          send({ method: 'initialized' });
          send({ id: 2, method: 'config/read', params: { includeLayers: false } });
        }
        if (row.id === 2) {
          const cfg = row.result?.config;
          const category =
            !cfg || typeof cfg !== 'object'
              ? 'config-shape'
              : cfg.model_provider && cfg.model_provider !== 'openai'
                ? 'custom-provider'
                : cfg.model_providers?.openai
                  ? 'custom-openai'
                  : Object.keys(cfg.features ?? {}).some(
                        (key) =>
                          /auth|proxy|psp|identity/.test(key) && !AUTH_FEATURES.includes(key),
                      )
                    ? 'auth-features'
                    : cfg.chatgpt_base_url &&
                        ![
                          'https://chatgpt.com/backend-api/',
                          'https://chatgpt.com/backend-api',
                        ].includes(cfg.chatgpt_base_url)
                      ? 'custom-endpoint'
                      : cfg.forced_login_method || cfg.forced_chatgpt_workspace_id
                        ? 'auth-policy'
                        : cfg.model_provider_auth
                          ? 'auth-adapter'
                          : null;
          if (category) {
            finish(null, category);
            return;
          }
          for (const name of AUTH_FEATURES) {
            const value = cfg.features?.[name];
            if (value == null) continue;
            if (typeof value !== 'boolean') {
              finish(null, 'auth-feature-shape');
              return;
            }
            authFeatures[`features.${name}`] = value;
          }
          const candidate = cfg.cli_auth_credentials_store ?? 'file';
          if (!['file', 'keyring', 'auto'].includes(candidate)) {
            abort();
            return;
          }
          store = candidate;
          send({ id: 3, method: 'configRequirements/read', params: {} });
        }
        if (row.id === 3) {
          if (!row.result || row.result.requirements !== null) {
            finish(null, 'managed-policy');
            return;
          }
          send({ id: 4, method: 'model/list', params: { includeHidden: false, limit: 100 } });
        }
        if (row.id === 4) {
          const rows = row.result?.data;
          if (!Array.isArray(rows) || row.result?.nextCursor) {
            finish(null, 'model-catalog');
            return;
          }
          const candidate = rows.find(
            (item: any) =>
              item &&
              item.hidden !== true &&
              typeof item.model === 'string' &&
              /^(gpt-|codex-|o[0-9])/.test(item.model) &&
              /^[a-zA-Z0-9._-]{1,96}$/.test(item.model) &&
              /(?:mini|nano|luna)(?:$|[-.])/.test(item.model),
          );
          if (!candidate) {
            finish(null, 'cheap-model-unavailable');
            return;
          }
          const supported = candidate.supportedReasoningEfforts;
          const selectedEffort = ['none', 'minimal', 'low'].find(
            (value) =>
              Array.isArray(supported) &&
              supported.some((entry: any) => entry?.reasoningEffort === value),
          );
          if (!selectedEffort) {
            finish(null, 'cheap-effort-unavailable');
            return;
          }
          model = candidate.model;
          effort = selectedEffort;
          finish(store, 'ready');
        }
      }
    });
    send({
      id: 1,
      method: 'initialize',
      params: {
        clientInfo: { name: 'workspacer-readiness', version: '1' },
        capabilities: { experimentalApi: true },
      },
    });
  });
}

/** Read only the CLI's public model metadata cache, never its auth storage.
 * Transport flags belong to the advertised model: synthesizing classic Responses
 * for a Responses Lite model fails even when its id is available to the account.
 */
function modelTransport(model: string): { lite: boolean; summary: boolean } | null {
  try {
    const root = process.env.CODEX_HOME || path.join(os.homedir(), '.codex');
    const file = path.join(root, 'models_cache.json');
    const stat = fs.lstatSync(file);
    if (!stat.isFile() || stat.size > 4000000) return null;
    const data = JSON.parse(fs.readFileSync(file, 'utf8'));
    if (data.client_version !== '0.153.4' || !Array.isArray(data.models)) return null;
    const entry = data.models.find((row: any) => row?.slug === model);
    if (
      !entry ||
      (entry.use_responses_lite != null && typeof entry.use_responses_lite !== 'boolean') ||
      (entry.supports_reasoning_summary_parameter != null &&
        typeof entry.supports_reasoning_summary_parameter !== 'boolean')
    )
      return null;
    return {
      lite: entry.use_responses_lite ?? false,
      summary: entry.supports_reasoning_summary_parameter ?? true,
    };
  } catch {
    return null;
  }
}

export async function completeCodexReadinessPing(
  bin: string,
  signal: AbortSignal,
): Promise<CompletionResult> {
  const started = Date.now();
  let model: string | null = null;
  const fail = (
    reason: import('./directCompletion').CompletionFailureReason,
    category = 'unavailable',
  ): CompletionResult => ({
    ok: false,
    reason,
    message: `Codex readiness: ${category}`,
    provider: 'codex',
    model,
    elapsedMs: Date.now() - started,
  });
  let scratch: string | undefined;
  try {
    if (
      process.platform === 'win32' ||
      process.env.OPENAI_BASE_URL ||
      (process.env.CODEX_HOME && !path.isAbsolute(process.env.CODEX_HOME))
    )
      return fail('no-tools-unsupported');
    bin = resolveCodexReadinessBinary(bin) ?? bin;
    const fd = fs.openSync(bin, 'r'),
      magic = Buffer.alloc(4);
    try {
      fs.readSync(fd, magic, 0, 4, 0);
    } finally {
      fs.closeSync(fd);
    }
    if (!['7f454c46', 'cffaedfe', 'feedfacf', 'cafebabe'].includes(magic.toString('hex')))
      return fail('no-tools-unsupported');
    const version = await runCli({
      bin,
      args: ['--version'],
      stdin: '',
      timeoutMs: 2000,
      maxOutputChars: 256,
      signal,
      cwd: os.tmpdir(),
    });
    if (version.trim() !== CODEX_PING_VERSION) return fail('no-tools-unsupported');
    const prerequisite = await authStoreForIsolatedPing(bin, signal);
    const store = prerequisite.store;
    if (signal.aborted) return fail('cancelled');
    if (!store || !prerequisite.model)
      return fail(
        prerequisite.category === 'timeout' ? 'timeout' : 'no-tools-unsupported',
        prerequisite.category,
      );
    model = prerequisite.model;
    const transport = modelTransport(model);
    if (!transport) return fail('no-tools-unsupported', 'model-transport-unavailable');
    scratch = fs.mkdtempSync(path.join(os.tmpdir(), 'workspacer-codex-ping-'));
    const catalog = path.join(scratch, 'models.json');
    const metadata = structuredClone(CODEX_PING_CATALOG);
    metadata.models[0].use_responses_lite = transport.lite;
    metadata.models[0].supports_reasoning_summary_parameter = transport.summary;
    metadata.models[0].slug = model;
    metadata.models[0].display_name = model;
    metadata.models[0].default_reasoning_level = prerequisite.effort;
    metadata.models[0].supported_reasoning_levels = [
      { effort: prerequisite.effort, description: 'Readiness effort' },
    ];
    fs.writeFileSync(catalog, JSON.stringify(metadata), { mode: 0o600 });
    const output = await runCli({
      bin,
      args: codexPingArgs(catalog, store, prerequisite.authFeatures, model, prerequisite.effort),
      stdin: 'Reply OK.',
      timeoutMs: 12000,
      maxOutputChars: 8000,
      signal,
      cwd: scratch,
    });
    let answer = '',
      completed = false,
      startedTurn = false;
    for (const line of output.trim().split('\n')) {
      const row = JSON.parse(line);
      if (row.type === 'error' || row.type === 'turn.failed')
        return fail(classifyCodexFailure(JSON.stringify(row)), 'provider-error');
      if (row.type === 'turn.started') startedTurn = true;
      if (row.type === 'item.completed') {
        if (row.item?.type === 'error') {
          const message = typeof row.item.message === 'string' ? row.item.message : '';
          if (
            !startedTurn &&
            (message.startsWith('Under-development features enabled:') ||
              /^`\[features\]\.[a-z_]+` is deprecated\./.test(message))
          )
            continue;
          return fail(classifyCodexFailure(message), 'cli-error');
        }
        if (row.item?.type === 'agent_message') answer = row.item.text;
        else if (row.item?.type !== 'reasoning') return fail('failed', 'unexpected-item');
      }
      if (row.type === 'turn.completed') completed = true;
    }
    if (!completed || answer?.trim() !== 'OK') return fail('empty');
    return {
      ok: true,
      text: 'OK',
      provider: 'codex',
      model,
      elapsedMs: Date.now() - started,
    };
  } catch (error) {
    const reason =
      error instanceof CompletionFailure
        ? error.reason === 'failed'
          ? classifyCodexFailure(error.message)
          : error.reason
        : 'failed';
    return fail(
      reason,
      error instanceof CompletionFailure
        ? /ECONN|ENOTFOUND|EAI_AGAIN|network|connection|fetch failed/i.test(error.message)
          ? 'network'
          : 'process-error'
        : 'protocol-error',
    );
  } finally {
    if (scratch) {
      try {
        fs.rmSync(scratch, { recursive: true, force: true });
      } catch {
        /* public model metadata only; never turn cleanup into a raw IPC error */
      }
    }
  }
}
