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
 *   - a codex spawn that names NO transport resolves the configured default
 *     (codex.transport, shipped 'stream') here — this is THE choke point, so
 *     every entry point that forgets to fill the field still lands headless;
 *   - opencode/pi carry NO transport key — their daemon adapters don't accept
 *     one, and they have only one session shape;
 *   - codex resume forwards resumeSessionId (the daemon rejoins the prior
 *     app-server thread), while the claude-stream-only extras
 *     (permissionMode/extraArgs/env) never leak into codex payloads;
 *   - on win32, HEADLESS codex goes down the managed app-server path like
 *     everywhere else (no PTY is involved, so the ConPTY concerns behind the
 *     old unconditional pin don't apply); only an explicit 'pty' takes the
 *     rollout hybrid there.
 *
 * Strategy mirrors claudeSpawn.test.ts: mock every collaborator so only
 * spawnManagedAgent runs, and inspect the payload handed to
 * claudemonSessionClient.spawnManaged / .spawn.
 */

import { describe, it, expect, vi, beforeEach } from 'vitest';

// The spawn cwd pre-flight is real fs (lib/spawnCwd.ts) and these are wire-shape
// tests against paths like '/proj' that do not exist on disk. Keep the
// normalization real and stub only the assertion — its own behavior is pinned in
// lib/spawnCwd.test.ts, and the test below pins that this path still calls it.
const assertSpawnCwdMock = vi.fn();
vi.mock('../lib/spawnCwd', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../lib/spawnCwd')>()),
  assertSpawnCwd: (...a: unknown[]) => assertSpawnCwdMock(...a),
}));

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
  reconcileSessionFacadeGrants: vi.fn(() => []),
}));

vi.mock('./claudemonDaemon', () => ({
  claudemonOverlayPath: () => '/overlay/settings.json',
  claudeSettingsOverlayEnabled: () => false,
}));

const installSupervisorSkill = vi.fn();
const installManagerSkills = vi.fn();
vi.mock('./supervisorSkill', () => ({
  ensureSupervisorHome: vi.fn(() => '/home/super'),
  installSupervisorSkill: (...a: unknown[]) => installSupervisorSkill(...a),
}));
vi.mock('./managerSkills', () => ({
  installManagerSkills: (...a: unknown[]) => installManagerSkills(...a),
}));
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

/**
 * The pre-flight itself. claudemon answers 200 and registers the session id
 * BEFORE the child launches, so a cwd no process could run in used to reach the
 * user as an agent card whose session was already stopped and whose every
 * message came back 409 — the "it opens and nothing goes through" report. The
 * refusal has to happen on this side of the spawn, so the wiring is pinned here
 * and the rule itself in lib/spawnCwd.test.ts.
 */
