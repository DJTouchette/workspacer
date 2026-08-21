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
const getProfiles = vi.fn(() => [] as unknown[]);
vi.mock('./claudeProfiles', async (importOriginal) => ({
  // The scrub functions are pure — keep the real ones so profileGranted tests
  // exercise the actual configDir-keep/drop behavior, not a stub of it.
  ...(await importOriginal<typeof import('./claudeProfiles')>()),
  claudeProfiles: {
    getProfile: (...a: unknown[]) => getProfile(...a),
    getProfiles: (...a: unknown[]) => getProfiles(...a),
  },
}));

// ONLY listWithSecrets is mocked, deliberately: `list()` masks MCP env/headers
// to __WKS_SECRET__, and a spawn that used it would write that literal into the
// session's --mcp-config as the API token — a broken server, from a change that
// reads like a simplification. Switching this call site back to `list()` throws
// here instead of shipping.
const libraryList = vi.fn(() => [] as unknown[]);
vi.mock('./libraryService', () => ({
  libraryService: { listWithSecrets: (...a: unknown[]) => libraryList(...a) },
}));

vi.mock('./configService', () => ({
  configService: {
    getConfig: () => ({
      supervisor: { model: 'sup-model', summarizerModel: 'sonnet', pollSeconds: 30 },
    }),
  },
  // Needed by the REAL claudeProfiles module (its profilesFile path is
  // computed at import time) — the mock above pulls the original in.
  getConfigDir: () => '/tmp/wks-test-config',
}));

const installSupervisorSkill = vi.fn();
const ensureSupervisorHome = vi.fn(() => '/home/super');
vi.mock('./supervisorSkill', () => ({
  installSupervisorSkill: (...a: unknown[]) => installSupervisorSkill(...a),
  ensureSupervisorHome: (...a: unknown[]) => ensureSupervisorHome(...a),
}));
vi.mock('./managerSkills', () => ({ installManagerSkills: vi.fn() }));

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

const mintSessionFacadeToken = vi.fn(() => ({
  token: 'tok-123',
  scope: 'operator',
  created: '2026-01-01T00:00:00.000Z',
}));
vi.mock('./remoteTokens', () => ({
  mintSessionFacadeToken: (...a: unknown[]) => mintSessionFacadeToken(...a),
}));

const { spawnClaudeAgent } = await import('./claudeSpawn');

/** argv from the most recent claudemonSessionClient.spawn call. */
function lastArgv(): string[] {
  return (spawnMock.mock.calls.at(-1)![0] as { argv: string[] }).argv;
}
/** the full spawn options object from the most recent spawn call. */
function lastSpawn(): {
  argv: string[];
  cwd: string;
  env: Record<string, string>;
  sessionId: string;
} {
  return spawnMock.mock.calls.at(-1)![0] as {
    argv: string[];
    cwd: string;
    env: Record<string, string>;
    sessionId: string;
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  getProfile.mockReturnValue(undefined);
  libraryList.mockReturnValue([]);
  buildSessionMcpConfig.mockReturnValue({
    path: '/cfg/session-mcp/srv.json',
    toolNames: ['mcp__srv1'],
  });
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
      { id: 'prompt1', kind: 'prompt' }, // wrong kind
      { id: 'srv3', kind: 'mcp' }, // selected but no .mcp
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

  it('mints an operator session token for supervisor/mcpFacade and passes it to facadeSpawnArgs', async () => {
    await spawnClaudeAgent({ supervisor: true });

    expect(mintSessionFacadeToken).toHaveBeenCalledTimes(1);
    expect(mintSessionFacadeToken.mock.calls[0][1]).toBe('operator');
    const args = facadeSpawnArgs.mock.calls[0][0] as { token?: string; scope?: string };
    expect(args.token).toBe('tok-123');
    expect(args.scope).toBe('operator');
  });

  it('toolScope alone implies the facade and mints at that tier', async () => {
    await spawnClaudeAgent({ cwd: '/proj', toolScope: 'view', pluginTools: ['djtouchette.jira'] });

    expect(mintSessionFacadeToken).toHaveBeenCalledTimes(1);
    const [sessionId, scope, plugins] = mintSessionFacadeToken.mock.calls[0] as unknown as [
      string,
      string,
      string[] | undefined,
    ];
    expect(sessionId).toBeTruthy();
    expect(scope).toBe('view');
    expect(plugins).toEqual(['djtouchette.jira']);
    const args = facadeSpawnArgs.mock.calls[0][0] as { scope?: string };
    expect(args.scope).toBe('view');
    // The facade config rides argv exactly like the legacy facade path.
    const argv = lastArgv();
    expect(argv[argv.indexOf('--mcp-config') + 1]).toBe('/cfg/facade.json');
  });

  it('a supervisor stays operator even if a narrower toolScope is passed', async () => {
    await spawnClaudeAgent({ supervisor: true, toolScope: 'view' });

    expect(mintSessionFacadeToken.mock.calls[0][1]).toBe('operator');
  });

  it('a plain spawn (no facade flags) mints no token', async () => {
    await spawnClaudeAgent({ cwd: '/proj' });

    expect(mintSessionFacadeToken).not.toHaveBeenCalled();
  });

  it('a manager spawn mints its token with a profilesAllowed grant for every local profile', async () => {
    getProfiles.mockReturnValue([{ id: 'default' }, { id: 'work' }]);
    await spawnClaudeAgent({ cwd: '/home/u/Work', manager: true, toolScope: 'operator' });

    expect(mintSessionFacadeToken).toHaveBeenCalledTimes(1);
    expect(mintSessionFacadeToken.mock.calls[0][3]).toEqual(['default', 'work']);
    // No full-access unless the manager was spawned with it.
    expect(mintSessionFacadeToken.mock.calls[0][4]).toBe(false);
  });

  it('a full-access manager spawn also mints the yolo grant (5th arg true)', async () => {
    getProfiles.mockReturnValue([{ id: 'default' }]);
    await spawnClaudeAgent({
      cwd: '/home/u/Work',
      manager: true,
      toolScope: 'operator',
      fleetFullAccess: true,
    });

    expect(mintSessionFacadeToken.mock.calls[0][4]).toBe(true);
  });

  it('a non-manager facade spawn mints NO profile or yolo grant', async () => {
    getProfiles.mockReturnValue([{ id: 'default' }]);
    // fleetFullAccess is ignored without manager — a plain facade worker never
    // gets the grant even if the flag leaks in.
    await spawnClaudeAgent({ supervisor: true, fleetFullAccess: true });

    expect(mintSessionFacadeToken.mock.calls[0][3]).toBeUndefined();
    expect(mintSessionFacadeToken.mock.calls[0][4]).toBeUndefined();
  });
});

