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
vi.mock('./claudeProfiles', () => ({
  claudeProfiles: { getProfile: (...a: unknown[]) => getProfile(...a) },
}));

vi.mock('./libraryService', () => ({ libraryService: { list: vi.fn(() => []) } }));

vi.mock('./agentProviders', () => ({
  resolveAgentBinary: vi.fn((provider: string) => `/bin/${provider}`),
  isAgentBinaryInstalled: vi.fn(() => true),
}));

vi.mock('./configService', () => ({ configService: { getConfig: () => ({}) } }));

vi.mock('./mcpConfig', () => ({
  MCP_FACADE_URL: 'http://127.0.0.1:0/mcp',
  managedFacadeInstructions: vi.fn(() => 'FACADE'),
  buildSessionMcpConfig: vi.fn(() => null),
}));

vi.mock('./claudemonDaemon', () => ({
  claudemonOverlayPath: () => '/overlay/settings.json',
  claudeSettingsOverlayEnabled: () => false,
}));

vi.mock('./supervisorSkill', () => ({ ensureSupervisorHome: vi.fn(() => '/home/super') }));
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
