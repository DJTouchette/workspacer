/**
 * Tests for registerHubCapabilities — the bus/MCP capability registry the main
 * process exposes on the hub. These caps are the remote/web/MCP control surface,
 * so the regressions that matter are behavioural, not line-coverage:
 *
 *   - agents.spawn dispatches managed (Codex/OpenCode/Pi) providers through
 *     spawnManagedAgent and Claude through spawnClaudeAgent, forwarding
 *     mcpItemIds (which this path silently dropped once before);
 *   - the SECURITY sanitization: a bus caller can NEVER auto-bypass approvals
 *     (skipPermissions / bypassPermissions / yolo are forced off);
 *   - the read-only discovery caps (providers.listModels/checkAll) and the live
 *     control pass-throughs (claude.setModel/setPermissionMode/handoffBrief);
 *   - a throwing handler surfaces a structured Error to the caller rather than
 *     crashing.
 *
 * Strategy: mock ./hubClient so registerCapability records handlers into a map
 * we can invoke directly, and mock every collaborator so only the capability
 * bodies run.
 */

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import * as os from 'os';
import * as path from 'path';
import * as fs from 'fs';

// Capture every registered capability handler so tests can invoke them directly.
const registered = new Map<string, (params: unknown) => unknown>();
vi.mock('./hubClient', () => ({
  registerCapability: (method: string, handler: (params: unknown) => unknown) => {
    registered.set(method, handler);
  },
}));

// Keep catalog delegation OFF so `cat`-registered caps register through the real
// registerCapability too (the default env has DELEGATE_CATALOG_TO_BRAIN = true,
// which no-ops them). Not strictly needed for the caps under test — they all use
// registerCapability directly — but keeps the registry complete.
vi.mock('./brainDelegation', () => ({ DELEGATE_CATALOG_TO_BRAIN: false }));

const spawnManagedAgent = vi.fn(async () => 'managed-session-id');
vi.mock('./managedSpawn', () => ({
  spawnManagedAgent: (...a: unknown[]) => spawnManagedAgent(...a),
}));

const spawnClaudeAgent = vi.fn(async () => 'claude-session-id');
vi.mock('./claudeSpawn', () => ({ spawnClaudeAgent: (...a: unknown[]) => spawnClaudeAgent(...a) }));

const clientMock = {
  message: vi.fn(async () => ({ ok: true })),
  setPermissionMode: vi.fn(async () => ({ ok: true, mode: 'plan' })),
  setModel: vi.fn(async () => ({ ok: true })),
  handoffBrief: vi.fn(async () => ({ path: '/brief.md' })),
  listProviderModels: vi.fn(async () => ['m1', 'm2']),
};
vi.mock('./claudemonSessionClient', () => ({ claudemonSessionClient: clientMock }));

const notePermissionMode = vi.fn();
const getAllSnapshots = vi.fn(() => [] as unknown[]);
vi.mock('./claudeSessionStore', () => ({
  claudeSessionStore: {
    notePermissionMode: (...a: unknown[]) => notePermissionMode(...a),
    getAllSnapshots: (...a: unknown[]) => getAllSnapshots(...a),
    getSnapshot: vi.fn(),
  },
}));

const checkAllProviders = vi.fn(async () => ({ codex: true }));
const resolveAgentBinary = vi.fn(() => '/bin/codex');
vi.mock('./agentProviders', () => ({
  checkAllProviders: (...a: unknown[]) => checkAllProviders(...a),
  resolveAgentBinary: (...a: unknown[]) => resolveAgentBinary(...a),
}));

const getConfig = vi.fn(() => ({ agents: { binaries: { codex: '/custom/codex' } } }));
// A real on-disk config dir: the confinement helpers canonicalize through the
// filesystem, and the config-secret deny-list below only means anything if the
// dir it guards actually exists.
const cfg = vi.hoisted(() => {
  const nodeFs = require('fs') as typeof import('fs');
  const nodeOs = require('os') as typeof import('os');
  const nodePath = require('path') as typeof import('path');
  return {
    dir: nodeFs.realpathSync(nodeFs.mkdtempSync(nodePath.join(nodeOs.tmpdir(), 'wks-cap-cfg-'))),
  };
});
const getConfigDirMock = vi.fn(() => cfg.dir);
vi.mock('./configService', () => ({
  configService: {
    getConfig: (...a: unknown[]) => getConfig(...a),
    reloadConfig: vi.fn(),
    getConfigPath: vi.fn(),
    saveConfig: vi.fn(),
  },
  getConfigDir: (...a: unknown[]) => getConfigDirMock(...a),
}));

