import { EventEmitter } from 'events';
import { PassThrough } from 'stream';
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
vi.mock('./configService', () => ({
  configService: { getConfig: () => ({ agents: { binaries: {} } }) },
}));
vi.mock('./claudemonDaemon', () => ({ CLAUDEMON_API_URL: 'http://127.0.0.1:0' }));
vi.mock('./claudeResolver', () => ({ claudeBaseArgv: () => ['fake-claude'] }));
vi.mock('./agentProviders', () => ({
  resolveAgentBinary: (p: string) => p,
  isAgentBinaryInstalled: () => true,
}));
vi.mock('child_process', () => ({ spawn: vi.fn() }));
import { spawn } from 'child_process';
import { complete } from './directCompletion';
let child: EventEmitter & {
  stdout: PassThrough;
  stderr: PassThrough;
  stdin: PassThrough;
  kill: ReturnType<typeof vi.fn>;
};
beforeEach(() => {
  vi.mocked(spawn)
    .mockReset()
    .mockImplementation(() => {
      child = Object.assign(new EventEmitter(), {
        stdout: new PassThrough(),
        stderr: new PassThrough(),
        stdin: new PassThrough(),
        kill: vi.fn(),
      });
      queueMicrotask(() => {
        child.stdout.write('result');
        child.emit('close', 0);
      });
      return child as never;
    });
});
afterEach(() => vi.unstubAllGlobals());
describe('no-tools adapter boundary', () => {
  it.each(['codex', 'opencode'] as const)(
    'refuses %s before a process or model',
    async (provider) => {
      const result = await complete({ provider, prompt: 'untrusted data', requireNoTools: true });
      expect(result).toMatchObject({ ok: false, reason: 'no-tools-unsupported' });
      expect(spawn).not.toHaveBeenCalled();
    },
  );
  it.each(['pi', 'copilot'] as const)(
    'passes enforced no-tools argv for %s without a project cwd',
    async (provider) => {
      const result = await complete({
        provider,
        prompt: 'data',
        requireNoTools: true,
        model: provider === 'pi' ? 'anthropic/claude-small' : 'auto',
      });
      expect(result.ok).toBe(true);
      const [, args, options] = vi.mocked(spawn).mock.calls[0];
      if (provider === 'pi')
        expect(args).toEqual(
          expect.arrayContaining(['--no-tools', '--no-extensions', '--no-skills', '--no-session']),
        );
      else
        expect(args).toEqual(
          expect.arrayContaining([
            '--available-tools=',
            '--no-custom-instructions',
            '--disable-builtin-mcps',
          ]),
        );
      expect(options).toMatchObject({ shell: false });
      expect(options?.cwd).not.toBe(process.cwd());
    },
  );
  it('passes explicit Claude null and no-tools requirements to the daemon', async () => {
    const fetchMock = vi.fn(async () => new Response(JSON.stringify({ ok: true, text: '{}' })));
    vi.stubGlobal('fetch', fetchMock);
    await complete({ provider: 'claude', model: null, prompt: 'data', requireNoTools: true });
    const body = JSON.parse(
      (fetchMock.mock.calls[0] as unknown as [string, RequestInit])[1].body as string,
    );
    expect(body).toMatchObject({ model: null, harness_default: true, no_tools: true });
    expect(spawn).not.toHaveBeenCalled();
  });
  it('kills a cancelled CLI and reports cancellation', async () => {
    vi.mocked(spawn).mockImplementation(() => {
      child = Object.assign(new EventEmitter(), {
        stdout: new PassThrough(),
        stderr: new PassThrough(),
        stdin: new PassThrough(),
        kill: vi.fn(),
      });
      return child as never;
    });
    const controller = new AbortController();
    const pending = complete({
      provider: 'pi',
      prompt: 'data',
      requireNoTools: true,
      signal: controller.signal,
    });
    controller.abort();
    expect(await pending).toMatchObject({ ok: false, reason: 'cancelled' });
    expect(child.kill).toHaveBeenCalledWith('SIGKILL');
  });
});
