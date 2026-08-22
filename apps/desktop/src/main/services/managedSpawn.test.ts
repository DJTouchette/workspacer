/**
 * Wire-shape contract for spawnManagedAgent — the ONE managed-provider dispatch
 * shared by the `claude:spawn` IPC handler and the `agents.spawn` hub
 * capability (standing project rule: both transports must go through here).
 *
 * Pins the codex headless ('stream') plumbing added with provider parity:
 *
 *   - codex + transport 'stream' sends `transport: 'stream'` in the
 *     spawn-managed payload AND stamps it in setSpawnMeta (the client's only
 *     way to tell headless from hybrid before the daemon frame arrives);
 *   - hybrid codex (no transport) and opencode/pi carry NO transport key —
 *     their daemon adapters don't accept one;
 *   - codex resume forwards resumeSessionId (the daemon rejoins the prior
 *     app-server thread), while the claude-stream-only extras
 *     (permissionMode/extraArgs/env) never leak into codex payloads;
 *   - on win32, codex+stream falls back to the rollout hybrid (PTY spawn),
 *     never spawn-managed.
 *
 * Strategy mirrors claudeSpawn.test.ts: mock every collaborator so only
 * spawnManagedAgent runs, and inspect the payload handed to
 * claudemonSessionClient.spawnManaged / .spawn.
 */

import { describe, it, expect, vi, beforeEach } from 'vitest';

const spawnManagedMock = vi.fn(async () => 'managed-session-id');
const spawnMock = vi.fn(async () => 'pty-session-id');
vi.mock('./claudemonSessionClient', () => ({
  claudemonSessionClient: {
    spawnManaged: (...a: unknown[]) => spawnManagedMock(...a),
    spawn: (...a: unknown[]) => spawnMock(...a),
  },
}));

const setSpawnMeta = vi.fn();
const ensureManagedSession = vi.fn();
vi.mock('./claudeSessionStore', () => ({
  claudeSessionStore: {
    setSpawnMeta: (...a: unknown[]) => setSpawnMeta(...a),
    ensureManagedSession: (...a: unknown[]) => ensureManagedSession(...a),
  },
}));

const getProfile = vi.fn(() => undefined as unknown);
const getProfiles = vi.fn(() => [{ id: 'default' }] as unknown[]);
vi.mock('./claudeProfiles', () => ({
  claudeProfiles: {
    getProfile: (...a: unknown[]) => getProfile(...a),
    getProfiles: (...a: unknown[]) => getProfiles(...a),
  },
}));

// listWithSecrets only — see the same mock in claudeSpawn.test.ts: `list()`
// masks MCP credentials, so a spawn path using it would authenticate with the
// literal __WKS_SECRET__.
vi.mock('./libraryService', () => ({ libraryService: { listWithSecrets: vi.fn(() => []) } }));

vi.mock('./agentProviders', () => ({
  resolveAgentBinary: vi.fn((provider: string) => `/bin/${provider}`),
  isAgentBinaryInstalled: vi.fn(() => true),
}));

// Mutable per-test config — reset in beforeEach, mutated by the supervisor
// full-access tests (the flag is config-resolved inside spawnManagedAgent).
let mockConfig: Record<string, unknown>;
vi.mock('./configService', () => ({ configService: { getConfig: () => mockConfig } }));

const managedFacadeInstructions = vi.fn(() => 'FACADE');
const facadeSessionMcpConfig = vi.fn(() => '/cfg/session-facade.json');
vi.mock('./mcpConfig', () => ({
  MCP_FACADE_URL: 'http://127.0.0.1:0/mcp',
  managedFacadeInstructions: (...a: unknown[]) => managedFacadeInstructions(...a),
  buildSessionMcpConfig: vi.fn(() => null),
  facadeSessionMcpConfig: (...a: unknown[]) => facadeSessionMcpConfig(...a),
  facadeUrlWithToken: (token: string) => `http://127.0.0.1:0/mcp?t=${token}`,
}));

