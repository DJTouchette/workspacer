/**
 * Regression test: the `claude.profiles.add` hub capability must forward
 * mcpItemIds to claudeProfiles.addProfile. The web/remote Settings UI sends the
 * user's selected MCP servers in that field; the handler dropped it, so remote
 * profiles were created with no MCP servers (the desktop IPC path forwards it).
 *
 * SCOPE: this file covers the KILL-SWITCH copy of the handler, not the shipping
 * one. `claude.profiles.add` is registered through `cat()`, which is a no-op
 * unless DELEGATE_CATALOG_TO_BRAIN is false (i.e. WORKSPACER_NO_BRAIN=1) — by
 * default the headless Go brain is the single provider for it on the bus. The
 * mock below forces the non-delegated path on purpose, so main's copy is pinned
 * even though most users never reach it.
 *
 * TWIN: TestProfilesAddForwardsMcpItemIds / TestProfilesAddDefaultsMcpItemIds in
 * services/hub/cmd/brain/profiles_test.go pin the same two behaviours on the
 * copy that answers by default. Change the two together — the brain already
 * forwarded mcpItemIds, but it left the field nil (and `omitempty` dropped the
 * key), so the same method replied with two different shapes depending on which
 * provider ran until the Go side learned main's `?? []` default.
 */
import { describe, it, expect, vi, beforeEach } from 'vitest';

const { handlers, addProfile, updateProfile } = vi.hoisted(() => ({
  handlers: new Map<string, (params: unknown) => unknown>(),
  updateProfile: vi.fn((id: string, updates: unknown) => ({ id, ...(updates as object) })),
  addProfile: vi.fn(
    (name: string, configDir: string, extraArgs: string[], mcpItemIds: string[] = []) => ({
      id: 'p1',
      name,
      configDir,
      extraArgs,
      mcpItemIds,
      isDefault: true,
    }),
  ),
}));

// Capture every registered capability handler.
vi.mock('../../src/main/services/hubClient', () => ({
  registerCapability: (name: string, handler: (params: unknown) => unknown) => {
    handlers.set(name, handler);
  },
}));
// The REAL scrubBypassProfile, not a stub: the bus handler now scrubs at WRITE
// time (a bus caller must not be able to persist a CLAUDE_CONFIG_DIR or a
// permission bypass for the local user to pick up later), and stubbing it would
// make the assertions below assert nothing about that.
vi.mock('../../src/main/services/claudeProfiles', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../../src/main/services/claudeProfiles')>();
  return {
    claudeProfiles: { addProfile, updateProfile },
    scrubBypassProfile: actual.scrubBypassProfile,
  };
});

// Catalog capabilities (incl. claude.profiles.add) are registered by main only
// when catalog isn't delegated to the brain. Force the non-delegated path so
// this test exercises main's handler regardless of the runtime default.
vi.mock('../../src/main/services/brainDelegation', () => ({ DELEGATE_CATALOG_TO_BRAIN: false }));
// Stub the heavy/native/electron-touching deps so the module imports cleanly.
vi.mock('electron', () => ({ Notification: { isSupported: () => false } }));
vi.mock('../../src/main/services/claudeSessionStore', () => ({ claudeSessionStore: {} }));
vi.mock('../../src/main/services/claudemonSessionClient', () => ({ claudemonSessionClient: {} }));
vi.mock('../../src/main/services/claudeResolver', () => ({ buildClaudeArgv: vi.fn() }));
vi.mock('../../src/main/services/configService', () => ({
  configService: { getConfig: () => ({}) },
  getConfigDir: () => '/tmp',
}));
vi.mock('../../src/main/services/claudeModels', () => ({ listClaudeModels: vi.fn(() => []) }));
vi.mock('../../src/main/services/libraryService', () => ({ libraryService: {} }));
vi.mock('../../src/main/services/sessionService', () => ({ sessionService: {} }));
vi.mock('../../src/main/services/sessionHistory', () => ({ sessionHistory: {} }));
vi.mock('../../src/main/services/layoutService', () => ({ layoutService: {} }));
vi.mock('../../src/main/services/claudeSessionList', () => ({
  listClaudeSessionsForDir: vi.fn(() => []),
}));
vi.mock('../../src/main/services/fileService', () => ({
  readTextFile: vi.fn(),
  writeTextFile: vi.fn(),
  listDir: vi.fn(),
}));
vi.mock('../../src/main/services/fileWatchService', () => ({
  startWatch: vi.fn(),
  stopWatch: vi.fn(),
}));
vi.mock('../../src/main/services/searchService', () => ({ searchProject: vi.fn() }));

