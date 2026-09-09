import { EventEmitter } from 'events';
import { PassThrough } from 'stream';
import * as fs from 'fs';
import * as path from 'path';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
vi.mock('./configService', () => ({ configService: { getConfig: () => ({}) } }));
vi.mock('./claudemonDaemon', () => ({ CLAUDEMON_API_URL: 'http://unused' }));
vi.mock('./claudeResolver', () => ({ claudeBaseArgv: () => ['unused'] }));
vi.mock('child_process', () => ({ spawn: vi.fn() }));
import { spawn } from 'child_process';
import {
  completeCodexReadinessPing,
  CODEX_PING_VERSION,
  codexPingArgs,
} from './codexReadinessPing';
import { completionReadiness } from './providerReadiness';
let root: string, bin: string, version: string, config: any, requirements: any, mode: string;
let children: any[], prompts: string[];
const success =
  '{"type":"thread.started"}\n{"type":"turn.started"}\n{"type":"item.completed","item":{"type":"agent_message","text":"OK"}}\n{"type":"turn.completed"}\n';
beforeEach(() => {
  root = fs.mkdtempSync(path.join(process.cwd(), '.codex-ping-test-'));
  bin = path.join(root, 'codex');
  fs.writeFileSync(bin, Buffer.from('7f454c46', 'hex'));
  vi.stubEnv('CODEX_HOME', root);
  fs.writeFileSync(
    path.join(root, 'models_cache.json'),
    JSON.stringify({
      client_version: '0.153.4',
      models: [{ slug: 'gpt-5.4-mini', use_responses_lite: false }],
    }),
  );
  version = CODEX_PING_VERSION;
  config = { model_provider: 'openai', cli_auth_credentials_store: 'auto' };
  requirements = null;
  mode = 'ok';
  children = [];
  prompts = [];
  vi.mocked(spawn)
    .mockReset()
    .mockImplementation((_bin, args) => {
      const child = Object.assign(new EventEmitter(), {
        stdout: new PassThrough(),
        stderr: new PassThrough(),
        stdin: new PassThrough(),
        kill: vi.fn(),
      });
      children.push(child);
      if (args?.includes('app-server')) {
        child.stdin.on('data', (chunk) => {
          for (const line of chunk.toString().trim().split('\n')) {
            const request = JSON.parse(line);
            if (request.method === 'thread/start' || request.method === 'turn/start')
              throw Error('Metadata must never start a thread');
            if (mode === 'hang-metadata') return;
            const result =
              request.id === 1
                ? {}
                : request.id === 2
                  ? { config }
                  : request.id === 3
                    ? { requirements }
                    : {
                        data: [
                          {
                            model: 'gpt-5.4-mini',
                            hidden: false,
                            supportedReasoningEfforts: [{ reasoningEffort: 'low' }],
                          },
                        ],
                        nextCursor: null,
                      };
            if (request.id)
              queueMicrotask(() =>
                child.stdout.write(JSON.stringify({ id: request.id, result }) + '\n'),
              );
          }
        });
      } else {
        let input = '';
        child.stdin.on('data', (chunk) => {
          input += chunk;
        });
        child.stdin.on('finish', () => {
          prompts.push(input);
          if (mode === 'hang-inference' && !args?.includes('--version')) return;
          const out = args?.includes('--version')
            ? version
            : mode === 'ok'
              ? success
              : mode === 'invalid'
                ? '{}'
                : JSON.stringify({ type: 'turn.failed', error: { message: mode } }) + '\n';
          child.stdout.write(out);
          child.emit('close', 0);
        });
      }
      return child as never;
    });
});
afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllEnvs();
  fs.rmSync(root, { recursive: true, force: true });
});
it('preserves provider/account selection while isolating the exact binary; only normalized state escapes', async () => {
  const result = await completeCodexReadinessPing(bin, new AbortController().signal);
  expect(result).toMatchObject({ ok: true, provider: 'codex', model: 'gpt-5.4-mini' });
  expect(vi.mocked(spawn).mock.calls.every(([value]) => value === bin)).toBe(true);
  expect(spawn).toHaveBeenCalledTimes(3);
  const args = vi.mocked(spawn).mock.calls[2][1]!;
  expect(args).toEqual(
    expect.arrayContaining([
      '--ignore-user-config',
      '--ignore-rules',
      '--ephemeral',
      '--model',
      'gpt-5.4-mini',
      'cli_auth_credentials_store="auto"',
    ]),
  );
  expect(args).toEqual(
    expect.arrayContaining([
      'model_provider="workspacer_readiness"',
      'model_providers.workspacer_readiness.requires_openai_auth=true',
      'model_providers.workspacer_readiness.request_max_retries=0',
    ]),
  );
  expect(prompts).toEqual(['', 'Reply OK.']);
  expect(completionReadiness(result, 123)).toEqual({ state: 'responding', checkedAt: 123 });
  expect(children[1].kill).toHaveBeenCalledWith('SIGKILL');
});
it('rejects unknown installed versions before reading config or making inference', async () => {
  version = 'codex-cli 0.154.0';
  expect(await completeCodexReadinessPing(bin, new AbortController().signal)).toMatchObject({
    ok: false,
    reason: 'no-tools-unsupported',
  });
  expect(spawn).toHaveBeenCalledTimes(1);
});
it('rejects the package-fetching shell wrapper before execution', async () => {
  fs.writeFileSync(bin, '#!/bin/sh\nnpx --prefer-online codex "$@"');
  expect(await completeCodexReadinessPing(bin, new AbortController().signal)).toMatchObject({
    ok: false,
    reason: 'no-tools-unsupported',
  });
  expect(spawn).not.toHaveBeenCalled();
});
it.each([
  { model_provider: 'other' },
  { model_provider: 'openai', chatgpt_base_url: 'PRIVATE' },
  { model_provider: 'openai', forced_chatgpt_workspace_id: ['PRIVATE'] },
  { model_provider: 'openai', cli_auth_credentials_store: 'future' },
  { model_provider: 'openai', features: { future_auth_storage: false } },
  null,
])('refuses ambiguous/changed auth configuration without a paid call', async (cfg) => {
  config = cfg;
  const result = await completeCodexReadinessPing(bin, new AbortController().signal);
  expect(result).toMatchObject({ ok: false, reason: 'no-tools-unsupported' });
  expect(spawn).toHaveBeenCalledTimes(2);
  expect(JSON.stringify(result)).not.toContain('PRIVATE');
});
it('refuses managed constraints instead of bypassing them', async () => {
  requirements = { features: { shell_tool: true } };
  expect(await completeCodexReadinessPing(bin, new AbortController().signal)).toMatchObject({
    ok: false,
    reason: 'no-tools-unsupported',
  });
  expect(spawn).toHaveBeenCalledTimes(2);
});
it.each([
  ['401 Unauthorized', 'unauthenticated'],
  ['429 quota', 'limited'],
  ['unexpected failure', 'error'],
  ['invalid', 'error'],
])('classifies %s without claiming every failure is auth', async (error, state) => {
  mode = error;
  expect(
    completionReadiness(await completeCodexReadinessPing(bin, new AbortController().signal), 1)
      .state,
  ).toBe(state);
  expect(spawn).toHaveBeenCalledTimes(3);
});
it('cancellation during config handshake kills only the disposable process and prevents inference', async () => {
  mode = 'hang-metadata';
  const controller = new AbortController();
  const pending = completeCodexReadinessPing(bin, controller.signal);
  await vi.waitFor(() => expect(children).toHaveLength(2));
  controller.abort();
  expect(await pending).toMatchObject({ ok: false, reason: 'cancelled' });
  expect(children[1].kill).toHaveBeenCalledWith('SIGKILL');
  expect(spawn).toHaveBeenCalledTimes(2);
});
it('cancelled inference never becomes responding', async () => {
  mode = 'hang-inference';
  const controller = new AbortController();
  const pending = completeCodexReadinessPing(bin, controller.signal);
  await vi.waitFor(() => expect(children).toHaveLength(3));
  controller.abort();
  expect(await pending).toMatchObject({ ok: false, reason: 'cancelled' });
  expect(children[2].kill).toHaveBeenCalledWith('SIGKILL');
});
it('pins every context/tool switch, without modifying the generic no-tools adapter', () => {
  const args = codexPingArgs('/fixture/catalog', 'file');
  for (const value of [
    'skills.include_instructions=false',
    'skills.bundled.enabled=false',
    'project_doc_max_bytes=0',
    'features.plugins=false',
    'features.hooks=false',
    'features.shell_tool=false',
    'features.remote_models=false',
    'tools.update_plan.enabled=false',
    'tools.experimental_request_user_input.enabled=false',
    'agents.enabled=false',
  ])
    expect(args).toContain(value);
});