// Handoff brief authored path — used by claude.handoffAgentBrief.
vi.mock('./agentHandoff', () => ({
  agentHandoffBrief: vi.fn(async () => ({ path: '/agent-brief.md' })),
}));

// The rest are only referenced inside handlers we do not invoke; mock them so
// importing hubCapabilities does not pull in Electron/native plumbing.
// Notification instances record their listeners so a test can fire the click
// handler — that handler is the openExternal sink under test.
const notificationHandlers = new Map<string, (...a: unknown[]) => void>();
const openExternal = vi.fn(async () => {});
vi.mock('electron', () => {
  // A plain function, not an arrow implementation: the capability calls
  // `new Notification(...)`, which an arrow can't service.
  const NotificationMock = vi.fn(function (this: Record<string, unknown>) {
    this.show = vi.fn();
    this.on = (event: string, cb: (...a: unknown[]) => void) => {
      notificationHandlers.set(event, cb);
    };
  });
  (NotificationMock as unknown as { isSupported: () => boolean }).isSupported = () => true;
  return { Notification: NotificationMock, shell: { openExternal } };
});
vi.mock('./claudeProfiles', () => ({ claudeProfiles: {} }));
vi.mock('../lib/appIcon', () => ({ appIconPath: () => undefined }));
vi.mock('./claudeModels', () => ({ listClaudeModels: vi.fn(() => []) }));
const libraryMock = { list: vi.fn(() => []), save: vi.fn(), remove: vi.fn() };
vi.mock('./libraryService', () => ({ libraryService: libraryMock }));
vi.mock('./agentNotifier', () => ({
  agentNotifier: { postInApp: vi.fn(), focusAgent: vi.fn(), focusWindow: vi.fn() },
}));
vi.mock('./sessionService', () => ({ sessionService: {} }));
vi.mock('./sessionHistory', () => ({ sessionHistory: {} }));
vi.mock('./layoutService', () => ({ layoutService: {} }));
vi.mock('./claudeSessionList', () => ({ listClaudeSessionsForDir: vi.fn() }));
const listRecentSessions = vi.fn(async () => [{ sessionId: 's1', provider: 'claude' }]);
vi.mock('./recentSessions', () => ({ listRecentSessions: () => listRecentSessions() }));
vi.mock('./fileService', () => ({
  readTextFile: vi.fn(),
  writeTextFile: vi.fn(),
  listDir: vi.fn(() => ({ path: '', entries: [] })),
}));
vi.mock('./fileWatchService', () => ({ startWatch: vi.fn(), stopWatch: vi.fn() }));
vi.mock('./searchService', () => ({
  searchProject: vi.fn(() => ({ results: [], truncated: false })),
}));
// `workRoot` is part of the mock because git.diff's path guard consults it:
// gitService runs every command from the work-tree toplevel, so that — not the
// caller's cwd — is what a `path` is resolved against. Default it to the cwd
// (repo root == agent cwd); the nested-cwd test overrides it.
const workRootFor = vi.fn(async (cwd: string): Promise<string | null> => cwd);
vi.mock('./gitService', () => ({
  status: vi.fn(async () => ({ branch: 'main', files: [] })),
  workRoot: (cwd: string) => workRootFor(cwd),
  diff: vi.fn(async () => ''),
  numstat: vi.fn(async () => []),
  stage: vi.fn(async () => ''),
  unstage: vi.fn(async () => ''),
  commit: vi.fn(async () => 'committed'),
  push: vi.fn(async () => 'pushed'),
}));
vi.mock('./terminalShare', () => ({}));
vi.mock('./supervisorSkill', () => ({ ensureSupervisorHome: vi.fn(() => '/home/super') }));

const { registerHubCapabilities } = await import('./hubCapabilities');
const { readTextFile, writeTextFile } = await import('./fileService');
const { searchProject } = await import('./searchService');
const gitMock = await import('./gitService');

/** Invoke a registered capability by method name. */
function call(method: string, params?: unknown): unknown {
  const handler = registered.get(method);
  if (!handler) throw new Error(`capability not registered: ${method}`);
  return handler(params);
}

beforeEach(() => {
  vi.clearAllMocks();
  registered.clear();
  clientMock.setPermissionMode.mockResolvedValue({ ok: true, mode: 'plan' });
  registerHubCapabilities();
});

