/** Opt-in installed-CLI wire proof. Only a loopback fixture sees the request;
 * the CLI uses a new temporary CODEX_HOME, no real provider credentials. */
import { createServer } from 'http';
import * as fs from 'fs';
import * as path from 'path';
import { expect, it, vi } from 'vitest';
vi.mock('./configService', () => ({ configService: { getConfig: () => ({}) } }));
vi.mock('./claudemonDaemon', () => ({ CLAUDEMON_API_URL: 'http://unused' }));
vi.mock('./claudeResolver', () => ({ claudeBaseArgv: () => ['unused'] }));
import { codexPingArgs, CODEX_PING_CATALOG } from './codexReadinessPing';
import { runCli } from './directCompletion';
const native = process.env.WORKSPACER_TEST_CODEX_BIN;
it.skipIf(!native).each([
  { lite: false, status: 200 },
  { lite: true, status: 200 },
  { lite: true, status: 429 },
  { lite: true, status: 500 },
  { lite: true, status: 0 },
])(
  'installed Codex isolation and single attempt: %j',
  async ({ lite, status }) => {
    const root = fs.mkdtempSync(path.join(process.cwd(), '.codex-native-test-'));
    const home = path.join(root, 'home');
    fs.mkdirSync(home);
    const catalog = path.join(root, 'models.json');
    const metadata = structuredClone(CODEX_PING_CATALOG);
    metadata.models[0].use_responses_lite = lite;
    fs.writeFileSync(catalog, JSON.stringify(metadata));
    fs.writeFileSync(path.join(root, 'AGENTS.md'), 'PRIVATE_REPO_CANARY');
    fs.writeFileSync(
      path.join(home, 'config.toml'),
      'developer_instructions="PRIVATE_CONFIG_CANARY"\nnotify=["touch", "SHOULD_NOT_EXIST"]\n',
    );
    const requests: any[] = [];
    const controller = new AbortController();
    const server = createServer((req, res) => {
      let raw = '';
      req.on('data', (chunk) => {
        raw += chunk;
      });
      req.on('end', () => {
        requests.push(JSON.parse(raw));
        if (status === 0) {
          controller.abort();
          return;
        }
        if (status !== 200) {
          res.writeHead(status, { 'Content-Type': 'application/json' });
          res.end(
            JSON.stringify({
              error: { message: status === 429 ? 'rate limited' : 'server failure' },
            }),
          );
          return;
        }
        const events = [
          {
            type: 'response.created',
            response: { id: 'fixture', status: 'in_progress', output: [] },
          },
          {
            type: 'response.output_item.done',
            output_index: 0,
            item: {
              type: 'message',
              id: 'message',
              role: 'assistant',
              status: 'completed',
              content: [{ type: 'output_text', text: 'OK', annotations: [] }],
            },
          },
          {
            type: 'response.completed',
            response: {
              id: 'fixture',
              status: 'completed',
              output: [],
              usage: { input_tokens: 1, output_tokens: 1, total_tokens: 2 },
            },
          },
        ];
        res.writeHead(200, { 'Content-Type': 'text/event-stream' });
        res.end(events.map((e) => 'data: ' + JSON.stringify(e) + '\n\n').join(''));
      });
    });
    await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve));
    try {
      const port = (server.address() as { port: number }).port;
      const args = codexPingArgs(catalog, 'file');
      args.pop();
      args.push(
        '-c',
        'model_provider="fixture"',
        '-c',
        `model_providers.fixture={name="Fixture",base_url="http://127.0.0.1:${port}",wire_api="responses",requires_openai_auth=false,request_max_retries=0,stream_max_retries=0}`,
        '-',
      );
      const env = Object.fromEntries(
        Object.entries(process.env).filter(([k]) => !/(TOKEN|KEY|CODEX|OPENAI|ANTHROPIC)/i.test(k)),
      );
      const pending = runCli({
        bin: native!,
        args,
        stdin: 'Reply OK.',
        signal: controller.signal,
        timeoutMs: 15000,
        maxOutputChars: 8000,
        cwd: home,
        env: { ...env, CODEX_HOME: home },
      });
      if (status === 200) expect(await pending).toContain('turn.completed');
      else
        await expect(pending).rejects.toMatchObject({
          reason: status === 0 ? 'cancelled' : status === 429 ? 'rate-limited' : 'failed',
        });
      expect(requests).toHaveLength(1);
      expect(requests[0].tools).toEqual(lite ? undefined : []);
      expect(requests[0].instructions).toBe(lite ? undefined : 'Reply OK.');
      expect(requests[0].model).toBe('gpt-5.4-mini');
      const messages = requests[0].input;
      for (const item of messages) {
        if (item.type === 'additional_tools') expect(item.tools).toEqual([]);
        else expect(item.content).toEqual([{ type: 'input_text', text: 'Reply OK.' }]);
      }
      const users = messages.filter((item: any) => item.role === 'user');
      expect(users).toHaveLength(1);
      expect(users[0]).toMatchObject({
        role: 'user',
        content: [{ type: 'input_text', text: 'Reply OK.' }],
      });
      expect(JSON.stringify(requests)).not.toContain('CANARY');
      expect(fs.existsSync(path.join(home, 'SHOULD_NOT_EXIST'))).toBe(false);
      expect(fs.existsSync(path.join(home, 'auth.json'))).toBe(false);
      expect(fs.existsSync(path.join(home, 'sessions'))).toBe(false);
    } finally {
      server.closeAllConnections();
      await new Promise<void>((resolve) => server.close(() => resolve()));
      fs.rmSync(root, { recursive: true, force: true });
    }
  },
  20000,
);

it.skipIf(!native || process.env.WORKSPACER_TEST_CODEX_LIVE !== '1')(
  'authorized tiny live Codex check emits normalized facts only',
  async () => {
    const { completeCodexReadinessPing } = await import('./codexReadinessPing');
    const { completionReadiness } = await import('./providerReadiness');
    const result = await completeCodexReadinessPing(native!, new AbortController().signal);
    console.info(
      'Codex live readiness:',
      JSON.stringify({
        ...completionReadiness(result, Date.now()),
        category: result.ok ? 'responding' : result.message,
      }),
    );
    expect(result.ok).toBe(true);
  },
  20000,
);

it.skipIf(!native || !process.env.WORKSPACER_TEST_CODEX_WRAPPER)(
  'installed wrapper resolves to the verified native binary without executing it',
  async () => {
    const { resolveCodexReadinessBinary } = await import('./codexReadinessBinary');
    expect(resolveCodexReadinessBinary(process.env.WORKSPACER_TEST_CODEX_WRAPPER!)).toBe(
      fs.realpathSync(native!),
    );
  },
);
