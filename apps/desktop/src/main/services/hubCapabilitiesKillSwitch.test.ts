/**
 * The KILL-SWITCH copy of the path-confined catalog capabilities.
 *
 * READ THIS BEFORE TRUSTING ANY ASSERTION IN THIS FILE AS PRODUCTION COVERAGE.
 *
 * The handlers exercised here — fs.read / fs.write / fs.listEntries /
 * fs.listDir and library.list / library.save / library.remove — register
 * through hubCapabilities.ts's `cat` door, which is a no-op whenever
 * DELEGATE_CATALOG_TO_BRAIN is true. That is the default and the shipping
 * configuration. Main only serves these when the user sets WORKSPACER_NO_BRAIN=1
 * (the escape hatch for a missing/broken packaged brain binary); the rest of the
 * time the answering copy of the guard is services/hub/cmd/brain/fsguard.go.
 *
 * So this file mocks delegation OFF deliberately, and it is the ONLY file
 * allowed to. Its siblings run in the production mode:
 *   - hubCapabilities.test.ts       — main's registerCapability surface;
 *   - hubCapabilitiesGuards.test.ts — the same fixture-driven guard sweep as
 *                                     below, for the methods main owns in
 *                                     production;
 *   - hubCapabilitiesDelegated.test.ts — which door each capability uses.
 *
 * The PREDICATE these cases lean on is not owned here either: it lives in
 * main/lib/pathConfinement.ts and is pinned across all three shipping copies
 * (this one, fsguard.go, internal/bus/policy.go) by the shared fixture
 * contracts/path-containment-cases.json. What this file adds on top is that
 * main's kill-switch handlers actually CALL that predicate, with the right root
 * set, before touching the filesystem.
 *
 * Strategy: mock ./hubClient so registerCapability records handlers into a map
 * we can invoke directly, and mock every collaborator so only the capability
 * bodies run. The mock preamble is a verbatim copy of hubCapabilities.test.ts's;
 * the two files differ only in the ./brainDelegation mock.
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

// KILL-SWITCH MODE (WORKSPACER_NO_BRAIN=1): delegation OFF, so `cat(...)` is the
// real registerCapability and main's own catalog handlers exist on the bus. This
// is NOT the default configuration — see the header.
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

/** Invoke a registered capability by method name. */
function call(method: string, params?: unknown): unknown {
  const handler = registered.get(method);
  if (!handler)
    throw new Error(
      `capability not registered under DELEGATE_CATALOG_TO_BRAIN=false: ${method} — ` +
        `with the kill switch on main registers everything, so a miss here is a ` +
        `renamed or deleted capability, not a delegation split`,
    );
  return handler(params);
}

beforeEach(() => {
  vi.clearAllMocks();
  registered.clear();
  registerHubCapabilities();
});

describe('fs.* path confinement', () => {
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

// ── Fixture-driven sweep over every kill-switch-only path capability ────────
//
// The hand-written describes above pick their cases; this one cannot. It walks
// contracts/path-containment-cases.json's `methods` block — the same list the
// Go side and the capspec drift guard walk — and demands that EVERY method main
// serves only under the kill switch registers here and refuses an out-of-roots
// value for its declared path parameter. A method added to the fixture with
// providers: ["main-killswitch"] and no guard fails here without anyone
// remembering to write a test for it.
//
// The production twin of this loop is in hubCapabilitiesGuards.test.ts, which
// covers the providers: ["main"] entries and asserts the mirror image — that
// the methods below are ABSENT with delegation on.
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
const killSwitchOnly = fixture.methods.filter((m) => m.providers.includes('main-killswitch'));

describe('fixture-driven guard coverage — kill-switch-only path capabilities', () => {
  let agentCwd: string;
  let outside: string;
  beforeEach(() => {
    agentCwd = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'wks-guard-')));
    // A real directory that is in neither root set: not a live agent cwd (so it
    // is outside `workspace`) and under the temp dir rather than home (so it is
    // outside `browse` too, which is why the browse cases use /etc instead).
    outside = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'wks-guard-outside-')));
    getAllSnapshots.mockReturnValue([{ cwd: agentCwd }] as never);
  });

  /** Run a capability and return the error message it produced, or '' for none.
   *  Handlers guard both synchronously and inside an async body, so both shapes
   *  have to be collapsed before the message can be matched. */
  async function attempt(method: string, params: unknown): Promise<string> {
    try {
      await call(method, params);
      return '';
    } catch (err) {
      return (err as Error).message;
    }
  }

  it('the fixture lists methods this owner serves', () => {
    // A fixture that stopped naming 'main-killswitch' would silently turn every
    // assertion below into zero assertions.
    expect(killSwitchOnly.length).toBeGreaterThan(0);
  });

  for (const entry of killSwitchOnly) {
    describe(entry.method, () => {
      it('is registered when the kill switch is on', () => {
        expect(registered.has(entry.method)).toBe(true);
      });

      it(`denies a ${entry.field} outside the ${entry.rootSet} roots`, async () => {
        // `browse` reaches the whole home tree, so the out-of-roots value has to
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
});