describe('registerHubCapabilities — registration', () => {
  it('registers the core control + discovery capabilities', () => {
    for (const method of [
      'agents.spawn',
      'agents.sendMessage',
      'providers.listModels',
      'providers.checkAll',
      'claude.setModel',
      'claude.setPermissionMode',
      'claude.handoffBrief',
    ]) {
      expect(registered.has(method), `missing ${method}`).toBe(true);
    }
  });

  // sessions.recent is what makes the web client's Sessions pane non-empty:
  // sessions.snapshots only covers LIVE sessions, so the resumable list has to
  // come from the daemon-backed enrichment in recentSessions.ts. It must be a
  // real bus method (not a `cat`-delegated one) because that enrichment reads
  // main's own history DB and local transcripts.
  it('serves the daemon session list over the bus, delegating to listRecentSessions', async () => {
    expect(registered.has('sessions.recent')).toBe(true);
    await expect(call('sessions.recent')).resolves.toEqual([
      { sessionId: 's1', provider: 'claude' },
    ]);
    expect(listRecentSessions).toHaveBeenCalledTimes(1);
  });
});

describe('agents.spawn — dispatch', () => {
  it('routes a managed provider (codex) through spawnManagedAgent, not spawnClaudeAgent', async () => {
    const res = await call('agents.spawn', {
      provider: 'codex',
      cwd: '/proj',
      model: 'o1',
      effort: 'high',
    });

    expect(spawnManagedAgent).toHaveBeenCalledTimes(1);
    expect(spawnClaudeAgent).not.toHaveBeenCalled();
    const arg = spawnManagedAgent.mock.calls[0][0] as {
      provider: string;
      cwd: string;
      model: string;
    };
    expect(arg.provider).toBe('codex');
    expect(arg.cwd).toBe('/proj');
    expect(arg.model).toBe('o1');
    expect(res).toEqual({ sessionId: 'managed-session-id' });
  });

  it('routes provider=claude (or unset) through spawnClaudeAgent and forwards mcpItemIds', async () => {
    const res = await call('agents.spawn', {
      provider: 'claude',
      cwd: '/proj',
      mcpItemIds: ['srv1', 'srv2'],
    });

    expect(spawnClaudeAgent).toHaveBeenCalledTimes(1);
    expect(spawnManagedAgent).not.toHaveBeenCalled();
    const arg = spawnClaudeAgent.mock.calls[0][0] as { mcpItemIds: string[] };
    // Regression: the hub path used to drop mcpItemIds so remote-spawned agents
    // came up with none of their selected Library MCP servers.
    expect(arg.mcpItemIds).toEqual(['srv1', 'srv2']);
    expect(res).toEqual({ sessionId: 'claude-session-id' });
  });

  it('defaults to the Claude path when no provider is given', async () => {
    await call('agents.spawn', { cwd: '/proj' });
    expect(spawnClaudeAgent).toHaveBeenCalledTimes(1);
    expect(spawnManagedAgent).not.toHaveBeenCalled();
  });

  it("routes claude + transport 'stream' through spawnManagedAgent (standing rule: both spawn transports share the managed dispatch)", async () => {
    const res = await call('agents.spawn', {
      provider: 'claude',
      transport: 'stream',
      cwd: '/proj',
      model: 'opus',
    });

    expect(spawnManagedAgent).toHaveBeenCalledTimes(1);
    expect(spawnClaudeAgent).not.toHaveBeenCalled();
    const arg = spawnManagedAgent.mock.calls[0][0] as {
      provider: string;
      transport: string;
      cwd: string;
      model: string;
    };
    expect(arg.provider).toBe('claude');
    expect(arg.transport).toBe('stream');
    expect(arg.cwd).toBe('/proj');
    expect(arg.model).toBe('opus');
    expect(res).toEqual({ sessionId: 'managed-session-id' });
  });

  it("forwards profileId and mcpItemIds on the claude 'stream' branch (parity with the IPC stream path — this branch used to drop both)", async () => {
    await call('agents.spawn', {
      provider: 'claude',
      transport: 'stream',
      cwd: '/proj',
      profileId: 'profile-1',
      mcpItemIds: ['mcp-a', 'mcp-b'],
    });

    expect(spawnManagedAgent).toHaveBeenCalledTimes(1);
    const arg = spawnManagedAgent.mock.calls[0][0] as {
      profileId?: string;
      mcpItemIds?: string[];
    };
    expect(arg.profileId).toBe('profile-1');
    expect(arg.mcpItemIds).toEqual(['mcp-a', 'mcp-b']);
  });

  it("claude + transport 'pty' (or unset, with no config default) stays on spawnClaudeAgent", async () => {
    await call('agents.spawn', { provider: 'claude', transport: 'pty', cwd: '/proj' });
    expect(spawnClaudeAgent).toHaveBeenCalledTimes(1);
    expect(spawnManagedAgent).not.toHaveBeenCalled();
  });

  it('falls back to the config default (claude.transport) when the caller omits transport', async () => {
    getConfig.mockReturnValueOnce({
      agents: { binaries: { codex: '/custom/codex' } },
      claude: { transport: 'stream' },
    } as never);
    await call('agents.spawn', { provider: 'claude', cwd: '/proj' });
    expect(spawnManagedAgent).toHaveBeenCalledTimes(1);
    expect(spawnClaudeAgent).not.toHaveBeenCalled();
  });

  it('sanitizes permission bypass on the claude-stream path too', async () => {
    await call('agents.spawn', {
      provider: 'claude',
      transport: 'stream',
      cwd: '/proj',
      skipPermissions: true,
      permissionMode: 'bypassPermissions',
    });
    const arg = spawnManagedAgent.mock.calls[0][0] as {
      skipPermissions: boolean;
      permissionMode: string | undefined;
    };
    expect(arg.skipPermissions).toBe(false);
    expect(arg.permissionMode).toBeUndefined();
  });
});

