/**
 * Tests for spawnClaudeAgent — the shared Claude PTY spawn body called by both
 * the `claude:spawn` IPC handler and the `agents.spawn` hub capability.
 *
 * Focus: the per-spawn Library MCP wiring (`mcpItemIds` → --mcp-config /
 * --strict-mcp-config / pre-allowed tools) that the hub path used to drop, plus
 * the facade-takes-precedence and permission-mode resolution behaviour. The
 * spawn is verified by capturing the argv handed to claudemonSessionClient.spawn.
 *
 * Strategy: mock every collaborator (session store, claudemon client, library,
 * config, supervisor skill, mcpConfig) so only spawnClaudeAgent + the real
 * buildClaudeArgv run. 'fs' is mocked so buildClaudeArgv's base argv resolves to
 * the ['claude'] fallback on Linux (and cwd falls back to home).
 */

import { describe, it, expect, vi, beforeEach } from 'vitest';

vi.mock('fs', async (importOriginal) => {
  const actual = await importOriginal<typeof import('fs')>();
  return { ...actual, existsSync: vi.fn().mockReturnValue(false) };
});

const spawnMock = vi.fn(async () => 'spawned-session-id');
vi.mock('./claudemonSessionClient', () => ({
  claudemonSessionClient: { spawn: (...a: unknown[]) => spawnMock(...a) },
}));

const setSpawnMeta = vi.fn();
vi.mock('./claudeSessionStore', () => ({
  claudeSessionStore: { setSpawnMeta: (...a: unknown[]) => setSpawnMeta(...a) },
}));

const getProfile = vi.fn(() => undefined as unknown);
vi.mock('./claudeProfiles', () => ({
  claudeProfiles: { getProfile: (...a: unknown[]) => getProfile(...a) },
}));

const libraryList = vi.fn(() => [] as unknown[]);
vi.mock('./libraryService', () => ({
  libraryService: { list: (...a: unknown[]) => libraryList(...a) },
}));

vi.mock('./configService', () => ({
  configService: {
    getConfig: () => ({ supervisor: { model: 'sup-model', summarizerModel: 'sonnet', pollSeconds: 30 } }),
  },
}));

const installSupervisorSkill = vi.fn();
const ensureSupervisorHome = vi.fn(() => '/home/super');
vi.mock('./supervisorSkill', () => ({
  installSupervisorSkill: (...a: unknown[]) => installSupervisorSkill(...a),
  ensureSupervisorHome: (...a: unknown[]) => ensureSupervisorHome(...a),
}));

const buildSessionMcpConfig = vi.fn();
const facadeSpawnArgs = vi.fn(() => ({
  mcpConfig: '/cfg/facade.json',
  allowedTools: ['mcp__workspacer'],
  appendSystemPrompt: 'ROLE',
}));
vi.mock('./mcpConfig', () => ({
  buildSessionMcpConfig: (...a: unknown[]) => buildSessionMcpConfig(...a),
  facadeSpawnArgs: (...a: unknown[]) => facadeSpawnArgs(...a),
}));

const { spawnClaudeAgent } = await import('./claudeSpawn');

/** argv from the most recent claudemonSessionClient.spawn call. */
function lastArgv(): string[] {
  return (spawnMock.mock.calls.at(-1)![0] as { argv: string[] }).argv;
}
/** the full spawn options object from the most recent spawn call. */
function lastSpawn(): { argv: string[]; cwd: string; env: Record<string, string>; sessionId: string } {
  return spawnMock.mock.calls.at(-1)![0] as { argv: string[]; cwd: string; env: Record<string, string>; sessionId: string };
}

beforeEach(() => {
  vi.clearAllMocks();
  getProfile.mockReturnValue(undefined);
  libraryList.mockReturnValue([]);
  buildSessionMcpConfig.mockReturnValue({ path: '/cfg/session-mcp/srv.json', toolNames: ['mcp__srv1'] });
});

