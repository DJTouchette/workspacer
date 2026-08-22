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
 * DELEGATION MODE: this file runs with DELEGATE_CATALOG_TO_BRAIN = true, the
 * production default. Everything asserted here registers through
 * `registerCapability`, so main owns it in the mode it actually ships in. The
 * `cat`-door capabilities (fs.read/write/listEntries/listDir, library.*) are
 * NOT registered here — the Go brain answers those — and their main-side
 * handlers are exercised in the sibling hubCapabilitiesKillSwitch.test.ts,
 * which is the only file allowed to mock delegation off. Delegation-off is the
 * marked special case; it is not the baseline. This file used to mock it off
 * "for completeness", and that is precisely why a security bug in the shipping
 * path stayed invisible: the test never touched the code that runs.
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
const emitToRenderer = vi.fn();
vi.mock('./hubClient', () => ({
  registerCapability: (method: string, handler: (params: unknown) => unknown) => {
    registered.set(method, handler);
  },
  emitToRenderer: (...a: unknown[]) => emitToRenderer(...a),
}));

// PRODUCTION MODE: the brain owns the catalog, so `cat(...)` registers nothing
// and only main's own registerCapability handlers exist on the bus. Do not flip
// this to false to make a test pass — a capability that is missing here is a
// capability the brain serves, and it belongs in hubCapabilitiesKillSwitch.test.ts.
vi.mock('./brainDelegation', () => ({ DELEGATE_CATALOG_TO_BRAIN: true }));

const spawnManagedAgent = vi.fn(async () => 'managed-session-id');
vi.mock('./managedSpawn', () => ({
  spawnManagedAgent: (...a: unknown[]) => spawnManagedAgent(...a),
}));

const spawnClaudeAgent = vi.fn(async () => 'claude-session-id');
vi.mock('./claudeSpawn', () => ({ spawnClaudeAgent: (...a: unknown[]) => spawnClaudeAgent(...a) }));