describe('agents.spawn — SECURITY: remote callers cannot auto-bypass approvals', () => {
  it('forces skipPermissions off even when the caller requests it (Claude path)', async () => {
    await call('agents.spawn', { cwd: '/proj', skipPermissions: true });
    const arg = spawnClaudeAgent.mock.calls[0][0] as { skipPermissions: boolean };
    expect(arg.skipPermissions).toBe(false);
  });

  it('drops a bypassPermissions permissionMode to undefined (never auto-bypass)', async () => {
    await call('agents.spawn', { cwd: '/proj', permissionMode: 'bypassPermissions' });
    const arg = spawnClaudeAgent.mock.calls[0][0] as {
      skipPermissions: boolean;
      permissionMode: string | undefined;
    };
    expect(arg.skipPermissions).toBe(false);
    expect(arg.permissionMode).toBeUndefined();
  });

  it('drops a yolo permissionMode to undefined', async () => {
    await call('agents.spawn', { cwd: '/proj', permissionMode: 'yolo' });
    const arg = spawnClaudeAgent.mock.calls[0][0] as { permissionMode: string | undefined };
    expect(arg.permissionMode).toBeUndefined();
  });

  it('forces skipPermissions off on the managed path too', async () => {
    await call('agents.spawn', {
      provider: 'codex',
      cwd: '/proj',
      skipPermissions: true,
      permissionMode: 'yolo',
    });
    const arg = spawnManagedAgent.mock.calls[0][0] as { skipPermissions: boolean };
    expect(arg.skipPermissions).toBe(false);
  });

  it('preserves a safe explicit permissionMode (plan) unchanged', async () => {
    await call('agents.spawn', { cwd: '/proj', permissionMode: 'plan' });
    const arg = spawnClaudeAgent.mock.calls[0][0] as { permissionMode: string | undefined };
    expect(arg.permissionMode).toBe('plan');
  });
});

describe('providers discovery', () => {
  it('providers.listModels resolves the binary and queries claudemon for the provider', async () => {
    const res = await call('providers.listModels', { provider: 'codex', cwd: '/proj' });
    expect(resolveAgentBinary).toHaveBeenCalledWith('codex', '/custom/codex');
    expect(clientMock.listProviderModels).toHaveBeenCalledWith('codex', '/proj', '/bin/codex');
    expect(res).toEqual(['m1', 'm2']);
  });

  it('providers.listModels rejects an unknown provider', async () => {
    await expect(
      async () => await call('providers.listModels', { provider: 'bogus' }),
    ).rejects.toThrow(/providers\.listModels requires/);
    expect(clientMock.listProviderModels).not.toHaveBeenCalled();
  });

  it('providers.checkAll passes the configured custom binaries through', async () => {
    const res = await call('providers.checkAll');
    expect(checkAllProviders).toHaveBeenCalledWith({ codex: '/custom/codex' });
    expect(res).toEqual({ codex: true });
  });
});

