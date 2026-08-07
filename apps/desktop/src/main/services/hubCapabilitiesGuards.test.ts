/**
 * Fixture-driven path-guard coverage for the capabilities MAIN owns in
 * production (DELEGATE_CATALOG_TO_BRAIN = true, the default).
 *
 * contracts/path-containment-cases.json's `methods` block is the register of
 * every capability that takes a caller-supplied filesystem path, which side
 * answers it, and which root set confines it. The Go brain's drift guard walks
 * the same block for its own eight; capspec's PathParam check keeps the block
 * itself complete, so a new path-taking capability cannot be added without
 * landing in it. This file closes the loop on the TypeScript side: for every
 * entry main serves on the bus in production, assert both that
 *
 *   1. the method is REGISTERED here (a miss is never tolerated as a skip — a
 *      silently absent capability is exactly the failure this family of tests
 *      exists to catch; fs.readImage shipped registered on the wrong door and
 *      broke every remote thumbnail with nothing failing at test time), and
 *   2. its declared path parameter is actually run through the confinement
 *      guard: out-of-roots is refused, in-roots is not.
 *
 * The mirror image is asserted too: entries the brain answers in production
 * (providers: ["main-killswitch"]) must NOT be registered here. Their main-side
 * handlers are covered by hubCapabilitiesKillSwitch.test.ts, which runs the same
 * loop with delegation off. Between the two files, every path-taking capability
 * main can ever serve is exercised in the mode it serves in.
 *
 * The predicate itself is not under test here — that is
 * main/lib/pathConfinement.ts, pinned across all three shipping copies by the
 * same fixture. What is under test is that these handlers CALL it.
 */

import { describe, it, expect, vi, beforeEach } from 'vitest';
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

// PRODUCTION MODE: the brain owns the catalog, so `cat(...)` registers nothing.
// The whole point of this file is the guard sweep in this mode; flipping it
// would make the "must be absent" half vacuous and the rest untrue of shipping.
vi.mock('./brainDelegation', () => ({ DELEGATE_CATALOG_TO_BRAIN: true }));

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
// filesystem, so the config-store roots must resolve to something real.
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

vi.mock('./agentHandoff', () => ({
  agentHandoffBrief: vi.fn(async () => ({ path: '/agent-brief.md' })),
}));

const openExternal = vi.fn(async () => {});
vi.mock('electron', () => {
  const NotificationMock = vi.fn(function (this: Record<string, unknown>) {
    this.show = vi.fn();
    this.on = vi.fn();
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
vi.mock('./recentSessions', () => ({ listRecentSessions: vi.fn(async () => []) }));
vi.mock('./fileService', () => ({
  readTextFile: vi.fn(),
  writeTextFile: vi.fn(),
  listDir: vi.fn(() => ({ path: '', entries: [] })),
}));
// fs.readImage is one of the methods swept below, and the real module decodes
// through electron's nativeImage — mock it so the guard, not the decoder, is
// what decides the outcome.
vi.mock('./imagePreview', () => ({ readImagePreview: vi.fn(() => ({ dataUrl: 'data:,' })) }));
vi.mock('./fileWatchService', () => ({ startWatch: vi.fn(), stopWatch: vi.fn() }));
vi.mock('./searchService', () => ({
  searchProject: vi.fn(() => ({ results: [], truncated: false })),
}));
// git.diff consults workRoot to anchor its `path` operand; default it to the cwd.
const workRootFor = vi.fn(async (cwd: string): Promise<string | null> => cwd);
vi.mock('./gitService', () => ({
  status: vi.fn(async () => ({ branch: 'main', files: [] })),
  log: vi.fn(async () => []),
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

/** Invoke a registered capability by method name. */
function call(method: string, params?: unknown): unknown {
  const handler = registered.get(method);
  if (!handler)
    throw new Error(`capability not registered under DELEGATE_CATALOG_TO_BRAIN=true: ${method}`);
  return handler(params);
}

/** Run a capability and return the error message it produced, or '' for none.
 *  The guards fire synchronously in some handlers and inside an async body in
 *  others (git.diff), so both shapes are collapsed before matching. */
async function attempt(method: string, params: unknown): Promise<string> {
  try {
    await call(method, params);
    return '';
  } catch (err) {
    return (err as Error).message;
  }
}

interface MethodEntry {
  method: string;
  field: string;
  params: Record<string, unknown>;
  rootSet: 'workspace' | 'browse';
  providers: string[];
  note?: string;
}

// src/main/services/ → five levels below the repo root, where contracts/ sits.
const fixture: { methods: MethodEntry[] } = JSON.parse(
  fs.readFileSync(
    path.join(__dirname, '../../../../../contracts/path-containment-cases.json'),
    'utf-8',
  ),
);
const mainOwned = fixture.methods.filter((m) => m.providers.includes('main'));
const brainOwned = fixture.methods.filter((m) => m.providers.includes('main-killswitch'));

let agentCwd: string;
let outside: string;

beforeEach(() => {
  vi.clearAllMocks();
  registered.clear();
  registerHubCapabilities();
  // One live agent cwd is the entire workspace root set that matters here; the
  // helpers canonicalize through the real filesystem, so it has to exist.
  agentCwd = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'wks-guard-')));
  // A real directory in neither root set: not a live agent cwd (outside
  // `workspace`) and under the temp dir rather than home (outside `browse`).
  outside = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'wks-guard-outside-')));
  getAllSnapshots.mockReturnValue([{ cwd: agentCwd }] as never);
});

describe('fixture-driven guard coverage — path capabilities main owns in production', () => {
  it('the fixture lists methods this owner serves', () => {
    // A fixture that stopped naming 'main' would silently turn every assertion
    // below into zero assertions.
    expect(mainOwned.length).toBeGreaterThan(0);
    expect(brainOwned.length).toBeGreaterThan(0);
  });

  for (const entry of mainOwned) {
    describe(entry.method, () => {
      it('is registered with the brain owning the catalog', () => {
        // Not a skip, not a conditional: if main does not register this in the
        // shipping mode, no provider answers it and every remote call fails.
        expect(registered.has(entry.method)).toBe(true);
      });

      it(`denies a ${entry.field} outside the ${entry.rootSet} roots`, async () => {
        // `browse` reaches the whole home tree, so an out-of-roots value has to
        // come from outside it entirely.
        const target = entry.rootSet === 'browse' ? '/etc' : outside;
        const msg = await attempt(entry.method, { ...entry.params, [entry.field]: target });
        expect(msg).toMatch(/outside the allowed workspace/);
      });

      it(`allows a ${entry.field} inside a live agent cwd`, async () => {
        // A live agent cwd is in both root sets, so one value serves either.
        const msg = await attempt(entry.method, { ...entry.params, [entry.field]: agentCwd });
        expect(msg).not.toMatch(/outside the allowed workspace/);
      });
    });
  }

  // The mirror image. These are `cat`-door methods: with delegation on the Go
  // brain is the single provider, and main registering them too would collide on
  // a router that is single-owner per method.
  for (const entry of brainOwned) {
    it(`${entry.method} is left to the brain (not registered here)`, () => {
      expect(registered.has(entry.method)).toBe(false);
    });
  }
});