const createWorktree = vi.fn(async () => ({ ok: true, path: '/wt/proj-abc', branch: 'agent/x' }));
vi.mock('./worktreeService', () => ({
  createWorktree: (...a: unknown[]) => createWorktree(...a),
  worktreeInfo: vi.fn(async () => ({ isRepo: true, root: '/proj' })),
}));

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
const getSnapshot = vi.fn(() => null as unknown);
const noteRequestedModel = vi.fn();
vi.mock('./claudeSessionStore', () => ({
  claudeSessionStore: {
    notePermissionMode: (...a: unknown[]) => notePermissionMode(...a),
    getAllSnapshots: (...a: unknown[]) => getAllSnapshots(...a),
    getSnapshot: (...a: unknown[]) => getSnapshot(...a),
    noteRequestedModel: (...a: unknown[]) => noteRequestedModel(...a),
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
  // log / commitDiff / commitNumstat were not even mocked, which is the tell:
  // no test in this file had ever invoked those three handlers at all.
  log: vi.fn(async () => []),
  commitDiff: vi.fn(async () => ''),
  commitNumstat: vi.fn(async () => []),
}));
vi.mock('./terminalShare', () => ({}));
vi.mock('./supervisorSkill', () => ({ ensureSupervisorHome: vi.fn(() => '/home/super') }));

const { registerHubCapabilities } = await import('./hubCapabilities');
const { searchProject } = await import('./searchService');
const gitMock = await import('./gitService');

/** Invoke a registered capability by method name. */
function call(method: string, params?: unknown): unknown {
  const handler = registered.get(method);
  if (!handler)
    throw new Error(
      `capability not registered under DELEGATE_CATALOG_TO_BRAIN=true: ${method} — ` +
        `if this is a \`cat\`-door capability the brain answers it, and its main-side ` +
        `handler belongs in hubCapabilitiesKillSwitch.test.ts`,
    );
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

// The plural call is a BACKGROUND feed: every consumer (promoteSessionSnapshots,
// useSessionSnapshots) compacts it on arrival and OverviewPane never reads
// `conversation` at all. Serializing the full transcript here paid to have it
// thrown away — over the bus that is every session's whole transcript as JSON
// on a WebSocket, on connect and on every OverviewPane refresh.
describe('sessions.snapshots — compacted before it leaves the process', () => {
  const bigSession = () => ({
    sessionId: 's1',
    cwd: '/proj',
    conversation: Array.from({ length: 50 }, (_, i) => ({
      role: i % 2 === 0 ? 'user' : 'assistant',
      content: 'x'.repeat(9000),
    })),
    completedToolCalls: Array.from({ length: 40 }, (_, i) => ({
      id: `t${i}`,
      status: 'complete',
      completedAt: i,
      input: { blob: 'y'.repeat(9000) },
    })),
  });

  it('trims the conversation tail and banks the dropped turns', async () => {
    getAllSnapshots.mockReturnValue([bigSession()] as never);
    const out = (await call('sessions.snapshots')) as Array<Record<string, any>>;

    expect(out).toHaveLength(1);
    expect(out[0].conversation).toHaveLength(12);
    // Absolute turn indices must survive the trim: 50 - 12 = 38 dropped, and
    // half of those were user sends. ClaudePane anchors on both.
    expect(out[0].conversationOffset).toBe(38);
    expect(out[0].conversationUserOffset).toBe(19);
    expect(out[0].completedToolCalls).toHaveLength(20);
  });

  it('truncates the payloads it does keep', async () => {
    getAllSnapshots.mockReturnValue([bigSession()] as never);
    const out = (await call('sessions.snapshots')) as Array<Record<string, any>>;
    // 9000 chars in, MAX_TEXT_CHARS out — the point is that the wire never
    // carries the untruncated body.
    expect(out[0].conversation[0].content.length).toBeLessThan(9000);
    expect(JSON.stringify(out[0]).length).toBeLessThan(200_000);
  });

  // The active pane reads the SINGULAR call and needs every turn; compacting it
  // would silently cut scrollback for the session the user is looking at.
  it('leaves sessions.snapshot (singular) full', async () => {
    getSnapshot.mockReturnValue(bigSession() as never);
    const out = (await call('sessions.snapshot', { sessionId: 's1' })) as Record<string, any>;
    expect(out.conversation).toHaveLength(50);
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

  // mcpItemIds is CLAMPED on this path, the same way skipPermissions is, and for
  // a sharper reason. Each id resolves — through libraryService -> toClaudeEntry
  // -> buildSessionMcpConfig — into a --mcp-config entry whose `command`, `args`
  // and `env` come verbatim out of a library item, and the spawn passes
  // `--allowedTools mcp__<id>` alongside it, so the server is PRE-APPROVED and no
  // permission prompt gates it: `mcpItemIds: ['x']` is argv[0] of a host process
  // chosen by whoever wrote item x. The write side cannot be closed — a bus
  // caller reaches the item through library.save OR a plain fs.write into
  // <configDir>/library, which is a configStoreRoot by design — so the identity
  // of the SPAWNER is the only thing left to gate on. The local IPC path
  // (ipc.ts) still honours the selection.
  it('routes provider=claude (or unset) through spawnClaudeAgent and CLAMPS mcpItemIds', async () => {
    const res = await call('agents.spawn', {
      provider: 'claude',
      cwd: '/proj',
      mcpItemIds: ['srv1', 'srv2'],
    });

    expect(spawnClaudeAgent).toHaveBeenCalledTimes(1);
    expect(spawnManagedAgent).not.toHaveBeenCalled();
    const arg = spawnClaudeAgent.mock.calls[0][0] as { mcpItemIds?: string[]; cwd?: string };
    expect(
      arg.mcpItemIds,
      'a bus spawn carried mcpItemIds — an MCP server definition is argv[0] of a host process, pre-approved via --allowedTools',
    ).toBeUndefined();
    // The rest of the call still rides through, so the clamp cannot be
    // "everything was dropped".
    expect(arg.cwd).toBe('/proj');
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

  it("forwards profileId but CLAMPS mcpItemIds on the claude 'stream' branch", async () => {
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
      scrubProfileBypass?: boolean;
    };
    // profileId still rides through — and is scrubbed downstream, which is where
    // the profile's OWN mcpItemIds are dropped (scrubBypassProfile).
    expect(arg.profileId).toBe('profile-1');
    expect(arg.scrubProfileBypass).toBe(true);
    expect(arg.mcpItemIds, 'the stream branch is the shipping default and must clamp too').toBe(
      undefined,
    );
  });

  it('forwards the hub-stamped profileGranted to both claude branches, hardened to a strict boolean', async () => {
    // The hub's sanitizeSpawnParams already deleted any caller-supplied copy —
    // by the time it reaches this provider it is trustworthy. The `=== true`
    // hardening is for a hub-bypassing local caller handing a truthy string.
    await call('agents.spawn', {
      provider: 'claude',
      transport: 'stream',
      cwd: '/proj',
      profileId: 'work',
      profileGranted: true,
    });
    expect(
      (spawnManagedAgent.mock.calls[0][0] as { profileGranted?: boolean }).profileGranted,
    ).toBe(true);

    await call('agents.spawn', {
      provider: 'claude',
      transport: 'pty',
      cwd: '/proj',
      profileId: 'work',
      profileGranted: 'yes',
    });
    expect((spawnClaudeAgent.mock.calls[0][0] as { profileGranted?: boolean }).profileGranted).toBe(
      false,
    );

    await call('agents.spawn', { provider: 'claude', transport: 'pty', cwd: '/proj' });
    expect((spawnClaudeAgent.mock.calls[1][0] as { profileGranted?: boolean }).profileGranted).toBe(
      false,
    );
  });

  it("claude + transport 'pty' (or unset, with no config default) stays on spawnClaudeAgent", async () => {
    await call('agents.spawn', { provider: 'claude', transport: 'pty', cwd: '/proj' });
    expect(spawnClaudeAgent).toHaveBeenCalledTimes(1);
    expect(spawnManagedAgent).not.toHaveBeenCalled();
  });

  it('worktree:true carves a worktree in main and spawns the worker THERE (ship-task isolation)', async () => {
    createWorktree.mockResolvedValueOnce({ ok: true, path: '/wt/proj-abc', branch: 'agent/x' });
    await call('agents.spawn', {
      provider: 'claude',
      transport: 'pty',
      cwd: '/proj',
      worktree: true,
    });
    expect(createWorktree).toHaveBeenCalledTimes(1);
    expect((createWorktree.mock.calls[0][0] as { repoCwd: string }).repoCwd).toBe('/proj');
    // The worker's cwd is the worktree, not the checkout.
    expect((spawnClaudeAgent.mock.calls[0][0] as { cwd: string }).cwd).toBe('/wt/proj-abc');
  });

  it('a worktree failure falls back to cwd rather than refusing the dispatch', async () => {
    createWorktree.mockResolvedValueOnce({ ok: false, error: 'not a git repo' });
    await call('agents.spawn', {
      provider: 'claude',
      transport: 'pty',
      cwd: '/proj',
      worktree: true,
    });
    expect(spawnClaudeAgent).toHaveBeenCalledTimes(1);
    expect((spawnClaudeAgent.mock.calls[0][0] as { cwd: string }).cwd).toBe('/proj');
  });

  it('no worktree is created when worktree is unset (scout / in-place work)', async () => {
    await call('agents.spawn', { provider: 'claude', transport: 'pty', cwd: '/proj' });
    expect(createWorktree).not.toHaveBeenCalled();
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

  it('HONORS bypass when the hub stamped yoloGranted (fleet-manager full access)', async () => {
    // yoloGranted is provenance, not a request: the hub's sanitizeSpawnParams
    // only sets it after verifying the caller's YoloAllowed grant. With it, the
    // requested skipPermissions / bypass mode rides through instead of clamping.
    await call('agents.spawn', {
      provider: 'claude',
      transport: 'stream',
      cwd: '/proj',
      skipPermissions: true,
      permissionMode: 'bypassPermissions',
      yoloGranted: true,
    });
    const arg = spawnManagedAgent.mock.calls[0][0] as {
      skipPermissions: boolean;
      permissionMode: string | undefined;
    };
    expect(arg.skipPermissions).toBe(true);
    expect(arg.permissionMode).toBe('bypassPermissions');
  });

  it('a truthy-but-not-true yoloGranted does NOT unlock bypass (hub stamps a real boolean)', async () => {
    await call('agents.spawn', {
      provider: 'claude',
      transport: 'pty',
      cwd: '/proj',
      skipPermissions: true,
      yoloGranted: 'yes',
    });
    expect(
      (spawnClaudeAgent.mock.calls[0][0] as { skipPermissions: boolean }).skipPermissions,
    ).toBe(false);
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

// An OMITTED skipPermissions resolves to the config default the spawn dialog
// pre-selects (claude.skipPermissionsDefault / a bypass defaultPermissionMode),
// and the resolved value passes the SAME grant gate as an explicit request —
// honored only under the hub-stamped yoloGranted, clamped for everyone else.
// TWIN: cmd/brain spawn_skipdefault_test.go / cmd/mcp spawndefaults_test.go.
describe('agents.spawn — omitted skipPermissions resolves the config default', () => {
  const withClaudeCfg = (claude: Record<string, unknown>) =>
    getConfig.mockReturnValueOnce({
      agents: { binaries: { codex: '/custom/codex' } },
      claude,
    } as never);

  it('resolves skipPermissionsDefault:true for a granted (yoloGranted) spawn', async () => {
    withClaudeCfg({ skipPermissionsDefault: true, transport: 'pty' });
    await call('agents.spawn', { cwd: '/proj', yoloGranted: true });
    const arg = spawnClaudeAgent.mock.calls[0][0] as { skipPermissions: boolean };
    expect(arg.skipPermissions).toBe(true);
  });

  it('resolves a bypass defaultPermissionMode the same way', async () => {
    withClaudeCfg({
      skipPermissionsDefault: false,
      defaultPermissionMode: 'bypassPermissions',
      transport: 'pty',
    });
    await call('agents.spawn', { cwd: '/proj', yoloGranted: true });
    const arg = spawnClaudeAgent.mock.calls[0][0] as { skipPermissions: boolean };
    expect(arg.skipPermissions).toBe(true);
  });

  it('CLAMPS the config default for an ungranted caller — defaults never escalate a token', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    try {
      withClaudeCfg({ skipPermissionsDefault: true, transport: 'pty' });
      await call('agents.spawn', { cwd: '/proj' });
      const arg = spawnClaudeAgent.mock.calls[0][0] as { skipPermissions: boolean };
      expect(arg.skipPermissions).toBe(false);
      expect(warn.mock.calls.flat().join('\n')).toContain('config default');
    } finally {
      warn.mockRestore();
    }
  });

  it('an explicit false always beats the config default', async () => {
    withClaudeCfg({ skipPermissionsDefault: true, transport: 'pty' });
    await call('agents.spawn', { cwd: '/proj', skipPermissions: false, yoloGranted: true });
    const arg = spawnClaudeAgent.mock.calls[0][0] as { skipPermissions: boolean };
    expect(arg.skipPermissions).toBe(false);
  });

  it('default off + omitted field stays approvals-on even when granted', async () => {
    withClaudeCfg({ skipPermissionsDefault: false, transport: 'pty' });
    await call('agents.spawn', { cwd: '/proj', yoloGranted: true });
    const arg = spawnClaudeAgent.mock.calls[0][0] as { skipPermissions: boolean };
    expect(arg.skipPermissions).toBe(false);
  });

  it('the default rides the managed (claude-stream) leg too', async () => {
    withClaudeCfg({ skipPermissionsDefault: true, transport: 'stream' });
    await call('agents.spawn', { cwd: '/proj', yoloGranted: true });
    const arg = spawnManagedAgent.mock.calls[0][0] as { skipPermissions: boolean };
    expect(arg.skipPermissions).toBe(true);
  });
});

describe('providers discovery', () => {
  // A REAL directory registered as a live agent cwd: providers.listModels' `cwd`
  // is confined to browseRoots now (claudemon runs the provider CLI in it, and
  // opencode executes <cwd>/.opencode/plugin/*.js from there), and the guard
  // canonicalizes through the filesystem, so '/proj' no longer resolves to
  // anything a root contains.
  let providerCwd: string;
  beforeEach(() => {
    providerCwd = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'wks-prov-')));
    getAllSnapshots.mockReturnValue([{ cwd: providerCwd }] as never);
  });
  afterEach(() => {
    getAllSnapshots.mockReturnValue([] as never);
    fs.rmSync(providerCwd, { recursive: true, force: true });
  });

  it('providers.listModels resolves the binary and queries claudemon for the provider', async () => {
    const res = await call('providers.listModels', { provider: 'codex', cwd: providerCwd });
    expect(resolveAgentBinary).toHaveBeenCalledWith('codex', '/custom/codex');
    expect(clientMock.listProviderModels).toHaveBeenCalledWith('codex', providerCwd, '/bin/codex');
    expect(res).toEqual(['m1', 'm2']);
  });

  // The cwd is not read, it is EXECUTED IN. capspec's old excuse said it merely
  // "picks which project's provider config to read"; opencode loads and RUNS
  // every <cwd>/.opencode/plugin/*.js at startup, so an unconfined cwd made a
  // capability the consent list labels "List available models" the shortest path
  // to host code execution on the whole surface.
  it('providers.listModels refuses a cwd outside the browse roots', async () => {
    clientMock.listProviderModels.mockClear();
    await expect(
      async () => await call('providers.listModels', { provider: 'opencode', cwd: '/etc' }),
    ).rejects.toThrow(/outside the allowed workspace/);
    expect(clientMock.listProviderModels).not.toHaveBeenCalled();
  });

  // An absent cwd is indistinguishable from '' on the Go side, and '' is the
  // value that absolutizes to the process working directory. Refusing it here is
  // what keeps the two providers answering the same question.
  it('providers.listModels refuses an absent cwd rather than letting the daemon pick', async () => {
    clientMock.listProviderModels.mockClear();
    await expect(
      async () => await call('providers.listModels', { provider: 'codex' }),
    ).rejects.toThrow(/outside the allowed workspace/);
    expect(clientMock.listProviderModels).not.toHaveBeenCalled();
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

  // agents.spawn refuses to let a bus caller start an auto-approving agent —
  // "a YOLO agent must be started locally". claude.setPermissionMode reaches an
  // agent that is ALREADY RUNNING, does not ownership-check the sessionId, and
  // had no clamp at all: `mode` was validated as a non-empty string and
  // forwarded verbatim to POST /sessions/:id/permission-mode, which claudemon
  // applies for real (Shift+Tab to the bypass footer on PTY claude, the
  // adapter's auto-approve flag on codex/opencode/pi). One extra call therefore
  // undid the spawn clamp on an agent the LOCAL user had started in ask mode,
  // and agents.sendMessage drove it from there.
  for (const mode of ['bypassPermissions', 'yolo', 'dontAsk', 'auto']) {
    it(`claude.setPermissionMode refuses '${mode}' from a bus caller`, async () => {
      await expect(
        async () => await call('claude.setPermissionMode', { sessionId: 's1', mode }),
      ).rejects.toThrow(/cannot switch a running session into/);
      // A refusal that still reached the daemon would be no refusal at all.
      expect(clientMock.setPermissionMode).not.toHaveBeenCalled();
      expect(notePermissionMode).not.toHaveBeenCalled();
    });
  }

  // The floor: an allowlist that refused everything would satisfy the four cases
  // above while breaking the remote pill entirely. Tightening is not escalation.
  for (const mode of ['default', 'ask', 'acceptEdits', 'plan', 'manual']) {
    it(`claude.setPermissionMode still allows '${mode}'`, async () => {
      await call('claude.setPermissionMode', { sessionId: 's1', mode });
      expect(clientMock.setPermissionMode).toHaveBeenCalledWith('s1', mode);
    });
  }

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

describe('search.project cwd confinement', () => {
  // search.project is registerCapability, not `cat`: main answers it in
  // production, so its confinement is asserted here rather than in the
  // kill-switch file the fs.*/library.* cases moved to.
  //
  // A real temp dir stands in for a live agent's cwd — the confinement helpers
  // canonicalize via the real filesystem, so the roots must exist.
  let agentCwd: string;
  beforeEach(() => {
    agentCwd = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'wks-agent-')));
    getAllSnapshots.mockReturnValue([{ cwd: agentCwd }] as never);
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

describe('git.* cwd confinement', () => {
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

  // EVERY git.* capability, not the four somebody happened to write a test for.
  //
  // All ten take a caller-supplied absolute `cwd` and guardGitCwd is the only
  // thing confining them; capspec excuses all of them from PathParam on exactly
  // that ground. Yet only git.status / git.commit / git.push / git.diff were
  // ever named by a test, so the guardGitCwd() call could be deleted from
  // git.log, git.numstat, git.commitDiff, git.commitNumstat, git.stage and
  // git.unstage with the whole Go and desktop suites staying green — handing
  // every bus client (web / remote / MCP / any trusted connection) the diffs,
  // commit messages and file contents of any repo on the host, and write access
  // to any index. Table-driven so a new git.* handler has to be added here to
  // pass the completeness assertion at the end.
  const gitMethods: {
    method: string;
    params: Record<string, unknown>;
    fn: () => { mock: { calls: unknown[][] } };
  }[] = [
    { method: 'git.status', params: {}, fn: () => gitMock.status as never },
    { method: 'git.log', params: { limit: 5 }, fn: () => gitMock.log as never },
    { method: 'git.diff', params: {}, fn: () => gitMock.diff as never },
    { method: 'git.numstat', params: {}, fn: () => gitMock.numstat as never },
    { method: 'git.commitDiff', params: { hash: 'abc123' }, fn: () => gitMock.commitDiff as never },
    {
      method: 'git.commitNumstat',
      params: { hash: 'abc123' },
      fn: () => gitMock.commitNumstat as never,
    },
    { method: 'git.stage', params: { path: 'a.txt' }, fn: () => gitMock.stage as never },
    { method: 'git.unstage', params: { path: 'a.txt' }, fn: () => gitMock.unstage as never },
    { method: 'git.commit', params: { message: 'wip' }, fn: () => gitMock.commit as never },
    { method: 'git.push', params: {}, fn: () => gitMock.push as never },
  ];

  /** Some handlers are async (git.diff), so a refusal surfaces as a rejection
   *  rather than a synchronous throw. Normalize both into a message. */
  async function refusal(method: string, params: unknown): Promise<string> {
    try {
      await call(method, params);
    } catch (err) {
      return err instanceof Error ? err.message : String(err);
    }
    throw new Error(`${method} returned instead of refusing`);
  }

  for (const { method, params, fn } of gitMethods) {
    it(`${method} is confined to the workspace roots`, async () => {
      const outside = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'wks-other-repo-')));
      expect(await refusal(method, { cwd: outside, ...params })).toMatch(
        /outside the allowed workspace/,
      );
      // A refusal that still ran the command would be worse than no guard.
      expect(fn().mock.calls, `${method} must not reach gitService`).toHaveLength(0);
    });

    it(`${method} refuses a cwd that leaves the roots through a symlink`, async () => {
      const outside = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'wks-other-repo-')));
      // The reason the guard canonicalizes rather than string-prefixing: the
      // link SITS inside the allowed root.
      const link = path.join(agentCwd, 'escape');
      fs.symlinkSync(outside, link);
      expect(await refusal(method, { cwd: link, ...params })).toMatch(
        /outside the allowed workspace/,
      );
      expect(fn().mock.calls, `${method} must not reach gitService`).toHaveLength(0);
    });

    it(`${method} runs for a live agent cwd, and gets the CANONICAL path`, async () => {
      // The floor: a guard that refused everything would satisfy both cases
      // above. And what reaches gitService has to be the resolved directory —
      // the checked path and the directory git runs in cannot differ.
      const inner = path.join(agentCwd, 'real');
      fs.mkdirSync(inner, { recursive: true });
      fs.symlinkSync(inner, path.join(agentCwd, 'alias'));
      await call(method, { cwd: path.join(agentCwd, 'alias'), ...params });
      expect(fn().mock.calls[0]?.[0], `${method} must receive the canonical cwd`).toBe(inner);
    });
  }

  it('covers every git.* capability the provider registers', () => {
    // Without this, adding git.blame with no entry above would leave it as
    // unpinned as the six this block was written for.
    const registeredGit = [...registered.keys()].filter((m) => m.startsWith('git.')).sort();
    expect(gitMethods.map((g) => g.method).sort()).toEqual(registeredGit);
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

    // The `untracked` leg is a different capability wearing the same name.
    // `git diff --no-index -- /dev/null <path>` renders ANY readable file as an
    // all-added diff — gitignored, untracked and tracked-but-unmodified alike —
    // none of which a path-less diff shows. So the "confining to the repo
    // concedes nothing" argument does not cover it, and the work-tree root is a
    // DERIVED directory nothing ever checked against the allow-list: an agent
    // cwd of <repo>/apps/desktop read <repo>/services/hub/.env this way, a file
    // fs.read and fs.watch refuse for the same caller.
    it('refuses an untracked read of a sibling subtree the tracked pathspec allows', async () => {
      await expect(
        call('git.diff', { cwd: agentCwd, path: 'services/hub/.env', untracked: true }),
      ).rejects.toThrow(/outside the allowed workspace/);
      expect(gitMock.diff).not.toHaveBeenCalled();
    });

    it('still allows an untracked path INSIDE the agent cwd', async () => {
      const rel = path.relative(repoRoot, path.join(agentCwd, 'src', 'new.ts'));
      await call('git.diff', { cwd: agentCwd, path: rel, untracked: true });
      expect(gitMock.diff).toHaveBeenCalledWith(agentCwd, rel, undefined, true);
    });

    // ── the staging leg ────────────────────────────────────────────────
    //
    // git.stage and a path-less git.diff{staged} COMPOSE into exfiltration that
    // neither is on its own. `git add` runs from the DERIVED work-tree root
    // (rev-parse --show-toplevel, a directory nothing ever checked against the
    // allow-list), and `path` reached gitService with no guard at all — so a
    // root-relative pathspec, or NO pathspec at all (`git add -A` over the whole
    // repository), put files outside every allowed root into the index, where
    // `git diff --staged` renders each of them as an all-added diff with full
    // content because they are not in HEAD. git.commit persists it,
    // git.commitDiff hands it back, git.push publishes it.
    it('git.stage refuses a sibling-subtree pathspec the tracked diff would allow', async () => {
      await expect(call('git.stage', { cwd: agentCwd, path: 'services/hub/.env' })).rejects.toThrow(
        /outside the allowed workspace/,
      );
      expect(gitMock.stage).not.toHaveBeenCalled();
    });

    it('git.stage refuses an absolute pathspec outside the repo', async () => {
      await expect(call('git.stage', { cwd: agentCwd, path: '/etc/shadow' })).rejects.toThrow(
        /outside the allowed workspace/,
      );
      expect(gitMock.stage).not.toHaveBeenCalled();
    });

    // The path-LESS form is the half a per-path guard cannot reach: `git add -A`
    // from the root stages the sibling subtree without naming it.
    it('git.stage with no path stages the guarded cwd, not the whole repository', async () => {
      await call('git.stage', { cwd: agentCwd });
      expect(gitMock.stage).toHaveBeenCalledWith(agentCwd, 'apps/desktop');
    });

    it('git.unstage with no path is bounded the same way', async () => {
      await call('git.unstage', { cwd: agentCwd });
      expect(gitMock.unstage).toHaveBeenCalledWith(agentCwd, 'apps/desktop');
    });

    it('git.unstage refuses a sibling-subtree pathspec', async () => {
      await expect(
        call('git.unstage', { cwd: agentCwd, path: 'services/hub/.env' }),
      ).rejects.toThrow(/outside the allowed workspace/);
      expect(gitMock.unstage).not.toHaveBeenCalled();
    });

    it('git.stage still stages a path inside the agent cwd, root-relative', async () => {
      const rel = path.relative(repoRoot, path.join(agentCwd, 'src', 'new.ts'));
      await call('git.stage', { cwd: agentCwd, path: rel });
      expect(gitMock.stage).toHaveBeenCalledWith(agentCwd, rel);
    });

    // ── the TRACKED leg, where the work-tree-root assertion is the ONLY guard ──
    //
    // git.diff passes `untracked ? [workspaceRoots()] : []` as extraRootSets, so
    // on a TRACKED diff the extra sets are EMPTY and the single
    // `assertPathAllowed(cap, anchored, [root])` is the whole boundary — its
    // containment half and its secret gate both. Every git-pathspec test above
    // rides an extraRootSet (untracked, stage, unstage), so replacing that line
    // with `const canonicalFile = anchored;` left 88 files / 1379 tests green.
    it('a TRACKED diff refuses a pathspec that climbs out of the work-tree root', async () => {
      await expect(
        call('git.diff', { cwd: agentCwd, path: '../../../etc/passwd' }),
      ).rejects.toThrow(/outside the allowed workspace/);
      expect(gitMock.diff).not.toHaveBeenCalled();
    });

    it('a TRACKED diff refuses an absolute pathspec outside the work-tree root', async () => {
      await expect(call('git.diff', { cwd: agentCwd, path: '/etc/shadow' })).rejects.toThrow(
        /outside the allowed workspace/,
      );
      expect(gitMock.diff).not.toHaveBeenCalled();
    });

    it('a TRACKED diff refuses a credential the secret gate names', async () => {
      // The gate only ever runs INSIDE assertPathAllowed. A modified ~/.gitconfig
      // routinely carries credential-helper settings and url.<base>.insteadOf
      // tokens, and `.bus-token` / `.git/config` are the same shape.
      for (const p of ['.git/config', '.bus-token', '.gitconfig']) {
        await expect(call('git.diff', { cwd: agentCwd, path: p })).rejects.toThrow(
          /outside the allowed workspace/,
        );
      }
      expect(gitMock.diff).not.toHaveBeenCalled();
    });

    // BINDING DECISION 2 on the OPERAND: what git receives is a function of the
    // CANONICAL path, never of the caller's string. `return filePath` survived
    // the whole suite because every test above happens to pass a string that is
    // already the answer.
    it('hands git the pathspec derived from the canonical path, not the caller string', async () => {
      await call('git.diff', { cwd: agentCwd, path: path.join(repoRoot, 'services', 'a.go') });
      expect(gitMock.diff).toHaveBeenCalledWith(agentCwd, 'services/a.go', undefined, undefined);
      gitMock.diff.mockClear();
      await call('git.diff', { cwd: agentCwd, path: 'services/./hub/../a.go' });
      expect(gitMock.diff).toHaveBeenCalledWith(agentCwd, 'services/a.go', undefined, undefined);
    });

    // cwdPathspec's own fail-closed precondition. Its comment says the assertion
    // "proves the cwd really is at-or-inside the derived root before path.relative
    // is trusted to describe it (a `..` result would be a pathspec pointing OUT of
    // the repo)". The helper's OUTPUT is pinned by the two tests above; the
    // precondition was pinned by nothing, and the work-tree root is DERIVED — a
    // gitfile, GIT_WORK_TREE or a submodule can make it a directory that does not
    // contain the cwd at all.
    it('git.stage with no path refuses a work-tree root that does not contain the cwd', async () => {
      const elsewhere = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'wks-elsewhere-')));
      workRootFor.mockImplementation(async () => elsewhere);
      await expect(call('git.stage', { cwd: agentCwd })).rejects.toThrow(
        /outside the allowed workspace/,
      );
      expect(gitMock.stage).not.toHaveBeenCalled();
      await expect(call('git.unstage', { cwd: agentCwd })).rejects.toThrow(
        /outside the allowed workspace/,
      );
      expect(gitMock.unstage).not.toHaveBeenCalled();
      fs.rmSync(elsewhere, { recursive: true, force: true });
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

describe('terminals.open — the visible-terminal seam', () => {
  beforeEach(() => emitToRenderer.mockClear());

  it('pushes a FACADE_OPEN_TERMINAL event to the renderer with the caller fields intact', () => {
    const res = call('terminals.open', {
      cwd: os.tmpdir(),
      command: 'npm run dev',
      label: 'preheat dev server',
      parentSessionId: 'MGR',
    });
    expect(res).toEqual({ ok: true });
    expect(emitToRenderer).toHaveBeenCalledTimes(1);
    const [channel, payload] = emitToRenderer.mock.calls[0] as [string, Record<string, unknown>];
    // The renderer opens the pane off this channel (IPC.FACADE_OPEN_TERMINAL).
    expect(channel).toBe('terminal:facade-open');
    expect(payload).toMatchObject({
      command: 'npm run dev',
      label: 'preheat dev server',
      parentSessionId: 'MGR',
    });
    // cwd is normalized (an existing dir survives); it is always a string.
    expect(typeof payload.cwd).toBe('string');
  });

  it('drops non-string fields rather than forwarding junk', () => {
    call('terminals.open', {
      cwd: os.tmpdir(),
      command: 123,
      label: { nope: true },
      parentSessionId: 'MGR',
    });
    const [, payload] = emitToRenderer.mock.calls[0] as [string, Record<string, unknown>];
    expect(payload.command).toBeUndefined();
    expect(payload.label).toBeUndefined();
    expect(payload.parentSessionId).toBe('MGR');
  });
});