it('preserves explicit auth switches and nullable defaults from the CLI config projection', async () => {
  config.features = { secret_auth_storage: true, auth_elicitation: null };
  config.chatgpt_base_url = 'https://chatgpt.com/backend-api/';
  expect(await completeCodexReadinessPing(bin, new AbortController().signal)).toMatchObject({
    ok: true,
  });
  expect(vi.mocked(spawn).mock.calls[2][1]).toContain('features.secret_auth_storage=true');
});

it('bounds a metadata timeout without starting inference or retrying', async () => {
  vi.useFakeTimers();
  mode = 'hang-metadata';
  const pending = completeCodexReadinessPing(bin, new AbortController().signal);
  await vi.advanceTimersByTimeAsync(4001);
  expect(await pending).toMatchObject({ ok: false, reason: 'timeout' });
  expect(spawn).toHaveBeenCalledTimes(2);
  expect(children[1].kill).toHaveBeenCalledWith('SIGKILL');
});
it('refuses unknown model transport cache instead of guessing a wire protocol', async () => {
  fs.writeFileSync(
    path.join(root, 'models_cache.json'),
    JSON.stringify({ client_version: 'future', models: [] }),
  );
  expect(await completeCodexReadinessPing(bin, new AbortController().signal)).toMatchObject({
    ok: false,
    reason: 'no-tools-unsupported',
  });
  expect(spawn).toHaveBeenCalledTimes(2);
});

it('refuses cwd-dependent account roots before any process or auth read', async () => {
  vi.stubEnv('CODEX_HOME', 'relative-account');
  expect(await completeCodexReadinessPing(bin, new AbortController().signal)).toMatchObject({
    ok: false,
    reason: 'no-tools-unsupported',
  });
  expect(spawn).not.toHaveBeenCalled();
});