const mintSessionFacadeToken = vi.fn(() => ({
  token: 'tok-abc',
  scope: 'view',
  created: '2026-01-01T00:00:00.000Z',
}));
vi.mock('./remoteTokens', () => ({
  mintSessionFacadeToken: (...a: unknown[]) => mintSessionFacadeToken(...a),
  // Imported by the REAL fullAccessGrants module (the config-resolved grant
  // formula under test); never called by a spawn.
  reconcileSessionFacadeGrants: vi.fn(() => 0),
}));

vi.mock('./claudemonDaemon', () => ({
  claudemonOverlayPath: () => '/overlay/settings.json',
  claudeSettingsOverlayEnabled: () => false,
}));

vi.mock('./supervisorSkill', () => ({
  ensureSupervisorHome: vi.fn(() => '/home/super'),
  installSupervisorSkill: vi.fn(),
}));
vi.mock('./managerSkills', () => ({ installManagerSkills: vi.fn() }));
vi.mock('./systemNotice', () => ({ notifySystem: vi.fn() }));

const { spawnManagedAgent } = await import('./managedSpawn');

type Payload = Record<string, unknown>;

/** Payload of the most recent spawnManaged call. */
function lastManaged(): Payload {
  return spawnManagedMock.mock.calls.at(-1)![0] as Payload;
}
/** Spawn metadata from the most recent setSpawnMeta call. */
function lastMeta(): Payload {
  return setSpawnMeta.mock.calls.at(-1)![1] as Payload;
}

beforeEach(() => {
  vi.clearAllMocks();
  mockConfig = {};
});

describe('spawnManagedAgent — codex headless (stream) wire shape', () => {
  it("codex + transport 'stream' sends transport in the payload AND stamps it in spawn meta", async () => {
    await spawnManagedAgent({ provider: 'codex', transport: 'stream', cwd: '/proj' });

    expect(spawnManagedMock).toHaveBeenCalledTimes(1);
    expect(lastManaged().provider).toBe('codex');
    expect(lastManaged().transport).toBe('stream');
    expect(lastMeta().provider).toBe('codex');
    expect(lastMeta().transport).toBe('stream');
  });

  it('hybrid codex (no transport) sends NO transport key anywhere', async () => {
    await spawnManagedAgent({ provider: 'codex', cwd: '/proj' });

    expect(lastManaged()).not.toHaveProperty('transport');
    expect(lastMeta()).not.toHaveProperty('transport');
  });

  it.each(['opencode', 'pi'] as const)(
    '%s never sends a transport key, even if a caller passes one',
    async (provider) => {
      await spawnManagedAgent({ provider, transport: 'stream', cwd: '/proj' });

      expect(lastManaged().provider).toBe(provider);
      expect(lastManaged()).not.toHaveProperty('transport');
      expect(lastMeta()).not.toHaveProperty('transport');
    },
  );

  it('codex resume forwards resumeSessionId and pins the session id to it', async () => {
    await spawnManagedAgent({
      provider: 'codex',
      transport: 'stream',
      cwd: '/proj',
      resumeSessionId: 'prior-life-id',
    });

    expect(lastManaged().resumeSessionId).toBe('prior-life-id');
    expect(lastManaged().sessionId).toBe('prior-life-id');
  });

  it('claude-stream extras (permissionMode/extraArgs/env) never leak into codex payloads', async () => {
    await spawnManagedAgent({
      provider: 'codex',
      transport: 'stream',
      cwd: '/proj',
      resumeSessionId: 'prior-life-id',
      permissionMode: 'plan', // claude-only knob — codex must drop it from the wire
    });

    const payload = lastManaged();
    expect(payload).not.toHaveProperty('permissionMode');
    expect(payload).not.toHaveProperty('extraArgs');
    expect(payload).not.toHaveProperty('env');
  });

  it('claude stream keeps its extras: permissionMode + resumeSessionId ride the payload', async () => {
    await spawnManagedAgent({
      provider: 'claude',
      transport: 'stream',
      cwd: '/proj',
      resumeSessionId: 'claude-prior',
    });

    const payload = lastManaged();
    // No wire transport key for claude: spawn-managed claude IS the stream
    // adapter (only codex needs the hybrid/headless discriminator) — but the
    // client-side meta still stamps 'stream' so the pane gates its Term off.
    expect(payload).not.toHaveProperty('transport');
    expect(payload.permissionMode).toBe('default');
    expect(payload.resumeSessionId).toBe('claude-prior');
    expect(lastMeta().transport).toBe('stream');
  });

  // The stream adapter's `yolo` IS `--dangerously-skip-permissions` on the
  // headless argv, and Claude refuses a live switch to bypassPermissions without
  // it. Recording it lets the composer route "Full access" to a restart instead
  // of a request the CLI will reject.
  it.each([
    ['bypass mode', { permissionMode: 'bypassPermissions' }, true],
    ['the legacy boolean', { skipPermissions: true }, true],
    ['a plain spawn', {}, false],
    ['plan mode', { permissionMode: 'plan' }, false],
  ])('claude stream records bypassAvailable=%s for %s', async (_n, opts, expected) => {
    await spawnManagedAgent({ provider: 'claude', transport: 'stream', cwd: '/proj', ...opts });

    expect(lastMeta().settings.bypassAvailable).toBe(expected);
    expect(lastManaged().yolo).toBe(expected);
  });

  it('managed providers with no bypass mode of their own leave bypassAvailable unset', async () => {
    await spawnManagedAgent({
      provider: 'codex',
      transport: 'stream',
      cwd: '/proj',
      skipPermissions: true,
    });

    expect(lastMeta().settings).not.toHaveProperty('bypassAvailable');
  });

  it('registers the managed session immediately so the pane never shows "no session"', async () => {
    await spawnManagedAgent({ provider: 'codex', transport: 'stream', cwd: '/proj' });
    expect(ensureManagedSession).toHaveBeenCalledWith('managed-session-id', '/proj');
  });
});

