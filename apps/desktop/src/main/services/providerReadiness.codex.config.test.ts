import { EventEmitter } from 'events';
import { PassThrough } from 'stream';
import * as fs from 'fs';
import * as path from 'path';
import { expect, it, vi } from 'vitest';
const owner = vi.hoisted(() => ({ pid: 4242 as number | null }));
vi.mock('./remoteServer', () => ({ isRemoteClientMode: () => false }));
vi.mock('./claudemonDaemon', () => ({
  CLAUDEMON_API_URL: 'http://unused',
  getClaudemonReadinessOwner: () => owner.pid,
}));
vi.mock('./claudeResolver', () => ({ claudeBaseArgv: () => ['must-not-run-claude'] }));
vi.mock('child_process', () => ({ spawn: vi.fn() }));
import { spawn } from 'child_process';
it.skipIf(process.platform === 'win32')(
  'persisted Codex opt-out prevents all startup processes; saved binary, cancellation and owner reach the real adapter',
  async () => {
    const root = fs.mkdtempSync(path.join(process.cwd(), '.codex-startup-test-'));
    vi.stubEnv('XDG_CONFIG_HOME', root);
    vi.stubEnv('CODEX_HOME', root);
    vi.useFakeTimers();
    const bin = path.join(root, 'codex'),
      other = path.join(root, 'codex-other');
    for (const file of [bin, other]) fs.writeFileSync(file, Buffer.from('7f454c46', 'hex'));
    fs.writeFileSync(
      path.join(root, 'models_cache.json'),
      JSON.stringify({
        client_version: '0.153.4',
        models: [{ slug: 'gpt-5.6-luna', use_responses_lite: true }],
      }),
    );
    let service: { dispose: () => void } | undefined,
      hang = false;
    let lastChild: any;
    const prompts: string[] = [];
    vi.mocked(spawn).mockImplementation((_bin, args) => {
      const child = Object.assign(new EventEmitter(), {
        stdout: new PassThrough(),
        stderr: new PassThrough(),
        stdin: new PassThrough(),
        kill: vi.fn(),
      });
      lastChild = child;
      if (args?.includes('app-server'))
        child.stdin.on('data', (chunk) => {
          for (const line of chunk.toString().trim().split('\n')) {
            const request = JSON.parse(line);
            if (!request.id) continue;
            const result =
              request.id === 1
                ? {}
                : request.id === 2
                  ? { config: { model_provider: 'openai' } }
                  : request.id === 3
                    ? { requirements: null }
                    : {
                        data: [
                          {
                            model: 'gpt-5.6-luna',
                            hidden: false,
                            supportedReasoningEfforts: [{ reasoningEffort: 'low' }],
                          },
                        ],
                      };
            Promise.resolve().then(() =>
              child.stdout.write(JSON.stringify({ id: request.id, result }) + '\n'),
            );
          }
        });
      else {
        let input = '';
        child.stdin.on('data', (chunk) => (input += chunk));
        child.stdin.on('finish', () => {
          prompts.push(input);
          if (hang && !args?.includes('--version')) return;
          child.stdout.write(
            args?.includes('--version')
              ? 'codex-cli 0.153.4'
              : '{"type":"turn.started"}\n{"type":"item.completed","item":{"type":"agent_message","text":"OK"}}\n{"type":"turn.completed"}\n',
          );
          child.emit('close', 0);
        });
      }
      return child as never;
    });
    try {
      const { configService } = await import('./configService');
      configService.saveConfig({
        agents: {
          managerProvider: 'codex',
          checkProviderOnStartup: false,
          binaries: { codex: bin },
        },
      } as never);
      configService.reloadConfig();
      const runtime = await import('./providerReadinessRuntime');
      service = runtime.providerReadinessService;
      runtime.startProviderReadiness();
      runtime.startProviderReadiness();
      await vi.advanceTimersByTimeAsync(3000);
      expect(spawn).not.toHaveBeenCalled();
      expect(await runtime.providerReadinessService.check('codex')).toEqual({
        state: 'responding',
        checkedAt: expect.any(Number),
      });
      expect(spawn).toHaveBeenCalledTimes(3);
      expect(prompts).toEqual(['', 'Reply OK.']);
      expect(vi.mocked(spawn).mock.calls.every(([executable]) => executable === bin)).toBe(true);
      expect(vi.mocked(spawn).mock.calls[2][1]).toContain('gpt-5.6-luna');
      configService.saveConfig({
        agents: { binaries: { codex: other }, checkProviderOnStartup: true },
      } as never);
      hang = true;
      const pending = runtime.providerReadinessService.check('codex');
      await vi.advanceTimersByTimeAsync(1);
      expect(vi.mocked(spawn).mock.calls.at(-1)?.[0]).toBe(other);
      configService.saveConfig({ agents: { checkProviderOnStartup: false } } as never);
      expect(await pending).toEqual({ state: 'unchecked' });
      expect(lastChild.kill).toHaveBeenCalledWith('SIGKILL');
      owner.pid = null;
      const count = vi.mocked(spawn).mock.calls.length;
      expect(await runtime.providerReadinessService.check('codex')).toEqual({
        state: 'unsupported',
      });
      expect(spawn).toHaveBeenCalledTimes(count);
      owner.pid = 4242;
      hang = false;
      configService.saveConfig({ agents: { checkProviderOnStartup: true } } as never);
      runtime.providerReadinessService.dispose();
      vi.resetModules();
      const fresh = await import('./providerReadinessRuntime');
      service = fresh.providerReadinessService;
      fresh.startProviderReadiness();
      await vi.advanceTimersByTimeAsync(3000);
      expect(fresh.providerReadinessService.read('codex').state).toBe('responding');
      expect(spawn).toHaveBeenCalledTimes(count + 3);
    } finally {
      service?.dispose();
      owner.pid = 4242;
      vi.useRealTimers();
      vi.unstubAllEnvs();
      fs.rmSync(root, { recursive: true, force: true });
    }
  },
);