describe('claude control pass-throughs', () => {
  it('claude.setPermissionMode drives claudemon and syncs the store on success', async () => {
    const res = await call('claude.setPermissionMode', { sessionId: 's1', mode: 'plan' });
    expect(clientMock.setPermissionMode).toHaveBeenCalledWith('s1', 'plan');
    expect(notePermissionMode).toHaveBeenCalledWith('s1', 'plan');
    expect(res).toEqual({ ok: true, mode: 'plan' });
  });

  it('claude.setPermissionMode does NOT touch the store when claudemon reports failure', async () => {
    clientMock.setPermissionMode.mockResolvedValueOnce({ ok: false } as never);
    await call('claude.setPermissionMode', { sessionId: 's1', mode: 'plan' });
    expect(notePermissionMode).not.toHaveBeenCalled();
  });

  it('claude.setPermissionMode validates its params', async () => {
    await expect(
      async () => await call('claude.setPermissionMode', { sessionId: 's1' }),
    ).rejects.toThrow(/requires \{ sessionId, mode \}/);
  });

  it('claude.setModel forwards model + effort to claudemon', async () => {
    await call('claude.setModel', { sessionId: 's1', model: 'gpt', effort: 'high' });
    expect(clientMock.setModel).toHaveBeenCalledWith('s1', 'gpt', 'high');
  });

  it('claude.setModel rejects when neither model nor effort is given', async () => {
    await expect(async () => await call('claude.setModel', { sessionId: 's1' })).rejects.toThrow(
      /requires \{ sessionId, model and\/or effort \}/,
    );
  });

  it('claude.handoffBrief forwards to claudemon', async () => {
    const res = await call('claude.handoffBrief', { sessionId: 's1' });
    expect(clientMock.handoffBrief).toHaveBeenCalledWith('s1');
    expect(res).toEqual({ path: '/brief.md' });
  });

  it('claude.handoffBrief rejects a missing sessionId', async () => {
    await expect(async () => await call('claude.handoffBrief', {})).rejects.toThrow(
      /requires \{ sessionId \}/,
    );
  });
});

describe('agents.sendMessage', () => {
  it('forwards to claudemon.message and returns ok', async () => {
    const res = await call('agents.sendMessage', { sessionId: 's1', text: 'hi' });
    expect(clientMock.message).toHaveBeenCalledWith('s1', 'hi');
    expect(res).toEqual({ ok: true });
  });

  it('surfaces a not-accepting-input rejection when claudemon returns ok:false', async () => {
    clientMock.message.mockResolvedValueOnce({ ok: false, mode: 'Approval' } as never);
    await expect(
      async () => await call('agents.sendMessage', { sessionId: 's1', text: 'hi' }),
    ).rejects.toThrow(/not accepting input.*Approval/);
  });

  it('validates params before hitting claudemon', async () => {
    await expect(async () => await call('agents.sendMessage', { sessionId: 's1' })).rejects.toThrow(
      /requires \{ sessionId, text \}/,
    );
    expect(clientMock.message).not.toHaveBeenCalled();
  });
});

describe('error propagation', () => {
  it('a handler throwing (validation) surfaces a structured Error, not a crash', async () => {
    // The bus caller invokes the handler; an invalid call must reject with a
    // real Error whose message the bus can serialize — never throw synchronously
    // in a way that kills the provider.
    await expect(async () => await call('claude.setModel', {})).rejects.toBeInstanceOf(Error);
  });

  it('propagates a rejection from the underlying spawn (does not swallow it)', async () => {
    spawnClaudeAgent.mockRejectedValueOnce(new Error('spawn boom'));
    await expect(async () => await call('agents.spawn', { cwd: '/proj' })).rejects.toThrow(
      'spawn boom',
    );
  });

  it('propagates a rejection from claudemon.setModel', async () => {
    clientMock.setModel.mockRejectedValueOnce(new Error('daemon down'));
    await expect(
      async () => await call('claude.setModel', { sessionId: 's1', model: 'x' }),
    ).rejects.toThrow('daemon down');
  });
});