describe('spawnManagedAgent — win32 codex fallback', () => {
  it('codex + stream on win32 spawns the rollout hybrid (PTY), never spawn-managed', async () => {
    const realPlatform = process.platform;
    Object.defineProperty(process, 'platform', { value: 'win32' });
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    try {
      await spawnManagedAgent({ provider: 'codex', transport: 'stream', cwd: '/proj' });

      expect(spawnManagedMock).not.toHaveBeenCalled();
      expect(spawnMock).toHaveBeenCalledTimes(1);
      const payload = spawnMock.mock.calls[0][0] as Payload;
      expect(payload.rolloutProvider).toBe('codex');
      // The hybrid fallback stamps no stream transport — it IS a PTY session.
      expect(lastMeta()).not.toHaveProperty('transport');
    } finally {
      warn.mockRestore();
      Object.defineProperty(process, 'platform', { value: realPlatform });
    }
  });
});

describe('spawnManagedAgent — facade tool tiers', () => {
  it('codex + toolScope mints a token and carries it as a ?t= query on the facade URL', async () => {
    await spawnManagedAgent({ provider: 'codex', cwd: '/proj', toolScope: 'view' });

    expect(mintSessionFacadeToken).toHaveBeenCalledTimes(1);
    expect(mintSessionFacadeToken.mock.calls[0][1]).toBe('view');
    expect(lastManaged().mcp).toBe('http://127.0.0.1:0/mcp?t=tok-abc');
    expect(lastManaged().instructions).toBe('FACADE');
  });

  it('claude stream + facade carries the token in a per-session config file, never payload.mcp', async () => {
    await spawnManagedAgent({
      provider: 'claude',
      transport: 'stream',
      cwd: '/proj',
      toolScope: 'triage',
    });

    expect(mintSessionFacadeToken.mock.calls[0][1]).toBe('triage');
    // The config FILE rides extraArgs; the token itself must not be in argv.
    const extraArgs = lastManaged().extraArgs as string[];
    expect(extraArgs[extraArgs.indexOf('--mcp-config') + 1]).toBe('/cfg/session-facade.json');
    expect(extraArgs).toContain('--allowedTools');
    expect(extraArgs.join(' ')).not.toContain('tok-abc');
    expect(lastManaged()).not.toHaveProperty('mcp');
    expect(lastManaged().instructions).toBe('FACADE');
  });

  it('legacy mcpFacade still works and defaults to operator', async () => {
    await spawnManagedAgent({ provider: 'opencode', cwd: '/proj', mcpFacade: true });

    expect(mintSessionFacadeToken.mock.calls[0][1]).toBe('operator');
    expect(lastManaged().mcp).toBe('http://127.0.0.1:0/mcp?t=tok-abc');
  });

  it('pi gets instructions but no token (it has no MCP client to spend it on)', async () => {
    await spawnManagedAgent({ provider: 'pi', cwd: '/proj', supervisor: true });

    expect(mintSessionFacadeToken).not.toHaveBeenCalled();
    expect(lastManaged().instructions).toBe('FACADE');
  });

  it('no facade flags → no token, no mcp, no instructions', async () => {
    await spawnManagedAgent({ provider: 'opencode', cwd: '/proj' });

    expect(mintSessionFacadeToken).not.toHaveBeenCalled();
    expect(lastManaged()).not.toHaveProperty('mcp');
    expect(lastManaged()).not.toHaveProperty('instructions');
  });
});