describe('spawnManagedAgent — spawn cwd pre-flight', () => {
  it('checks the RESOLVED cwd before spawning', async () => {
    assertSpawnCwdMock.mockClear();
    await spawnManagedAgent({ provider: 'codex', cwd: '/proj' });
    expect(assertSpawnCwdMock).toHaveBeenCalledWith('/proj');
  });

  it('a refused cwd fails the spawn instead of minting a session', async () => {
    assertSpawnCwdMock.mockClear();
    spawnManagedMock.mockClear();
    assertSpawnCwdMock.mockImplementationOnce(() => {
      throw new Error('Working directory "~" is not an existing directory.');
    });
    await expect(spawnManagedAgent({ provider: 'codex', cwd: '/proj' })).rejects.toThrow(
      'not an existing directory',
    );
    expect(spawnManagedMock).not.toHaveBeenCalled();
  });
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

  // THE default. Most of the fleet spawns codex through a path that names no
  // transport at all (a bus dispatch, a respawn, the Fleet Manager), and this
  // is the one place that decides what that means.
  it('codex with NO transport resolves the shipped default (headless) on the wire AND in spawn meta', async () => {
    await spawnManagedAgent({ provider: 'codex', cwd: '/proj' });

    expect(lastManaged().transport).toBe('stream');
    expect(lastMeta().transport).toBe('stream');
  });

  it('config codex.transport=pty flips that default to the hybrid', async () => {
    mockConfig = { codex: { transport: 'pty' } };
    await spawnManagedAgent({ provider: 'codex', cwd: '/proj' });

    expect(lastManaged().transport).toBe('pty');
    expect(lastMeta().transport).toBe('pty');
  });

  it('an explicit hybrid request beats a headless config default', async () => {
    mockConfig = { codex: { transport: 'stream' } };
    await spawnManagedAgent({ provider: 'codex', transport: 'pty', cwd: '/proj' });

    expect(lastManaged().transport).toBe('pty');
    expect(lastMeta().transport).toBe('pty');
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

/**
 * Windows was pinned to the rollout hybrid UNCONDITIONALLY, so `transport:
 * 'stream'` there was a warning and a downgrade — which is what left the same
 * user with GUI-only codex on Linux and a TUI+viewer pair on Windows. The ws
 * app-server was chosen precisely because plain-TCP ws works on Windows
 * (codex.rs module header), and a headless session spawns no PTY at all, so
 * none of the ConPTY reasons behind that pin apply to it.
 *
 * NOT runtime-verified on Windows from this machine — the safety net is
 * claudemon's: if `codex app-server` fails to come up, run_session degrades the
 * session to the rollout hybrid in place, resets the transport stamp to 'pty'
 * so the pane grows its Term view back, and pushes a ⚠️ notice into the
 * conversation. Loud and working, rather than a dead pane.
 */
describe('spawnManagedAgent — win32 codex', () => {
  const onWin32 = async (fn: () => Promise<void>) => {
    const realPlatform = process.platform;
    Object.defineProperty(process, 'platform', { value: 'win32' });
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    try {
      await fn();
    } finally {
      warn.mockRestore();
      Object.defineProperty(process, 'platform', { value: realPlatform });
    }
  };

  it('codex + stream on win32 takes the managed app-server path, like every other platform', async () => {
    await onWin32(async () => {
      await spawnManagedAgent({ provider: 'codex', transport: 'stream', cwd: '/proj' });

      expect(spawnMock).not.toHaveBeenCalled();
      expect(spawnManagedMock).toHaveBeenCalledTimes(1);
      expect(lastManaged().transport).toBe('stream');
      expect(lastMeta().transport).toBe('stream');
    });
  });

  it('a DEFAULTED codex spawn on win32 is headless too — the default is the whole point', async () => {
    await onWin32(async () => {
      await spawnManagedAgent({ provider: 'codex', cwd: '/proj' });

      expect(spawnMock).not.toHaveBeenCalled();
      expect(lastManaged().transport).toBe('stream');
    });
  });

  it('an explicit hybrid on win32 still takes the rollout hybrid (PTY spawn)', async () => {
    await onWin32(async () => {
      await spawnManagedAgent({ provider: 'codex', transport: 'pty', cwd: '/proj' });

      expect(spawnManagedMock).not.toHaveBeenCalled();
      expect(spawnMock).toHaveBeenCalledTimes(1);
      const payload = spawnMock.mock.calls[0][0] as Payload;
      expect(payload.rolloutProvider).toBe('codex');
      // It IS a PTY session, and now says so rather than leaving the field
      // absent — with codex defaulting to headless, an absent transport would
      // read as the default rather than as this.
      expect(lastMeta().transport).toBe('pty');
    });
  });

  it('config codex.transport=pty on win32 also lands on the rollout hybrid', async () => {
    await onWin32(async () => {
      mockConfig = { codex: { transport: 'pty' } };
      await spawnManagedAgent({ provider: 'codex', cwd: '/proj' });

      expect(spawnManagedMock).not.toHaveBeenCalled();
      expect(spawnMock).toHaveBeenCalledTimes(1);
    });
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
      expect.objectContaining({
        supervisor: true,
        scope: 'operator',
        sessionId: expect.any(String),
        fullAccess: true,
      }),
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
      expect.objectContaining({
        supervisor: true,
        scope: 'operator',
        sessionId: expect.any(String),
        fullAccess: false,
      }),
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

// ── The first message (spawn_agent / agents.spawn `message`) ─────────────────
//
// The dispatch prompt rides the SPAWN. Two-call dispatch has a real window:
// claudemon's `register_managed` marks the row `Input` while attaching no
// wrapper, and the provider driver only registers its prompt channel after the
// spawn handler has answered 200 — so a message posted in between comes back
// 404 and the worker sits with no task.
describe('spawnManagedAgent — firstMessage', () => {
  it('rides the spawn payload as its OWN field', async () => {
    await spawnManagedAgent({ provider: 'opencode', cwd: '/proj', firstMessage: 'ship the thing' });
    expect(lastManaged().firstMessage).toBe('ship the thing');
  });

  it('is NOT folded into instructions — instructions never start a turn', async () => {
    const schema = { type: 'object', properties: { commit: { type: 'string' } } };
    await spawnManagedAgent({
      provider: 'opencode',
      cwd: '/proj',
      toolScope: 'operator',
      resultSchema: schema,
      firstMessage: 'ship the thing',
    });
    // Both reach the worker, on separate fields, and the daemon prepends the
    // instructions to the prompt — so the contract lands ahead of the task in
    // one turn. Folding the task into `instructions` would leave it parked in
    // the adapter's pending-instructions slot waiting for a prompt to prepend
    // itself to, which is the very thing it was meant to be.
    const instructions = lastManaged().instructions as string;
    expect(instructions).toContain('FACADE');
    expect(instructions).toContain('wks-result');
    expect(instructions).not.toContain('ship the thing');
    expect(lastManaged().firstMessage).toBe('ship the thing');
  });

  it('sends no firstMessage key when none was asked for', async () => {
    await spawnManagedAgent({ provider: 'opencode', cwd: '/proj' });
    expect(lastManaged().firstMessage).toBeUndefined();
  });

  it('rides the codex PTY-hybrid spawn too — the one path with no instructions channel', async () => {
    const orig = process.platform;
    // The rollout hybrid is reached by asking for it now, not by being on
    // Windows: win32 + headless takes the managed app-server path like
    // everywhere else. Both facts are pinned in the win32 describe above.
    Object.defineProperty(process, 'platform', { value: 'win32' });
    try {
      await spawnManagedAgent({
        provider: 'codex',
        transport: 'pty',
        cwd: '/proj',
        firstMessage: 'ship the thing',
      });
    } finally {
      Object.defineProperty(process, 'platform', { value: orig });
    }
    expect(spawnMock).toHaveBeenCalledTimes(1);
    expect((spawnMock.mock.calls.at(-1)![0] as Payload).firstMessage).toBe('ship the thing');
  });
});

/**
 * The Fleet Manager ROLE on codex. The IPC managed branch used to drop
 * `manager` entirely, so none of this happened: the session came up unflagged
 * (invisible to the wake router) and its token was minted with no dispatch
 * grants, so every worker it spawned was clamped. Pinned here because a manager
 * missing any one of these looks exactly like a working one.
 */
describe('spawnManagedAgent — a Fleet Manager on codex', () => {
  beforeEach(() => {
    mockConfig = { agents: { fleetFullAccess: true } };
  });

  it('marks the session a wake target (isSupervisor)', async () => {
    await spawnManagedAgent({
      provider: 'codex',
      transport: 'stream',
      cwd: '/home/u/Work',
      manager: true,
      toolScope: 'operator',
      fleetFullAccess: true,
    });
    // nudgeParentOnFinish refuses to wake a parent without this flag — it IS
    // the difference between a manager and a decorative one.
    expect(lastMeta()).toMatchObject({ isSupervisor: true, provider: 'codex' });
  });

  it('mints its token with the manager role, profile grants and the config yolo grant', async () => {
    await spawnManagedAgent({
      provider: 'codex',
      transport: 'stream',
      cwd: '/home/u/Work',
      manager: true,
      toolScope: 'operator',
      fleetFullAccess: true,
    });
    const [sessionId, scope, plugins, profiles, yolo, role] = mintSessionFacadeToken.mock.calls.at(
      -1,
    )! as unknown[];
    expect(typeof sessionId).toBe('string');
    expect(scope).toBe('operator');
    expect(plugins).toBeUndefined();
    // profilesAllowed: the hub verifies this and stamps profileGranted on the
    // worker spawn, which is how a manager dispatches under another account.
    expect(profiles).toEqual(['default']);
    // yoloAllowed: config-resolved (never the caller's flag) — without it every
    // dispatched worker's skipPermissions is clamped off by the hub.
    expect(yolo).toBe(true);
    expect(role).toBe('manager');
    // …and the facade actually attaches, with the token on the URL (codex
    // registers MCP servers by URL and cannot send headers).
    expect(String(lastManaged().mcp)).toContain('t=tok-abc');
  });

  it('installs its slash commands into codex’s skills dir, not claude’s', async () => {
    await spawnManagedAgent({
      provider: 'codex',
      transport: 'stream',
      cwd: '/home/u/Work',
      manager: true,
      toolScope: 'operator',
    });
    expect(installManagerSkills).toHaveBeenCalledWith('codex');
  });

  it('drops the yolo grant when config says so, without touching the role', async () => {
    mockConfig = { agents: { fleetFullAccess: false } };
    await spawnManagedAgent({
      provider: 'codex',
      transport: 'stream',
      cwd: '/home/u/Work',
      manager: true,
      toolScope: 'operator',
      // A stale fleetFullAccess from a respawn record must NOT resurrect a
      // revoked grant — the mint reads config, not this flag.
      fleetFullAccess: true,
    });
    const call = mintSessionFacadeToken.mock.calls.at(-1)! as unknown[];
    expect(call[4]).toBe(false);
    expect(call[5]).toBe('manager');
  });
});

/**
 * THE TRAP THIS BLOCK EXISTS FOR. The bug behind the supervisor model picker
 * was never the dropdown: `supervisor.model` was never READ on the managed spawn
 * path at all, so the setting looked correct in the UI and did nothing. Every
 * model setting added since has to be traced to the actual spawn call, which is
 * the payload handed to claudemonSessionClient.spawnManaged — so that is what
 * these assert, not the resolver in isolation.
 *
 * `transport: 'stream'` throughout because that is how the Fleet Manager
 * actually spawns (chat-first); a manager model that only arrived on some other
 * transport would be a picker writing config nobody reads.
 */
describe('spawnManagedAgent — the Fleet Manager’s own model reaches the spawn', () => {
  it('takes agents.managerModels for the harness the manager runs on', async () => {
    mockConfig = {
      agents: { managerProvider: 'codex', managerModels: { claude: 'opus', codex: 'gpt-5' } },
    };
    await spawnManagedAgent({
      provider: 'codex',
      transport: 'stream',
      cwd: '/proj',
      manager: true,
    });
    expect(lastManaged().model).toBe('gpt-5');
    // …and it is recorded, not just passed: the daemon can only report what it
    // was told, and the card/pill read it from there.
    expect((lastMeta().settings as Payload).model).toBe('gpt-5');
  });

  it('never crosses harnesses — a claude manager takes the claude entry', async () => {
    mockConfig = { agents: { managerModels: { claude: 'opus', codex: 'gpt-5' } } };
    await spawnManagedAgent({
      provider: 'claude',
      transport: 'stream',
      cwd: '/proj',
      manager: true,
    });
    expect(lastManaged().model).toBe('opus');
  });

  it('an explicit model from the caller still wins over the configured one', async () => {
    mockConfig = { agents: { managerModels: { codex: 'gpt-5' } } };
    await spawnManagedAgent({
      provider: 'codex',
      transport: 'stream',
      cwd: '/proj',
      manager: true,
      model: 'gpt-5.1-codex-max',
    });
    expect(lastManaged().model).toBe('gpt-5.1-codex-max');
  });

  it('unset leaves the model undefined — the harness picks its own default', async () => {
    mockConfig = { agents: { managerProvider: 'codex' } };
    await spawnManagedAgent({
      provider: 'codex',
      transport: 'stream',
      cwd: '/proj',
      manager: true,
    });
    expect(lastManaged().model).toBeUndefined();
  });

  it('a NON-manager spawn is unaffected by the manager model', async () => {
    mockConfig = { agents: { managerModels: { codex: 'gpt-5' } } };
    await spawnManagedAgent({ provider: 'codex', transport: 'stream', cwd: '/proj' });
    expect(lastManaged().model).toBeUndefined();
  });
});

/**
 * The digest workers a supervisor spawns used to be described to it with a model
 * and no PROVIDER — and `spawn_agent` with no provider spawns Claude. So a codex
 * supervisor dispatched Claude summarizers, and `supervisor.summarizerModel`'s
 * claude-only `'sonnet'` default was right only by accident. Naming the harness
 * is what makes a per-harness summarizer model mean anything.
 */
describe('spawnManagedAgent — supervisor digest workers follow their supervisor’s harness', () => {
  it('names the supervisor’s OWN provider and that harness’s summarizer model', async () => {
    mockConfig = {
      supervisor: {
        provider: 'codex',
        summarizerModel: 'sonnet',
        summarizerModels: { codex: 'gpt-5' },
        pollSeconds: 30,
      },
    };
    await spawnManagedAgent({
      provider: 'codex',
      transport: 'stream',
      cwd: '/proj',
      supervisor: true,
    });
    const opts = managedFacadeInstructions.mock.calls.at(-1)![0] as Payload;
    expect(opts.summarizerProvider).toBe('codex');
    expect(opts.summarizerModel).toBe('gpt-5');
    expect(opts.pollSeconds).toBe(30);
  });

  it("withholds the claude-shaped 'sonnet' from a codex supervisor rather than forwarding it", async () => {
    mockConfig = { supervisor: { provider: 'codex', summarizerModel: 'sonnet' } };
    await spawnManagedAgent({
      provider: 'codex',
      transport: 'stream',
      cwd: '/proj',
      supervisor: true,
    });
    const opts = managedFacadeInstructions.mock.calls.at(-1)![0] as Payload;
    expect(opts.summarizerProvider).toBe('codex');
    // Undefined = "omit model, let codex default" — the only value valid here.
    expect(opts.summarizerModel).toBeUndefined();
  });

  it("keeps 'sonnet' for a claude-stream supervisor, where it is servable", async () => {
    mockConfig = { supervisor: { summarizerModel: 'sonnet' } };
    await spawnManagedAgent({
      provider: 'claude',
      transport: 'stream',
      cwd: '/proj',
      supervisor: true,
    });
    const opts = managedFacadeInstructions.mock.calls.at(-1)![0] as Payload;
    expect(opts.summarizerProvider).toBe('claude');
    expect(opts.summarizerModel).toBe('sonnet');
  });
});