describe('fs.* path confinement (SECURITY.md #8)', () => {
  // A real temp dir stands in for a live agent's cwd — the confinement helpers
  // canonicalize via the real filesystem, so the roots must exist.
  let agentCwd: string;
  beforeEach(() => {
    agentCwd = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'wks-agent-')));
    getAllSnapshots.mockReturnValue([{ cwd: agentCwd }] as never);
  });

  it('fs.read allows a path inside a live agent cwd', () => {
    const inside = path.join(agentCwd, 'notes.txt');
    expect(() => call('fs.read', { path: inside })).not.toThrow();
    expect(readTextFile).toHaveBeenCalledWith(inside);
  });

  it('fs.read denies an arbitrary host path (e.g. /etc/passwd)', () => {
    expect(() => call('fs.read', { path: '/etc/passwd' })).toThrow(/outside the allowed workspace/);
    expect(readTextFile).not.toHaveBeenCalled();
  });

  it('fs.read denies a traversal escape from the agent cwd', () => {
    const escape = path.join(agentCwd, '..', '..', '..', 'etc', 'passwd');
    expect(() => call('fs.read', { path: escape })).toThrow(/outside the allowed workspace/);
    expect(readTextFile).not.toHaveBeenCalled();
  });

  it('fs.write denies writing outside the workspace', () => {
    expect(() =>
      call('fs.write', { path: path.join(os.homedir(), '.ssh', 'authorized_keys'), contents: 'x' }),
    ).toThrow(/outside the allowed workspace/);
    expect(writeTextFile).not.toHaveBeenCalled();
  });

  it('fs.write allows a not-yet-existing file inside the agent cwd (nearest-ancestor canonicalize)', () => {
    const newFile = path.join(agentCwd, 'sub', 'new.txt'); // parent dir does not exist yet
    expect(() => call('fs.write', { path: newFile, contents: 'x' })).not.toThrow();
    expect(writeTextFile).toHaveBeenCalledWith(newFile, 'x');
  });

  it('search.project denies a cwd outside the workspace', () => {
    expect(() => call('search.project', { query: 'x', cwd: '/etc' })).toThrow(
      /outside the allowed workspace/,
    );
    expect(searchProject).not.toHaveBeenCalled();
  });

  it('search.project allows a cwd inside a live agent cwd', () => {
    expect(() => call('search.project', { query: 'x', cwd: agentCwd })).not.toThrow();
    expect(searchProject).toHaveBeenCalled();
  });

  it('fs.listDir (folder picker) denies browsing outside the home tree', () => {
    expect(() => call('fs.listDir', { path: '/etc' })).toThrow(/outside the allowed workspace/);
  });

  it('fs.listDir allows browsing inside a live agent cwd', () => {
    const res = call('fs.listDir', { path: agentCwd }) as { path: string };
    expect(res.path).toBe(agentCwd);
  });
});

describe('fs.* credential deny-list (twin of the brain fsguard)', () => {
  // The config dir used to be a workspace root wholesale, which handed any bus
  // caller remote-token / tokens.json / every plugin's .bus-token — i.e. the
  // credential that makes a connection `trusted`, which drops per-plugin scoping
  // and unlocks /plugins/install. Only library/, layouts/ and sessions/ are roots
  // now, and the deny still applies however the path got admitted.
  //
  // Every case below runs with an agent cwd that CONTAINS the config dir (the
  // user who spawned an agent in ~/.config), so the roots check passes and only
  // the deny-list can refuse — testing the gate that actually has to hold.
  let agentCwd: string;
  beforeEach(() => {
    fs.mkdirSync(path.join(cfg.dir, 'library'), { recursive: true });
    fs.mkdirSync(path.join(cfg.dir, 'plugins', 'acme.ci'), { recursive: true });
    fs.writeFileSync(path.join(cfg.dir, 'remote-token'), 'super-secret', 'utf-8');
    fs.writeFileSync(path.join(cfg.dir, 'plugins', 'acme.ci', '.bus-token'), 'tok', 'utf-8');
    agentCwd = path.dirname(cfg.dir);
    getAllSnapshots.mockReturnValue([{ cwd: agentCwd }] as never);
  });

  it('fs.read denies the remote-share token', () => {
    expect(() => call('fs.read', { path: path.join(cfg.dir, 'remote-token') })).toThrow(
      /outside the allowed workspace/,
    );
    expect(readTextFile).not.toHaveBeenCalled();
  });

  it("fs.read denies a plugin's .bus-token", () => {
    expect(() =>
      call('fs.read', { path: path.join(cfg.dir, 'plugins', 'acme.ci', '.bus-token') }),
    ).toThrow(/outside the allowed workspace/);
    expect(readTextFile).not.toHaveBeenCalled();
  });

  it('fs.write denies overwriting a credential (a token DoS is a bus outage)', () => {
    expect(() =>
      call('fs.write', { path: path.join(cfg.dir, 'remote-token'), contents: 'x' }),
    ).toThrow(/outside the allowed workspace/);
    expect(writeTextFile).not.toHaveBeenCalled();
  });

  it('denies .bus-token / .settings.json by basename anywhere, not just in the config dir', () => {
    // `workspacer plugin dev <dir>` mints these inside an ordinary project, which
    // is an agent cwd — a root no narrowing can help with.
    const devPlugin = fs.mkdtempSync(path.join(agentCwd, 'wks-devplugin-'));
    for (const name of ['.bus-token', '.settings.json']) {
      expect(() => call('fs.read', { path: path.join(devPlugin, name) })).toThrow(
        /outside the allowed workspace/,
      );
    }
    expect(readTextFile).not.toHaveBeenCalled();
    fs.rmSync(devPlugin, { recursive: true, force: true });
  });

  it('still allows the library/ subtree the UI edits', () => {
    const item = path.join(cfg.dir, 'library', 'prompt.md');
    expect(() => call('fs.read', { path: item })).not.toThrow();
    expect(readTextFile).toHaveBeenCalledWith(item);
  });

  it('denies the config secret on the roots check too when no agent cwd covers it', () => {
    getAllSnapshots.mockReturnValue([] as never);
    expect(() => call('fs.read', { path: path.join(cfg.dir, 'remote-token') })).toThrow(
      /outside the allowed workspace/,
    );
    expect(readTextFile).not.toHaveBeenCalled();
  });
});

