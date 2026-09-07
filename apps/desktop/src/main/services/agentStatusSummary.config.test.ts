import { it, expect, vi } from 'vitest';
import * as fs from 'fs';
import * as path from 'path';
vi.mock('./claudemonDaemon', () => ({ CLAUDEMON_API_URL: 'http://127.0.0.1:0' }));
vi.mock('./claudeResolver', () => ({ claudeBaseArgv: () => ['fake-claude'] }));
it('setting -> persisted YAML -> live config -> real completion adapter keeps exact provider/model and null', async () => {
  const root = fs.mkdtempSync(path.join(process.cwd(), '.status-summary-test-'));
  vi.stubEnv('XDG_CONFIG_HOME', root);
  const modelText = JSON.stringify({
    activity: 'Worker reports checking tests',
    progress: null,
    blocker: null,
    nextStep: null,
    unknowns: [],
  });
  const fetchMock = vi.fn(async () => new Response(JSON.stringify({ ok: true, text: modelText })));
  vi.stubGlobal('fetch', fetchMock);
  try {
    const { configService } = await import('./configService');
    const { AgentStatusSummaryService } = await import('./agentStatusSummary');
    expect(configService.getConfig().agents.statusSummary).toEqual({
      enabled: true,
      provider: 'claude',
      model: 'haiku',
    });
    const service = new AgentStatusSummaryService({
      owningHub: 'test',
      config: () => configService.getConfig().agents.statusSummary,
      authorize: async () => {},
      source: async () => ({
        projection: 'agent-status-source/v1',
        sessionId: 's',
        throughSeq: 1,
        firstSeq: 1,
        headTruncated: false,
        tailTruncated: false,
        textTruncated: false,
        events: [{ seq: 1, kind: 'user_message', text: 'Task', timestamp: null }],
      }),
    });
    configService.saveConfig({
      agents: { statusSummary: { enabled: true, provider: 'claude', model: 'sonnet' } } as never,
    });
    configService.reloadConfig();
    expect((await service.summarize('s')).model).toBe('sonnet');
    let body = JSON.parse(
      (fetchMock.mock.calls[0] as unknown as [string, RequestInit])[1].body as string,
    );
    expect(body).toMatchObject({
      model: 'sonnet',
      argv: ['fake-claude'],
      no_tools: true,
      harness_default: false,
    });
    expect(fs.readFileSync(path.join(root, 'workspacer/config.yaml'), 'utf8')).toContain(
      'model: sonnet',
    );
    configService.saveConfig({ agents: { statusSummary: { model: null } } as never });
    configService.reloadConfig();
    expect(configService.getConfig().agents.statusSummary?.model).toBeNull();
    expect((await service.summarize('s')).model).toBeNull();
    body = JSON.parse(
      (fetchMock.mock.calls[1] as unknown as [string, RequestInit])[1].body as string,
    );
    expect(body).toMatchObject({ model: null, harness_default: true, no_tools: true });
    configService.saveConfig({ agents: { statusSummary: { enabled: false } } as never });
    configService.reloadConfig();
    expect((await service.summarize('s')).status).toBe('disabled');
    expect(fetchMock).toHaveBeenCalledTimes(2);
  } finally {
    vi.unstubAllGlobals();
    vi.unstubAllEnvs();
    fs.rmSync(root, { recursive: true, force: true });
  }
});
