import { EventEmitter } from 'events';
import { PassThrough } from 'stream';
import * as fs from 'fs';
import * as path from 'path';
import { expect, it, vi } from 'vitest';
vi.mock('./remoteServer', () => ({ isRemoteClientMode: () => false }));
vi.mock('./claudemonDaemon', () => ({
  CLAUDEMON_API_URL: 'http://unused',
  getClaudemonReadinessOwner: () => 4242,
}));
vi.mock('./claudeResolver', () => ({ claudeBaseArgv: () => ['unused'] }));
vi.mock('child_process', () => ({ spawn: vi.fn() }));
import { spawn } from 'child_process';
it('persisted opt-out reaches startup; manual check uses saved binary and tiny prompt', async () => {
  const root = fs.mkdtempSync(path.join(process.cwd(), '.readiness-config-test-'));
  vi.stubEnv('XDG_CONFIG_HOME', root);
  vi.useFakeTimers();
  const bin = path.join(root, 'claude-native-fixture');
  fs.writeFileSync(bin, Buffer.from('7f454c46', 'hex'));
  let service: { dispose: () => void } | undefined;
  const prompts: string[] = [];
  try {
    vi.mocked(spawn).mockImplementation((_bin, args) => {
      const child = Object.assign(new EventEmitter(), {
        stdout: new PassThrough(),
        stderr: new PassThrough(),
        stdin: new PassThrough(),
        kill: vi.fn(),
      });
      let prompt = '';
      child.stdin.on('data', (chunk) => {
        prompt += chunk;
      });
      child.stdin.on('finish', () => {
        prompts.push(prompt);
        child.stdout.write(
          args?.includes('--help')
            ? '--safe-mode --tools --strict-mcp-config --no-session-persistence --system-prompt --disable-slash-commands --output-format'
            : '{"type":"result","is_error":false,"result":"OK"}',
        );
        child.emit('close', 0);
      });
      return child as never;
    });
    const { configService } = await import('./configService');
    expect(configService.getConfig().agents.checkProviderOnStartup).toBe(true);
    configService.saveConfig({
      agents: {
        managerProvider: 'claude',
        checkProviderOnStartup: false,
        binaries: { claude: bin },
      },
    } as never);
    configService.reloadConfig();
    expect(fs.readFileSync(path.join(root, 'workspacer/config.yaml'), 'utf8')).toContain(
      'checkProviderOnStartup: false',
    );
    const { providerReadinessService, startProviderReadiness } =
      await import('./providerReadinessRuntime');
    service = providerReadinessService;
    startProviderReadiness();
    startProviderReadiness();
    await vi.advanceTimersByTimeAsync(60000);
    expect(spawn).not.toHaveBeenCalled();
    expect(await providerReadinessService.check('claude')).toEqual({
      state: 'responding',
      checkedAt: expect.any(Number),
    });
    expect(spawn).toHaveBeenCalledTimes(2);
    expect(vi.mocked(spawn).mock.calls.every(([executable]) => executable === bin)).toBe(true);
    expect(prompts).toEqual(['', 'Reply OK.']);
    configService.saveConfig({ agents: { checkProviderOnStartup: true } } as never);
    configService.reloadConfig();
    expect(configService.getConfig().agents.checkProviderOnStartup).toBe(true);
    expect(providerReadinessService.read('claude')).toEqual({ state: 'unchecked' });
    providerReadinessService.dispose();
    vi.resetModules();
    const fresh = await import('./providerReadinessRuntime');
    service = fresh.providerReadinessService;
    fresh.startProviderReadiness();
    await vi.advanceTimersByTimeAsync(2000);
    expect(spawn).toHaveBeenCalledTimes(4);
    expect(fresh.providerReadinessService.read('claude').state).toBe('responding');
    const { configService: freshConfig } = await import('./configService');
    freshConfig.saveConfig({ claude: { transport: 'pty' } } as never);
    expect(await fresh.providerReadinessService.check('claude')).toEqual({ state: 'unsupported' });
    expect(spawn).toHaveBeenCalledTimes(4);
  } finally {
    service?.dispose();
    vi.useRealTimers();
    vi.unstubAllEnvs();
    fs.rmSync(root, { recursive: true, force: true });
  }
});