describe('spawnManagedAgent — manager grants (config-resolved, stream path)', () => {
  it('a manager spawn mints role "manager" with the yolo grant resolved from config, not the caller flag', async () => {
    // Caller passes a stale fleetFullAccess:true (e.g. a respawn re-passing
    // the value frozen at the original spawn) but config has since revoked it:
    // the re-minted token must be ungranted.
    await spawnManagedAgent({
      provider: 'claude',
      transport: 'stream',
      cwd: '/proj',
      manager: true,
      toolScope: 'operator',
      fleetFullAccess: true,
    });

    expect(mintSessionFacadeToken.mock.calls[0][4]).toBe(false);
    expect(mintSessionFacadeToken.mock.calls[0][5]).toBe('manager');
  });

  it('agents.fleetFullAccess (or a per-project yolo) grants the manager token without any caller flag', async () => {
    mockConfig = { agents: { fleetFullAccess: true } };
    await spawnManagedAgent({
      provider: 'claude',
      transport: 'stream',
      cwd: '/proj',
      manager: true,
      toolScope: 'operator',
    });
    expect(mintSessionFacadeToken.mock.calls[0][4]).toBe(true);

    mockConfig = { projects: { '/proj/app': { yolo: true } } };
    await spawnManagedAgent({
      provider: 'claude',
      transport: 'stream',
      cwd: '/proj',
      manager: true,
      toolScope: 'operator',
    });
    expect(mintSessionFacadeToken.mock.calls[1][4]).toBe(true);
  });
});

