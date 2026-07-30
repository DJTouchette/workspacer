/**
 * What main still registers when the capability catalog is DELEGATED TO THE
 * BRAIN — the default configuration.
 *
 * hubCapabilities.ts registers through two doors: `registerCapability` (main
 * always owns it) and `cat` (a no-op under delegation, because the Go brain
 * serves that method instead). Picking the wrong door is silent: the capability
 * simply has no provider on the bus, and every remote call fails at runtime
 * with nothing failing at build or test time. fs.readImage shipped that way in
 * v0.147.0 — registered with `cat`, with no brain counterpart, so thumbnails
 * were broken for every web and remote client.
 *
 * The sibling suite (hubCapabilities.test.ts) mocks delegation OFF, which is
 * exactly why it could not catch this. Hence this file.
 */
import { describe, it, expect, vi } from 'vitest';

const registered = new Map<string, (params: unknown) => unknown>();
vi.mock('./hubClient', () => ({
  registerCapability: (method: string, handler: (params: unknown) => unknown) => {
    registered.set(method, handler);
  },
}));

// The default: the brain owns the catalog, so `cat(...)` registers nothing.
vi.mock('./brainDelegation', () => ({ DELEGATE_CATALOG_TO_BRAIN: true }));

vi.mock('electron', () => ({
  Notification: vi.fn().mockImplementation(() => ({ show: vi.fn() })),
}));
vi.mock('./managedSpawn', () => ({ spawnManagedAgent: vi.fn() }));
vi.mock('./claudeSpawn', () => ({ spawnClaudeAgent: vi.fn() }));
vi.mock('./claudemonSessionClient', () => ({ claudemonSessionClient: {} }));
vi.mock('./claudeSessionStore', () => ({
  claudeSessionStore: { getAllSnapshots: vi.fn(() => []), getSnapshot: vi.fn() },
}));
vi.mock('./agentProviders', () => ({ checkAllProviders: vi.fn(), resolveAgentBinary: vi.fn() }));
vi.mock('./configService', () => ({
  configService: { getConfig: vi.fn(() => ({})) },
  getConfigDir: vi.fn(() => '/nonexistent'),
}));
vi.mock('./agentHandoff', () => ({ agentHandoffBrief: vi.fn() }));
vi.mock('./claudeProfiles', () => ({ claudeProfiles: {} }));
vi.mock('../lib/appIcon', () => ({ appIconPath: () => undefined }));
vi.mock('./claudeModels', () => ({ listClaudeModels: vi.fn(() => []) }));
vi.mock('./libraryService', () => ({ libraryService: {} }));
vi.mock('./sessionService', () => ({ sessionService: {} }));
vi.mock('./sessionHistory', () => ({ sessionHistory: {} }));
vi.mock('./layoutService', () => ({ layoutService: {} }));
vi.mock('./claudeSessionList', () => ({ listClaudeSessionsForDir: vi.fn() }));
vi.mock('./recentSessions', () => ({ listRecentSessions: vi.fn() }));
vi.mock('./fileService', () => ({
  readTextFile: vi.fn(),
  writeTextFile: vi.fn(),
  listDir: vi.fn(),
}));
vi.mock('./imagePreview', () => ({ readImagePreview: vi.fn(() => ({ dataUrl: 'data:,' })) }));
vi.mock('./fileWatchService', () => ({ startWatch: vi.fn(), stopWatch: vi.fn() }));
vi.mock('./searchService', () => ({ searchProject: vi.fn() }));
vi.mock('./gitService', () => ({
  status: vi.fn(),
  diff: vi.fn(),
  numstat: vi.fn(),
  stage: vi.fn(),
  unstage: vi.fn(),
  commit: vi.fn(),
  push: vi.fn(),
}));
vi.mock('./terminalShare', () => ({}));
vi.mock('./supervisorSkill', () => ({ ensureSupervisorHome: vi.fn(() => '/home/super') }));

const { registerHubCapabilities } = await import('./hubCapabilities');
registerHubCapabilities();

describe('capability registration under brain delegation', () => {
  it('registers fs.readImage — the brain has no counterpart for it', () => {
    expect(registered.has('fs.readImage')).toBe(true);
  });

  it('leaves the catalog fs.* methods to the brain', () => {
    // If these ever appear here, either delegation broke or someone moved them
    // off `cat` — both worth noticing, since main and the brain would disagree
    // about who answers.
    expect(registered.has('fs.read')).toBe(false);
    expect(registered.has('fs.write')).toBe(false);
    expect(registered.has('fs.listEntries')).toBe(false);
  });

  it('still registers the non-catalog file caps main owns', () => {
    expect(registered.has('fs.watch')).toBe(true);
    expect(registered.has('fs.unwatch')).toBe(true);
  });
});
