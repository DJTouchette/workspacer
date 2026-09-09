import { describe, it, expect, vi } from 'vitest';
import { prepareLaunchIntegration, validateLaunchPatch } from './launchIntegrations';

const manifest = {
  id: 'test.route',
  provides: ['test.route.prepare'],
  launchIntegration: {
    version: 1,
    agents: ['claude', 'codex'],
    prepareMethod: 'test.route.prepare',
  },
};
const context = { agent: 'codex', cwd: '/project', model: 'test-model', resume: true };
function deps(plugins: unknown[] = [manifest]) {
  return {
    list: vi.fn(async () => plugins),
    call: vi.fn(async (_method: string, _params: unknown): Promise<unknown> => ({
      env: { OPENAI_BASE_URL: 'http://127.0.0.1:8787/v1' },
      args: ['--config', 'openai_base_url="http://127.0.0.1:8787/v1"'],
    })),
    routing: vi.fn(async () => ({ id: 'openai' })),
  };
}

describe('optional launch integrations', () => {
  it('does no discovery or config probe when disabled', async () => {
    const d = deps();
    const base = { env: { CODEX_HOME: '/account' }, args: ['-c', 'model="m"'] };
    for (const id of [null, undefined])
      expect(await prepareLaunchIntegration(id, context, base, d)).toEqual(base);
    expect(d.list).not.toHaveBeenCalled();
    expect(d.routing).not.toHaveBeenCalled();
    expect(d.call).not.toHaveBeenCalled();
  });
  it('resolves routing under the selected account but sends only routing metadata to the plugin', async () => {
    const d = deps();
    const base = {
      bin: '/bin/codex',
      env: { CODEX_HOME: '/account', API_SECRET: 'private' },
      args: ['-c', 'model="m"'],
    };
    const prepared = await prepareLaunchIntegration(manifest.id, context, base, d);
    expect(d.routing).toHaveBeenCalledWith(['/bin/codex'], '/project', base.env, base.args);
    expect(d.call).toHaveBeenCalledWith('test.route.prepare', {
      version: 1,
      ...context,
      provider: { id: 'openai' },
    });
    expect(JSON.stringify(d.call.mock.calls)).not.toContain('private');
    expect(prepared.env).toEqual({ ...base.env, OPENAI_BASE_URL: 'http://127.0.0.1:8787/v1' });
    expect(prepared.args.slice(0, 2)).toEqual(base.args);
    expect(process.env.OPENAI_BASE_URL).not.toBe(prepared.env.OPENAI_BASE_URL);
  });
  it('does not probe Codex for Claude launches', async () => {
    const d = deps();
    await prepareLaunchIntegration(
      manifest.id,
      { ...context, agent: 'claude' },
      { env: {}, args: [] },
      d,
    );
    expect(d.routing).not.toHaveBeenCalled();
  });
  it.each(
    [
      [],
      [{ ...manifest, disabled: true }],
      [{ ...manifest, provides: [] }],
      [{ ...manifest, launchIntegration: { ...manifest.launchIntegration, agents: ['claude'] } }],
    ].map((plugins) => ({ plugins })),
  )('fails closed for unavailable contributions %#', async ({ plugins }) => {
    const d = deps(plugins);
    await expect(
      prepareLaunchIntegration(manifest.id, context, { env: {}, args: [], bin: '/bin/codex' }, d),
    ).rejects.toThrow('Restore the plugin/service');
    expect(d.call).not.toHaveBeenCalled();
  });
  it('propagates readiness errors instead of returning an unmodified launch', async () => {
    const d = deps();
    d.call.mockRejectedValueOnce(new Error('Proxy unavailable'));
    await expect(
      prepareLaunchIntegration(
        manifest.id,
        { ...context, agent: 'claude' },
        { env: {}, args: [] },
        d,
      ),
    ).rejects.toThrow('Proxy unavailable');
  });
  it.each([
    null,
    [],
    { cwd: '/elsewhere' },
    { executable: 'sh' },
    { env: null },
    { args: null },
    { env: { BAD: 1 } },
    { env: { BAD: 'nul\0' } },
    { args: ['nul\0'] },
    { args: Array(65).fill('a') },
    JSON.parse('{"env":{"__proto__":"x"}}'),
  ])('rejects malformed or overbroad patches %#', (value) => {
    expect(() => validateLaunchPatch(value)).toThrow();
  });
});