describe('library.* cwd confinement', () => {
  // `cwd` selects the project whose .workspacer/library + .claude assets are
  // listed, written and (recursively) deleted — untrusted on the bus.
  let agentCwd: string;
  beforeEach(() => {
    agentCwd = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'wks-lib-')));
    getAllSnapshots.mockReturnValue([{ cwd: agentCwd }] as never);
  });

  it('library.save is denied for a cwd outside the workspace', () => {
    expect(() =>
      call('library.save', {
        scope: 'project',
        cwd: os.homedir(),
        title: 't',
        kind: 'prompt',
        body: 'b',
      }),
    ).toThrow(/outside the allowed workspace/);
    expect(libraryMock.save).not.toHaveBeenCalled();
  });

  it('library.save runs for a live agent cwd', () => {
    call('library.save', {
      scope: 'project',
      cwd: agentCwd,
      title: 't',
      kind: 'prompt',
      body: 'b',
    });
    expect(libraryMock.save).toHaveBeenCalledTimes(1);
  });

  it('library.remove is denied for a cwd outside the workspace', () => {
    expect(() =>
      call('library.remove', { scope: 'claude', id: 'x', cwd: '/etc', kind: 'skill' }),
    ).toThrow(/outside the allowed workspace/);
    expect(libraryMock.remove).not.toHaveBeenCalled();
  });

  it('library.list is denied for a cwd outside the browsable tree', () => {
    expect(() => call('library.list', { cwd: '/etc' })).toThrow(/outside the allowed workspace/);
    expect(libraryMock.list).not.toHaveBeenCalled();
  });

  it('library.list allows a directory under home that is not an agent cwd yet', () => {
    // The New Agent dialog lists the library of the directory the user is about
    // to spawn in, to populate the project-MCP picker — no agent runs there yet,
    // and the dialog swallows errors, so a workspace-roots rule here would show
    // an empty picker rather than an error. Same browse rule as fs.listDir.
    const notYetSpawned = path.join(os.homedir(), 'some-project');
    call('library.list', { cwd: notYetSpawned });
    expect(libraryMock.list).toHaveBeenCalledWith(notYetSpawned);
  });
});

describe('notifications.post — external URL scheme check', () => {
  function clickWith(url: string): void {
    call('notifications.post', { title: 't', url });
    const onClick = notificationHandlers.get('click');
    expect(onClick).toBeDefined();
    onClick!();
  }

  it('opens an https URL', () => {
    clickWith('https://example.com/build/42');
    expect(openExternal).toHaveBeenCalledWith('https://example.com/build/42');
  });

  it('refuses a file:// URL (shell.openExternal would launch it)', () => {
    clickWith('file:///etc/passwd');
    expect(openExternal).not.toHaveBeenCalled();
  });

  it('refuses a custom-protocol URL', () => {
    clickWith('vscode://file/etc/shadow');
    expect(openExternal).not.toHaveBeenCalled();
  });
});