const { registerHubCapabilities } = await import('../../src/main/services/hubCapabilities');

beforeEach(() => {
  addProfile.mockClear();
  updateProfile.mockClear();
  handlers.clear();
  registerHubCapabilities();
});

describe('claude.profiles.add capability', () => {
  // mcpItemIds is SCRUBBED here, not forwarded. This test used to pin the
  // opposite, and the forwarding was deliberate — the comment at the call site
  // said the web/remote client sends the user's selected MCP servers.
  //
  // What that missed: a library item of kind `mcp` carries `command`, `args` and
  // `env` verbatim into a --mcp-config file, and the spawn passes
  // `--allowedTools mcp__<id>` alongside it, so the server is PRE-APPROVED and no
  // prompt gates it — a persisted id list is a persisted argv[0]. The id resolves
  // against a library a bus caller can write (library.save, or a plain fs.write
  // into <configDir>/library, a configStoreRoot by design), so there is nothing
  // to validate on the way in. SpawnAgentDialog copies a profile's mcpItemIds
  // into the spawn the moment the profile is selected, which is exactly the
  // "wait for the LOCAL user, where nothing scrubs" escalation this capability's
  // capspec reason claims to have closed — through the one field it did not.
  it('scrubs mcpItemIds at write time, like configDir and extraArgs', () => {
    const handler = handlers.get('claude.profiles.add')!;
    expect(handler).toBeTypeOf('function');
    // extraArgs spelled with a REMOTE-SAFE flag: this is a bus entry point and
    // the write is scrubbed (see the next test for the dropping half).
    handler({ name: 'P', extraArgs: ['--model', 'opus'], mcpItemIds: ['mcp-1', 'mcp-2'] });
    expect(addProfile).toHaveBeenCalledWith('P', '', ['--model', 'opus'], []);
  });

  it('scrubs mcpItemIds on update too — the other way to plant one', () => {
    handlers.get('claude.profiles.update')!({ id: 'p1', updates: { mcpItemIds: ['mcp-1'] } });
    expect(updateProfile).toHaveBeenCalledWith('p1', { mcpItemIds: [] });
  });

  it('defaults mcpItemIds to [] when absent', () => {
    handlers.get('claude.profiles.add')!({ name: 'P' });
    expect(addProfile).toHaveBeenCalledWith('P', '', [], []);
  });

  // Everything registered with cat() is a BUS entry point; the local Settings
  // write is a separate in-process IPC path (ipc.ts CLAUDE_PROFILES_ADD) and is
  // unaffected. scrubBypassProfile used to run only on the bus SPAWN, so a bus
  // caller could PERSIST a configDir (→ CLAUDE_CONFIG_DIR: settings.json,
  // permissions.allow and hooks — commands claude runs unprompted) plus
  // --dangerously-skip-permissions, then wait for the local user to pick that
  // profile in the New Agent dialog, where nothing scrubs. Twin:
  // TestProfilesWritesOverTheBusAreScrubbedAtWriteTime in the brain.
  it('scrubs configDir and bypass flags at WRITE time, not only at spawn time', () => {
    handlers.get('claude.profiles.add')!({
      name: 'pwn',
      configDir: '/tmp/attacker-claude-home',
      extraArgs: [
        '--dangerously-skip-permissions',
        '--settings',
        '/tmp/evil.json',
        '--model',
        'opus',
      ],
    });
    expect(addProfile).toHaveBeenCalledWith('pwn', '', ['--model', 'opus'], []);
  });

  it('scrubs the same fields on update', () => {
    handlers.get('claude.profiles.update')!({
      id: 'p1',
      updates: {
        configDir: '/tmp/attacker-claude-home',
        extraArgs: ['--dangerously-skip-permissions'],
      },
    });
    expect(updateProfile).toHaveBeenCalledWith('p1', { configDir: '', extraArgs: [] });

    // The floor: a remote-safe update still lands, and a field the caller did
    // not send is not invented.
    updateProfile.mockClear();
    handlers.get('claude.profiles.update')!({
      id: 'p1',
      updates: { extraArgs: ['--model', 'sonnet'] },
    });
    expect(updateProfile).toHaveBeenCalledWith('p1', { extraArgs: ['--model', 'sonnet'] });
  });
});