describe('spawnClaudeAgent — Library MCP servers (mcpItemIds)', () => {
  it('emits --mcp-config, --strict-mcp-config and --allowedTools when mcpItemIds resolve to servers', async () => {
    libraryList.mockReturnValue([{ id: 'srv1', kind: 'mcp', mcp: { command: 'srv' } }]);

    await spawnClaudeAgent({ cwd: '/proj', mcpItemIds: ['srv1'] });

    const argv = lastArgv();
    const cfgIdx = argv.indexOf('--mcp-config');
    expect(cfgIdx).toBeGreaterThan(-1);
    expect(argv[cfgIdx + 1]).toBe('/cfg/session-mcp/srv.json');
    expect(argv).toContain('--strict-mcp-config');
    const allowIdx = argv.indexOf('--allowedTools');
    expect(allowIdx).toBeGreaterThan(-1);
    expect(argv[allowIdx + 1]).toBe('mcp__srv1');
  });

  it('resolves mcpItemIds against the library: only selected, kind=mcp items are passed to buildSessionMcpConfig', async () => {
    libraryList.mockReturnValue([
      { id: 'srv1', kind: 'mcp', mcp: { command: 'a' } }, // selected
      { id: 'srv2', kind: 'mcp', mcp: { command: 'b' } }, // NOT selected
      { id: 'prompt1', kind: 'prompt' },                  // wrong kind
      { id: 'srv3', kind: 'mcp' },                        // selected but no .mcp
    ]);

    await spawnClaudeAgent({ cwd: '/proj', mcpItemIds: ['srv1', 'srv3'] });

    expect(buildSessionMcpConfig).toHaveBeenCalledTimes(1);
    const [, servers] = buildSessionMcpConfig.mock.calls[0] as [string, Array<{ id: string }>];
    expect(servers.map((s) => s.id)).toEqual(['srv1']);
  });

  it('passes the pinned session id to buildSessionMcpConfig so the config file matches the transcript', async () => {
    libraryList.mockReturnValue([{ id: 'srv1', kind: 'mcp', mcp: { command: 'srv' } }]);

    await spawnClaudeAgent({ cwd: '/proj', resumeSessionId: 'fixed-id', mcpItemIds: ['srv1'] });

    const [id] = buildSessionMcpConfig.mock.calls[0] as [string];
    expect(id).toBe('fixed-id');
    expect(lastSpawn().sessionId).toBe('fixed-id');
  });

  it('does NOT emit --mcp-config when no mcpItemIds are given', async () => {
    await spawnClaudeAgent({ cwd: '/proj' });
    expect(buildSessionMcpConfig).not.toHaveBeenCalled();
    expect(lastArgv()).not.toContain('--mcp-config');
  });

  it('does NOT emit --mcp-config when mcpItemIds is an empty array', async () => {
    await spawnClaudeAgent({ cwd: '/proj', mcpItemIds: [] });
    expect(buildSessionMcpConfig).not.toHaveBeenCalled();
    expect(lastArgv()).not.toContain('--mcp-config');
  });

  it('emits no MCP args when buildSessionMcpConfig finds nothing valid (returns null)', async () => {
    libraryList.mockReturnValue([{ id: 'srv1', kind: 'mcp', mcp: { command: 'srv' } }]);
    buildSessionMcpConfig.mockReturnValue(null);

    await spawnClaudeAgent({ cwd: '/proj', mcpItemIds: ['srv1'] });

    const argv = lastArgv();
    expect(argv).not.toContain('--mcp-config');
    expect(argv).not.toContain('--strict-mcp-config');
    expect(argv).not.toContain('--allowedTools');
  });
});

describe('spawnClaudeAgent — facade takes precedence over Library MCP', () => {
  it('a facade worker (mcpFacade) uses facadeSpawnArgs and ignores mcpItemIds', async () => {
    libraryList.mockReturnValue([{ id: 'srv1', kind: 'mcp', mcp: { command: 'srv' } }]);

    await spawnClaudeAgent({ cwd: '/proj', mcpFacade: true, mcpItemIds: ['srv1'] });

    expect(buildSessionMcpConfig).not.toHaveBeenCalled();
    expect(facadeSpawnArgs).toHaveBeenCalledTimes(1);
    const argv = lastArgv();
    const cfgIdx = argv.indexOf('--mcp-config');
    expect(argv[cfgIdx + 1]).toBe('/cfg/facade.json');
    expect(argv).toContain('--append-system-prompt');
  });

  it('a supervisor installs the /supervise skill and uses the facade config', async () => {
    await spawnClaudeAgent({ supervisor: true, mcpItemIds: ['srv1'] });

    expect(installSupervisorSkill).toHaveBeenCalledTimes(1);
    expect(buildSessionMcpConfig).not.toHaveBeenCalled();
    expect(facadeSpawnArgs).toHaveBeenCalledTimes(1);
  });
});

describe('spawnClaudeAgent — permission mode + metadata', () => {
  it('maps skipPermissions to --dangerously-skip-permissions and records bypassPermissions on the snapshot', async () => {
    await spawnClaudeAgent({ cwd: '/proj', skipPermissions: true });

    expect(lastArgv()).toContain('--dangerously-skip-permissions');
    const meta = setSpawnMeta.mock.calls[0][1] as { settings: { permissionMode: string } };
    expect(meta.settings.permissionMode).toBe('bypassPermissions');
  });

  it('an explicit permissionMode wins over the skipPermissions default', async () => {
    await spawnClaudeAgent({ cwd: '/proj', permissionMode: 'plan' });

    const argv = lastArgv();
    const idx = argv.indexOf('--permission-mode');
    expect(argv[idx + 1]).toBe('plan');
    const meta = setSpawnMeta.mock.calls[0][1] as { settings: { permissionMode: string } };
    expect(meta.settings.permissionMode).toBe('plan');
  });

  it('records provider=claude and defaults permissionMode to default when nothing is passed', async () => {
    await spawnClaudeAgent({ cwd: '/proj' });
    const meta = setSpawnMeta.mock.calls[0][1] as { provider: string; settings: { permissionMode: string } };
    expect(meta.provider).toBe('claude');
    expect(meta.settings.permissionMode).toBe('default');
  });
});

describe('spawnClaudeAgent — profile + return value', () => {
  it("sets CLAUDE_CONFIG_DIR from the profile's configDir", async () => {
    getProfile.mockReturnValue({ configDir: '/cfgdir', extraArgs: ['--foo'] });

    await spawnClaudeAgent({ cwd: '/proj', profileId: 'p1' });

    expect(getProfile).toHaveBeenCalledWith('p1');
    expect(lastSpawn().env.CLAUDE_CONFIG_DIR).toBe('/cfgdir');
    expect(lastArgv()).toContain('--foo');
  });

  it('returns the session id from claudemonSessionClient.spawn', async () => {
    const id = await spawnClaudeAgent({ cwd: '/proj' });
    expect(id).toBe('spawned-session-id');
  });
});