describe('git.* cwd confinement (SECURITY.md #6)', () => {
  // The review-pane git surface moved from claudemon to the host; its bus caps are
  // now the remote-reachable entry point, so a caller-supplied cwd must be confined
  // to the live agent cwds (the same workspace roots as fs.*), not any host repo.
  let agentCwd: string;
  beforeEach(() => {
    agentCwd = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'wks-git-')));
    getAllSnapshots.mockReturnValue([{ cwd: agentCwd }] as never);
    workRootFor.mockImplementation(async (cwd: string) => cwd);
  });

  it('git.commit runs when cwd is a live agent cwd', async () => {
    await call('git.commit', { cwd: agentCwd, message: 'wip' });
    expect(gitMock.commit).toHaveBeenCalledWith(agentCwd, 'wip');
  });

  it('git.commit is denied for a cwd outside the workspace', async () => {
    expect(() => call('git.commit', { cwd: '/tmp/some-other-repo', message: 'wip' })).toThrow(
      /outside the allowed workspace/,
    );
    expect(gitMock.commit).not.toHaveBeenCalled();
  });

  it('git.push is denied for a cwd outside the workspace', () => {
    expect(() => call('git.push', { cwd: os.homedir() })).toThrow(/outside the allowed workspace/);
    expect(gitMock.push).not.toHaveBeenCalled();
  });

  it('git.status (read) is also confined to the workspace', () => {
    expect(() => call('git.status', { cwd: '/etc' })).toThrow(/outside the allowed workspace/);
    expect(gitMock.status).not.toHaveBeenCalled();
  });

  it('git.status runs for a live agent cwd', async () => {
    await call('git.status', { cwd: agentCwd });
    expect(gitMock.status).toHaveBeenCalledWith(agentCwd);
  });

  // git.diff's `path` is not just a pathspec: with untracked:true gitService
  // hands it to `git diff --no-index -- /dev/null <path>`, where git treats it as
  // a filesystem operand. A legal cwd plus an escaping path therefore read any
  // file on the host as an all-added diff until the path was confined too.
  it('git.diff denies an absolute path outside the repo (untracked --no-index operand)', async () => {
    await expect(
      call('git.diff', { cwd: agentCwd, path: '/etc/shadow', untracked: true }),
    ).rejects.toThrow(/outside the allowed workspace/);
    expect(gitMock.diff).not.toHaveBeenCalled();
  });

  it('git.diff denies a traversal path that escapes the repo', async () => {
    await expect(
      call('git.diff', { cwd: agentCwd, path: '../../../etc/passwd', untracked: true }),
    ).rejects.toThrow(/outside the allowed workspace/);
    expect(gitMock.diff).not.toHaveBeenCalled();
  });

  it('git.diff still allows a repo-relative path inside the agent cwd', async () => {
    await call('git.diff', { cwd: agentCwd, path: 'src/new.ts', untracked: true });
    expect(gitMock.diff).toHaveBeenCalledWith(agentCwd, 'src/new.ts', undefined, true);
  });

  // The guard has to measure `path` the way git will: gitService anchors every
  // command at `rev-parse --show-toplevel`, so with the agent cwd nested in a
  // monorepo (the normal case) a path resolved against the agent cwd names a
  // different file than the one git opens.
  describe('with the agent cwd nested below the repo root', () => {
    let repoRoot: string;
    beforeEach(() => {
      repoRoot = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'wks-repo-')));
      agentCwd = path.join(repoRoot, 'apps', 'desktop');
      fs.mkdirSync(agentCwd, { recursive: true });
      getAllSnapshots.mockReturnValue([{ cwd: agentCwd }] as never);
      workRootFor.mockImplementation(async () => repoRoot);
    });
    afterEach(() => fs.rmSync(repoRoot, { recursive: true, force: true }));

    it('refuses a ../-path that measuring from the agent cwd would wave through', async () => {
      // Measured from the agent cwd this names a file inside a SECOND live
      // agent's cwd — inside a workspace root, so a cwd-based check admits it.
      // git runs two levels shallower, from repoRoot, where the same string
      // normalizes somewhere else entirely and outside every repo: the check and
      // the read were looking at different files.
      const otherAgent = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'wks-other-')));
      getAllSnapshots.mockReturnValue([{ cwd: agentCwd }, { cwd: otherAgent }] as never);
      const rel = path.relative(agentCwd, path.join(otherAgent, 'secret.env'));

      await expect(call('git.diff', { cwd: agentCwd, path: rel, untracked: true })).rejects.toThrow(
        /outside the allowed workspace/,
      );
      expect(gitMock.diff).not.toHaveBeenCalled();
      fs.rmSync(otherAgent, { recursive: true, force: true });
    });

    it('refuses a path that climbs out of the repo root', async () => {
      await expect(
        call('git.diff', { cwd: agentCwd, path: '../../../etc/passwd', untracked: true }),
      ).rejects.toThrow(/outside the allowed workspace/);
      expect(gitMock.diff).not.toHaveBeenCalled();
    });

    it('still allows a root-relative path in a sibling subtree (what git.status hands back)', async () => {
      // git.status prints repo-root-relative paths for the WHOLE repo, and the
      // review pane feeds them straight back; refusing them because they sit
      // outside the agent cwd would break review in every monorepo.
      await call('git.diff', { cwd: agentCwd, path: 'services/hub/main.go' });
      expect(gitMock.diff).toHaveBeenCalledWith(
        agentCwd,
        'services/hub/main.go',
        undefined,
        undefined,
      );
    });
  });
});
