/**
 * Regression test: the `claude.profiles.add` hub capability must forward
 * mcpItemIds to claudeProfiles.addProfile. The web/remote Settings UI sends the
 * user's selected MCP servers in that field; the handler dropped it, so remote
 * profiles were created with no MCP servers (the desktop IPC path forwards it).
 */
import { describe, it, expect, vi, beforeEach } from 'vitest';

const { handlers, addProfile } = vi.hoisted(() => ({
  handlers: new Map<string, (params: unknown) => unknown>(),
  addProfile: vi.fn((name: string, configDir: string, extraArgs: string[], mcpItemIds: string[] = []) => ({
    id: 'p1', name, configDir, extraArgs, mcpItemIds, isDefault: true,
  })),
}));

// Capture every registered capability handler.
vi.mock('../../src/main/services/hubClient', () => ({
  registerCapability: (name: string, handler: (params: unknown) => unknown) => { handlers.set(name, handler); },
}));
vi.mock('../../src/main/services/claudeProfiles', () => ({ claudeProfiles: { addProfile } }));

// Catalog capabilities (incl. claude.profiles.add) are registered by main only
// when catalog isn't delegated to the brain. Force the non-delegated path so
// this test exercises main's handler regardless of the runtime default.
vi.mock('../../src/main/services/brainDelegation', () => ({ DELEGATE_CATALOG_TO_BRAIN: false }));
// Stub the heavy/native/electron-touching deps so the module imports cleanly.
vi.mock('electron', () => ({ Notification: { isSupported: () => false } }));
vi.mock('../../src/main/services/claudeSessionStore', () => ({ claudeSessionStore: {} }));
vi.mock('../../src/main/services/claudemonSessionClient', () => ({ claudemonSessionClient: {} }));
vi.mock('../../src/main/services/claudeResolver', () => ({ buildClaudeArgv: vi.fn() }));
vi.mock('../../src/main/services/configService', () => ({ configService: { getConfig: () => ({}) }, getConfigDir: () => '/tmp' }));
vi.mock('../../src/main/services/claudeModels', () => ({ listClaudeModels: vi.fn(() => []) }));
vi.mock('../../src/main/services/libraryService', () => ({ libraryService: {} }));
vi.mock('../../src/main/services/sessionService', () => ({ sessionService: {} }));
vi.mock('../../src/main/services/sessionHistory', () => ({ sessionHistory: {} }));
vi.mock('../../src/main/services/layoutService', () => ({ layoutService: {} }));
vi.mock('../../src/main/services/claudeSessionList', () => ({ listClaudeSessionsForDir: vi.fn(() => []) }));
vi.mock('../../src/main/services/fileService', () => ({ readTextFile: vi.fn(), writeTextFile: vi.fn(), listDir: vi.fn() }));
vi.mock('../../src/main/services/fileWatchService', () => ({ startWatch: vi.fn(), stopWatch: vi.fn() }));
vi.mock('../../src/main/services/searchService', () => ({ searchProject: vi.fn() }));

const { registerHubCapabilities } = await import('../../src/main/services/hubCapabilities');

beforeEach(() => {
  addProfile.mockClear();
  handlers.clear();
  registerHubCapabilities();
});

describe('claude.profiles.add capability', () => {
  it('forwards mcpItemIds to addProfile', () => {
    const handler = handlers.get('claude.profiles.add')!;
    expect(handler).toBeTypeOf('function');
    handler({ name: 'P', configDir: '/c', extraArgs: ['--x'], mcpItemIds: ['mcp-1', 'mcp-2'] });
    expect(addProfile).toHaveBeenCalledWith('P', '/c', ['--x'], ['mcp-1', 'mcp-2']);
  });

  it('defaults mcpItemIds to [] when absent', () => {
    handlers.get('claude.profiles.add')!({ name: 'P' });
    expect(addProfile).toHaveBeenCalledWith('P', '', [], []);
  });
});