describe('spawnManagedAgent — supervisor full access (config supervisor.fullAccess)', () => {
  it('setting on: a claude-stream supervisor spawns bypassed, with the yolo token grant and the full-access role note', async () => {
    mockConfig = { supervisor: { fullAccess: true } };

    await spawnManagedAgent({
      provider: 'claude',
      transport: 'stream',
      cwd: '/proj',
      supervisor: true,
    });

    expect(lastManaged().yolo).toBe(true);
    expect(lastManaged().permissionMode).toBe('bypassPermissions');
    expect(lastMeta().settings.permissionMode).toBe('bypassPermissions');
    // 5th mint arg = yoloAllowed — the grant the hub verifies before stamping
    // yoloGranted on the supervisor's own spawn_agent calls, so its workers'
    // skipPermissions requests are honored instead of clamped.
    expect(mintSessionFacadeToken.mock.calls[0][4]).toBe(true);
    // …and the injected role instructions tell it to actually request that.
    expect(managedFacadeInstructions).toHaveBeenCalledWith(
      true,
      'operator',
      expect.any(String),
      true,
    );
  });

  it('setting on: a managed-provider supervisor (opencode) runs yolo too', async () => {
    mockConfig = { supervisor: { fullAccess: true } };

    await spawnManagedAgent({ provider: 'opencode', cwd: '/proj', supervisor: true });

    expect(lastManaged().yolo).toBe(true);
    expect(lastMeta().settings.permissionMode).toBe('yolo');
    expect(mintSessionFacadeToken.mock.calls[0][4]).toBe(true);
  });

  it('setting off: the supervisor prompts as today — no bypass, no yolo grant', async () => {
    await spawnManagedAgent({
      provider: 'claude',
      transport: 'stream',
      cwd: '/proj',
      supervisor: true,
    });

    expect(lastManaged().yolo).toBe(false);
    expect(lastManaged().permissionMode).toBe('default');
    expect(mintSessionFacadeToken.mock.calls[0][4]).toBeUndefined();
    expect(managedFacadeInstructions).toHaveBeenCalledWith(
      true,
      'operator',
      expect.any(String),
      false,
    );
  });

  it('setting on leaves non-supervisor spawns untouched', async () => {
    mockConfig = { supervisor: { fullAccess: true } };

    await spawnManagedAgent({
      provider: 'claude',
      transport: 'stream',
      cwd: '/proj',
      toolScope: 'view',
    });

    expect(lastManaged().yolo).toBe(false);
    expect(mintSessionFacadeToken.mock.calls[0][4]).toBeUndefined();
  });
});

// ── Structured-result contract (spawn_agent resultSchema) ────────────────────
describe('spawnManagedAgent — resultSchema', () => {
  const schema = { type: 'object', properties: { commit: { type: 'string' } } };

  it('rides the first-turn instructions for a PLAIN (non-facade) managed worker', async () => {
    await spawnManagedAgent({ provider: 'opencode', cwd: '/proj', resultSchema: schema });
    const instructions = lastManaged().instructions as string;
    expect(instructions).toContain('wks-result');
    expect(instructions).toContain('"commit"');
    // No facade was asked for, so no facade role text and no mcp URL.
    expect(instructions).not.toContain('FACADE');
    expect(lastManaged().mcp).toBeUndefined();
  });

  it('JOINS the facade role note and the contract rather than dropping one', async () => {
    await spawnManagedAgent({
      provider: 'opencode',
      cwd: '/proj',
      toolScope: 'operator',
      resultSchema: schema,
    });
    const instructions = lastManaged().instructions as string;
    expect(instructions).toContain('FACADE');
    expect(instructions).toContain('wks-result');
    expect(lastManaged().mcp).toBeTruthy();
  });

  it('records the schema on the spawn meta so the finish wake can validate against it', async () => {
    await spawnManagedAgent({ provider: 'opencode', cwd: '/proj', resultSchema: schema });
    expect(lastMeta()).toMatchObject({ resultSchema: schema });
  });

  it('sends no instructions at all without a schema or the facade', async () => {
    await spawnManagedAgent({ provider: 'opencode', cwd: '/proj' });
    expect(lastManaged().instructions).toBeUndefined();
  });

  it('REFUSES a malformed schema instead of silently dropping the contract', async () => {
    await expect(
      spawnManagedAgent({ provider: 'opencode', cwd: '/proj', resultSchema: 42 as never }),
    ).rejects.toThrow(/JSON Schema object/);
    expect(spawnManagedMock).not.toHaveBeenCalled();
  });
});