describe('spawnClaudeAgent — profileGranted (fleet-manager dispatch)', () => {
  const workProfile = {
    id: 'work',
    name: 'Work',
    configDir: '/accounts/work',
    extraArgs: ['--dangerously-skip-permissions', '--model', 'opus'],
    mcpItemIds: ['srv1'],
  };

  it('scrubProfileBypass alone drops the profile configDir (remote doctrine unchanged)', async () => {
    getProfile.mockReturnValue(workProfile);
    await spawnClaudeAgent({ cwd: '/proj', profileId: 'work', scrubProfileBypass: true });

    expect(lastSpawn().env.CLAUDE_CONFIG_DIR).toBeUndefined();
    expect(lastArgv()).not.toContain('--dangerously-skip-permissions');
  });

  it('a hub-stamped grant keeps the configDir but still strips the bypass args', async () => {
    getProfile.mockReturnValue(workProfile);
    await spawnClaudeAgent({
      cwd: '/proj',
      profileId: 'work',
      scrubProfileBypass: true,
      profileGranted: true,
    });

    expect(lastSpawn().env.CLAUDE_CONFIG_DIR).toBe('/accounts/work');
    const argv = lastArgv();
    expect(argv).not.toContain('--dangerously-skip-permissions');
    expect(argv[argv.indexOf('--model') + 1]).toBe('opus');
  });

  it('profileGranted without the scrub boundary changes nothing (local spawns already trust)', async () => {
    getProfile.mockReturnValue(workProfile);
    await spawnClaudeAgent({ cwd: '/proj', profileId: 'work', profileGranted: true });

    expect(lastSpawn().env.CLAUDE_CONFIG_DIR).toBe('/accounts/work');
    expect(lastArgv()).toContain('--dangerously-skip-permissions');
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
    const meta = setSpawnMeta.mock.calls[0][1] as {
      provider: string;
      settings: { permissionMode: string };
    };
    expect(meta.provider).toBe('claude');
    expect(meta.settings.permissionMode).toBe('default');
  });

  // Claude only lets a running session enter bypassPermissions if the process
  // was launched with --dangerously-skip-permissions, so the composer needs to
  // know whether this argv carried it — recorded from the same three inputs
  // buildClaudeArgv resolves the flag from.
  it.each([
    ['skipPermissions', { skipPermissions: true }, true],
    ['permissionMode bypassPermissions', { permissionMode: 'bypassPermissions' }, true],
    ['a plain spawn', {}, false],
    ['a non-bypass mode', { permissionMode: 'plan' }, false],
  ])('records bypassAvailable=%s for %s', async (_name, opts, expected) => {
    await spawnClaudeAgent({ cwd: '/proj', ...opts });
    const meta = setSpawnMeta.mock.calls[0][1] as { settings: { bypassAvailable?: boolean } };
    expect(meta.settings.bypassAvailable).toBe(expected);
    expect(lastArgv().includes('--dangerously-skip-permissions')).toBe(expected);
  });

  it('counts a profile that already pins --dangerously-skip-permissions', async () => {
    getProfile.mockReturnValue({ extraArgs: ['--dangerously-skip-permissions'] });

    await spawnClaudeAgent({ cwd: '/proj', profileId: 'p1' });

    const meta = setSpawnMeta.mock.calls[0][1] as { settings: { bypassAvailable?: boolean } };
    expect(meta.settings.bypassAvailable).toBe(true);
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
