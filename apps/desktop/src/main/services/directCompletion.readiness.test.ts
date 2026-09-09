import { EventEmitter } from 'events';
import { PassThrough } from 'stream';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
vi.mock('./configService', () => ({ configService: { getConfig: () => ({}) } }));
vi.mock('./claudemonDaemon', () => ({ CLAUDEMON_API_URL: 'http://unused' }));
vi.mock('./claudeResolver', () => ({ claudeBaseArgv: () => ['unused'] }));
vi.mock('child_process', () => ({ spawn: vi.fn() }));
const metadata = vi.hoisted(() => ({ magic: '7f454c46' }));
vi.mock('fs', async (load) => ({
  ...(await load<typeof import('fs')>()),
  openSync: () => 7,
  closeSync: () => {},
  readSync: (_fd: number, buffer: Buffer) => {
    Buffer.from(metadata.magic, 'hex').copy(buffer);
    return 4;
  },
}));
import { spawn } from 'child_process';
import { completeReadinessPing } from './directCompletion';
const help =
  '--safe-mode --tools --strict-mcp-config --no-session-persistence --system-prompt --disable-slash-commands --output-format';
let responses: Array<{ out: string; err?: string; code?: number }>;
beforeEach(() => {
  metadata.magic = '7f454c46';
  responses = [
    { out: help },
    { out: JSON.stringify({ type: 'result', is_error: false, result: 'OK' }) },
  ];
  vi.mocked(spawn)
    .mockReset()
    .mockImplementation(() => {
      const child = Object.assign(new EventEmitter(), {
        stdout: new PassThrough(),
        stderr: new PassThrough(),
        stdin: new PassThrough(),
        kill: vi.fn(),
      });
      const response = responses.shift()!;
      queueMicrotask(() => {
        child.stdout.write(response.out);
        child.stderr.write(response.err ?? '');
        child.emit('close', response.code ?? 0);
      });
      return child as never;
    });
});
afterEach(() => {
  vi.unstubAllEnvs();
  vi.useRealTimers();
});
it('capability-checks the exact configured native executable, preserving auth and isolating inference', async () => {
  vi.stubEnv('CLAUDE_CONFIG_DIR', 'fixture-account-secret');
  const result = await completeReadinessPing(
    'claude',
    '/fixture/claude',
    new AbortController().signal,
  );
  expect(result).toMatchObject({ ok: true, text: 'OK', provider: 'claude', model: 'haiku' });
  expect(spawn).toHaveBeenCalledTimes(2);
  const [bin, args, opts] = vi.mocked(spawn).mock.calls[1];
  expect(bin).toBe('/fixture/claude');
  expect(args).toEqual(
    expect.arrayContaining([
      '--safe-mode',
      '--tools',
      '',
      '--no-session-persistence',
      '--system-prompt',
      'Reply OK.',
      '--model',
      'haiku',
    ]),
  );
  expect(args).not.toContain('--bare');
  expect(args).not.toContain('--resume');
  expect(opts).toMatchObject({
    shell: false,
    env: {
      CLAUDE_CONFIG_DIR: 'fixture-account-secret',
      CLAUDE_CODE_MAX_RETRIES: '0',
      CLAUDE_CODE_MAX_OUTPUT_TOKENS: '64',
    },
  });
  expect(opts?.cwd).not.toBe(process.cwd());
  expect(JSON.stringify(result)).not.toContain('secret');
});
it.each(['codex', 'opencode', 'copilot', 'pi'] as const)(
  'does not execute unsupported %s or substitute providers',
  async (provider) => {
    expect(
      await completeReadinessPing(provider, '/fixture/bin', new AbortController().signal),
    ).toMatchObject({ ok: false, reason: 'no-tools-unsupported' });
    expect(spawn).not.toHaveBeenCalled();
  },
);
it('rejects package-fetching shell launchers before help', async () => {
  metadata.magic = '23212f62';
  expect(
    await completeReadinessPing('claude', '/fixture/shim', new AbortController().signal),
  ).toMatchObject({ ok: false, reason: 'no-tools-unsupported' });
  expect(spawn).not.toHaveBeenCalled();
});
it('old CLI without safe-mode never gets inference', async () => {
  responses[0] = { out: help.replace('--safe-mode', '') };
  expect(
    await completeReadinessPing('claude', '/fixture/bin', new AbortController().signal),
  ).toMatchObject({ ok: false, reason: 'no-tools-unsupported' });
  expect(spawn).toHaveBeenCalledTimes(1);
});
it.each([
  [{ out: '', err: '401 Unauthorized SECRET', code: 1 }, 'not-authed'],
  [{ out: '', err: '429 quota SECRET', code: 1 }, 'rate-limited'],
  [{ out: '{"type":"result","is_error":true,"result":"authentication failed"}' }, 'not-authed'],
  [{ out: 'OK' }, 'failed'],
  [{ out: '{"type":"result","is_error":false,"result":"different"}' }, 'empty'],
])(
  'does not treat failures/unknown wire shapes as successful inference',
  async (response, reason) => {
    responses[1] = response as (typeof responses)[number];
    expect(
      await completeReadinessPing('claude', '/fixture/bin', new AbortController().signal),
    ).toMatchObject({ ok: false, reason });
  },
);
it('aborted preflight makes zero process calls', async () => {
  const c = new AbortController();
  c.abort();
  expect(await completeReadinessPing('claude', '/fixture/bin', c.signal)).toMatchObject({
    ok: false,
    reason: 'cancelled',
  });
  expect(spawn).not.toHaveBeenCalled();
});
