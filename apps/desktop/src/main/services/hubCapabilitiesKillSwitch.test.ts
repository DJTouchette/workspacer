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

import { describe, it, expect, vi, beforeEach, afterEach, afterAll } from 'vitest';
import {
  SweepTally,
  itSweptTheWholeCorpus,
  itRanEveryGatedTest,
  gatedIt,
  CAN_SYMLINK,
} from '../../../tests/support/sweepTally';
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
  spawn: vi.fn(async () => 'terminal-session-id'),
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

// Every temp dir a beforeEach/test creates goes through here so the matching
// afterEach can remove it — this suite runs under mutation testing, where a
// leaked dir per test multiplies into hundreds of thousands in /tmp.
const tmpDirs: string[] = [];
const mkTmp = (prefix: string): string => {
  const dir = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), prefix)));
  tmpDirs.push(dir);
  return dir;
};
afterEach(() => {
  while (tmpDirs.length) fs.rmSync(tmpDirs.pop()!, { recursive: true, force: true });
});
afterAll(() => {
  fs.rmSync(cfg.dir, { recursive: true, force: true });
});

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
vi.mock('./imagePreview', () => ({ readImagePreview: vi.fn(() => ({ dataUrl: '', bytes: 0 })) }));
vi.mock('./searchService', () => ({
  searchProject: vi.fn(() => ({ results: [], truncated: false })),
}));
// `workRoot` is part of the mock because git.diff's path guard consults it:
// gitService runs every command from the work-tree toplevel, so that — not the
// caller's cwd — is what a `path` is resolved against. Default it to the cwd
// (repo root == agent cwd); the nested-cwd test overrides it.
const workRootFor = vi.fn(async (cwd: string): Promise<string | null> => cwd);
const gitStatus = vi.fn(async () => ({ branch: 'main', files: [] }));
vi.mock('./gitService', () => ({
  status: (cwd: string) => gitStatus(cwd),
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
const { readTextFile, writeTextFile, listDir } = await import('./fileService');
const { shellConfig } = await import('../lib/shellAllowlist');
const { startWatch, stopWatch } = await import('./fileWatchService');
const { readImagePreview } = await import('./imagePreview');
const { searchProject } = await import('./searchService');

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
    agentCwd = mkTmp('wks-agent-');
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

  // checkUse — the other half of BINDING DECISION 2, which the fixture states
  // and nothing enforced. Every allow-case above hands the service a path that
  // is already its own canonical form (the temp dir is realpath'd), so a handler
  // that went back to re-passing `params.path` satisfied all of them.
  //
  // `<cwd>/nope/../notes.txt` separates the two: the walk appends a component
  // that does not exist and lets the following '..' pop back onto ground that
  // does (the corpus's "'..' after a non-existent component pops lexically and
  // lands inside", an ALLOW case), so the guard returns `<cwd>/notes.txt` —
  // while the raw string is a different string, and the kernel would refuse it
  // with ENOENT because `nope` is not there to walk through.
  describe('the guarded handlers pass the CANONICAL path on, not the caller string', () => {
    const via = (name: string): string => `${path.join(agentCwd, 'nope')}/../${name}`;

    // Every case below is named for the exact `callSites` entry it covers, and
    // the set is compared against the fixture at the end of the block. checkUse
    // used to be read only for the presence of this owner's KEY — never the
    // requirement text and never the call-site list — so a hand-maintained table
    // that covered eight of ten entries looked complete.
    const covered = new Set<string>();
    const site = (name: string, fn: () => void | Promise<void>): void => {
      covered.add(name);
      it(name, fn);
    };

    site('fs.read -> readTextFile', () => {
      call('fs.read', { path: via('notes.txt') });
      expect(readTextFile).toHaveBeenCalledWith(path.join(agentCwd, 'notes.txt'));
    });

    site('fs.write -> writeTextFile', () => {
      call('fs.write', { path: via('out.txt'), contents: 'x' });
      expect(writeTextFile).toHaveBeenCalledWith(path.join(agentCwd, 'out.txt'), 'x');
    });

    site('fs.listEntries -> listDir', () => {
      call('fs.listEntries', { path: `${path.join(agentCwd, 'nope')}/..` });
      expect(listDir).toHaveBeenCalledWith(agentCwd);
    });

    site('fs.listDir -> readdirSync', () => {
      const res = call('fs.listDir', { path: `${path.join(agentCwd, 'nope')}/..` }) as {
        path: string;
      };
      expect(res.path).toBe(agentCwd);
    });

    site('search.project -> searchProject', () => {
      call('search.project', { cwd: `${path.join(agentCwd, 'nope')}/..`, query: 'needle' });
      expect(searchProject).toHaveBeenCalledWith(
        expect.objectContaining({ cwd: agentCwd, query: 'needle' }),
      );
    });

    // The three capabilities the DESKTOP is the only provider of. The corpus's
    // checkUse block names them among pathConfinement.ts's ten call sites, and
    // this describe covered five — so for exactly the three with no Go twin to
    // fall back on, nothing asserted which string reached the service. That is
    // not cosmetic: startWatch/stopWatch still run `path.resolve(filePath)` on
    // whatever they are handed — a second, whole-path opinion of the same kind
    // that shipped as the brain's trailing-space escape — so re-passing the raw
    // param re-creates a check-path/opened-path split rather than merely an
    // untested one.
    site('fs.readImage -> readImagePreview', () => {
      call('fs.readImage', { path: via('shot.png') });
      expect(readImagePreview).toHaveBeenCalledWith(path.join(agentCwd, 'shot.png'));
    });

    site('fs.watch -> startWatch', () => {
      call('fs.watch', { path: via('watched.txt') });
      expect(vi.mocked(startWatch).mock.calls[0]![0]).toBe(path.join(agentCwd, 'watched.txt'));
    });

    site('fs.unwatch -> stopWatch', () => {
      call('fs.unwatch', { path: via('watched.txt') });
      expect(vi.mocked(stopWatch).mock.calls[0]![0]).toBe(path.join(agentCwd, 'watched.txt'));
    });

    // The last two entries in the owner's list, which nothing reached. Both
    // handlers guard a `cwd` and then hand it to a service that composes many
    // more paths out of it, so re-passing the caller's string here is the widest
    // version of the split.
    site('library.list/save/remove -> libraryService', () => {
      const canonical = agentCwd;
      call('library.list', { cwd: `${path.join(agentCwd, 'nope')}/..` });
      expect(libraryMock.list.mock.calls[0]![0]).toBe(canonical);
      call('library.save', {
        scope: 'project',
        cwd: `${path.join(agentCwd, 'nope')}/..`,
        title: 't',
        kind: 'prompt',
        body: 'b',
      });
      expect((libraryMock.save.mock.calls[0]![0] as { cwd: string }).cwd).toBe(canonical);
      call('library.remove', {
        scope: 'project',
        id: 'x',
        cwd: `${path.join(agentCwd, 'nope')}/..`,
      });
      expect(libraryMock.remove.mock.calls[0]![2]).toBe(canonical);
    });

    site('git.* -> gitService', async () => {
      await call('git.status', { cwd: `${path.join(agentCwd, 'nope')}/..` });
      expect(gitStatus.mock.calls[0]![0]).toBe(agentCwd);
    });

    it('covers exactly the call sites the fixture names for this owner', () => {
      const entry = fixture.checkUse.find(
        (e: { owner: string }) => e.owner === 'apps/desktop/src/main/lib/pathConfinement.ts',
      );
      expect(
        entry?.callSites?.length,
        'the fixture must record this owner call sites',
      ).toBeTruthy();
      expect([...covered].sort()).toEqual([...(entry!.callSites as string[])].sort());
    });
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
    agentCwd = mkTmp('wks-lib-');
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

  it('library.save hands the service a per-file guard that confines the DESTINATION', () => {
    // The cwd check is not the write check: the destination
    // (<cwd>/.workspacer/library/<slug>.md, <cwd>/.claude/skills/<id>/SKILL.md)
    // is composed after it and handed to writeFileSync, which follows a symlink
    // planted there by an ordinary permitted fs.write. list and remove got this
    // guard and save did not, so the desktop overwrote <configDir>/config.yaml
    // through a call the Go brain refuses. (libraryService.save's own end of the
    // contract — that it uses this guard, before mkdir, on the path it opens —
    // is pinned in libraryService.test.ts.)
    call('library.save', {
      scope: 'project',
      cwd: agentCwd,
      title: 't',
      kind: 'prompt',
      body: 'b',
    });
    const guard = libraryMock.save.mock.calls[0][1] as (p: string) => string | null;
    expect(typeof guard).toBe('function');
    expect(guard(path.join(agentCwd, '.workspacer', 'library', 'ok.md'))).toBe(
      path.join(agentCwd, '.workspacer', 'library', 'ok.md'),
    );
    expect(guard(path.join(cfg.dir, 'config.yaml'))).toBeNull();
    expect(guard(path.join(cfg.dir, 'remote-token'))).toBeNull();
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
    expect(libraryMock.list).toHaveBeenCalledWith(notYetSpawned, expect.any(Function));
  });

  // Confining the cwd is not the same thing as confining what the service then
  // touches: every read and unlink is a path DERIVED from that cwd
  // (<cwd>/.workspacer/library/<name>.md, <cwd>/.claude/skills/<id>), composed
  // after the check. Unguarded, a symlink planted in the allowed project — an
  // ordinary permitted fs.write — read remote-token through library.list and
  // rm -rf'd the config dir through library.remove. So the handler passes a
  // per-file guard, and these assert the guard it passed actually confines.
  it('library.list hands the service a per-file guard that confines derived paths', () => {
    call('library.list', { cwd: agentCwd });
    const guard = libraryMock.list.mock.calls[0][1] as (p: string) => string | null;
    expect(guard(path.join(agentCwd, '.workspacer', 'library', 'ok.md'))).toBe(
      path.join(agentCwd, '.workspacer', 'library', 'ok.md'),
    );
    expect(guard(path.join(cfg.dir, 'remote-token'))).toBeNull();
    expect(guard(path.join(os.tmpdir(), 'somewhere-else', 'x.md'))).toBeNull();
    // …and the guard's roots are NOT the cwd's roots. library.list checks its
    // cwd against the browse roots (workspace + the whole home tree) because the
    // New Agent dialog lists a directory no agent runs in yet — handing those
    // same roots to the per-file guard made this call an arbitrary
    // home-directory READER: `<cwd>/.workspacer/library/a.md -> ~/.ssh/id_rsa`
    // canonicalizes inside $HOME, passes, and comes back as an item body, while
    // fs.read of the identical path is refused. A library item lives in the
    // project or in the global store, and nowhere else.
    //
    // Written against $HOME rather than a temp path on purpose: the assertion
    // above only holds because os.tmpdir() happens to sit outside the home tree,
    // so it inverts under TMPDIR=~/tmp and proved nothing about this widening.
    expect(guard(path.join(os.homedir(), '.ssh', 'id_rsa'))).toBeNull();
    expect(guard(path.join(os.homedir(), '.claude', '.credentials.json'))).toBeNull();
    // The global store stays readable: it is the other place items live.
    expect(guard(path.join(cfg.dir, 'library', 'shared.md'))).toBe(
      path.join(cfg.dir, 'library', 'shared.md'),
    );
  });

  it('library.remove hands the service a per-file guard that confines delete targets', () => {
    call('library.remove', { scope: 'claude', id: 'x', cwd: agentCwd, kind: 'skill' });
    const guard = libraryMock.remove.mock.calls[0][4] as (p: string) => string | null;
    expect(guard(path.join(agentCwd, '.claude', 'skills', 'x'))).toBe(
      path.join(agentCwd, '.claude', 'skills', 'x'),
    );
    expect(guard(path.join(cfg.dir, 'remote-token'))).toBeNull();
    expect(guard(path.join(cfg.dir, 'config.yaml'))).toBeNull();
  });

  // The per-file guard's ANSWER, which every assertion above leaves free.
  //
  // They all call `guard(x)` on symlink-free paths and expect `x` back, so
  // `assertPathAllowed(cap, filePath, roots); return filePath;` — the guard
  // deciding correctly and then handing back the CALLER-DERIVED string — passed
  // 188/188 focused and the whole 86-file main suite. `path.resolve(filePath)`
  // passed too. This is BINDING DECISION 2 at the one factory that supplies the
  // guard libraryService uses for every readFileSync / writeFileSync /
  // mkdirSync / fs.watch / unlinkSync / rmSync of a DERIVED library path, and
  // the module header says "every caller must hand the RETURNED canonical path
  // to the filesystem".
  //
  // Only a symlink separates the two strings: an alias and its target are both
  // legal, both inside the item roots, and they name different files. os.unlink
  // and fs.watch do not follow the final link while the guard does, so which
  // string comes back decides which file is opened.
  //
  // TWIN: the three `(per-file guard)` subtests of
  // TestGuardedHandlersOpenTheCanonicalPathTheyValidated in
  // services/hub/cmd/brain/fsguard_test.go.
  const perFileGate = { ran: 0 };
  const itLinks = gatedIt(CAN_SYMLINK, perFileGate);

  for (const leg of [
    { method: 'library.list', params: {}, argIndex: -1 },
    { method: 'library.remove', params: { scope: 'claude', id: 'x', kind: 'skill' }, argIndex: -1 },
    {
      method: 'library.save',
      params: { scope: 'project', title: 't', kind: 'prompt', body: 'b' },
      argIndex: -1,
    },
  ]) {
    itLinks(`${leg.method}'s per-file guard returns the RESOLVED path, not the caller's`, () => {
      const itemDir = path.join(agentCwd, '.workspacer', 'library');
      fs.mkdirSync(itemDir, { recursive: true });
      const target = path.join(itemDir, 'target.md');
      const alias = path.join(itemDir, 'alias.md');
      fs.writeFileSync(target, '---\ntitle: T\n---\n\nb\n', 'utf-8');
      fs.symlinkSync(target, alias);

      call(leg.method, { ...leg.params, cwd: agentCwd });
      const mock = (libraryMock as unknown as Record<string, { mock: { calls: unknown[][] } }>)[
        leg.method.split('.')[1]
      ];
      const guard = mock.mock.calls[0].at(leg.argIndex) as (p: string) => string | null;
      expect(typeof guard).toBe('function');
      expect(
        guard(alias),
        'the guard must hand back the file it validated, not the link the caller named',
      ).toBe(target);
    });
  }

  itRanEveryGatedTest(perFileGate, "the per-file guard's canonical-answer tests", 3);
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
  /** The narrower list a method's DERIVED paths are confined to, when it has
   *  one: 'item' = [<configDir>/library, cwd]. Absent for methods that open the
   *  field they were handed. */
  derivedRootSet?: 'item';
  providers: string[];
  note?: string;
}

// src/main/services/ → five levels below the repo root, where contracts/ sits.
const fixture: {
  methods: MethodEntry[];
  checkUse: { owner: string; requirement: string; callSites?: string[] }[];
} = JSON.parse(
  fs.readFileSync(
    path.join(__dirname, '../../../../../contracts/path-containment-cases.json'),
    'utf-8',
  ),
);
const killSwitchOnly = fixture.methods.filter((m) => m.providers.includes('main-killswitch'));

describe('fixture-driven guard coverage — kill-switch-only path capabilities', () => {
  let agentCwd: string;
  let outside: string;
  let sandboxHome: string;
  const realHome = { HOME: process.env.HOME, USERPROFILE: process.env.USERPROFILE };
  beforeEach(() => {
    agentCwd = mkTmp('wks-guard-');
    // A real directory that is in neither root set: not a live agent cwd (so it
    // is outside `workspace`) and under the temp dir rather than home (so it is
    // outside `browse` too, which is why the browse cases use /etc instead).
    outside = mkTmp('wks-guard-outside-');
    // A HOME of our own — see homeProbe below for why the probe has to live
    // under the home tree, and why it must not be the developer's real one.
    sandboxHome = mkTmp('wks-guard-home-');
    process.env.HOME = sandboxHome;
    process.env.USERPROFILE = sandboxHome;
    getAllSnapshots.mockReturnValue([{ cwd: agentCwd }] as never);
  });

  afterEach(() => {
    process.env.HOME = realHome.HOME;
    process.env.USERPROFILE = realHome.USERPROFILE;
  });

  /**
   * The probe that separates `workspace` from `browse`.
   *
   * browse = workspace roots + os.homedir(). This sweep used to probe
   * out-of-roots with an os.tmpdir() path for `workspace` and '/etc' for
   * `browse` — os.tmpdir() is outside BOTH sets and a live agent cwd is inside
   * both, so nothing here could distinguish workspaceRoots() from browseRoots()
   * and the fixture's `rootSet` column was decorative. Six methods could be
   * widened to the whole home tree with 951/951 desktop tests and all 19 Go
   * packages green.
   *
   * A path under the sandbox home that is nobody's cwd is inside `browse` and
   * outside `workspace`, by construction. The home is sandboxed because the
   * sweep drives the real handlers, so a widened fs.write LANDS a file in
   * whatever home it is pointed at — which is how the Go twin of this sweep
   * disarmed itself: it probed a fixed name in the real $HOME and skipped when
   * that name existed, and its own fs.write subtest created it.
   */
  function homeProbe(): string {
    expect(os.homedir()).toBe(sandboxHome);
    const probe = path.join(sandboxHome, 'wks-contract-probe-not-an-agent-cwd');
    // Never a skip on collision: "I could not run" must not read as "I passed".
    expect(fs.existsSync(probe)).toBe(false);
    return probe;
  }

  /** A SIBLING of the sandbox home: inside `dirname($HOME)`, outside `$HOME`,
   *  nobody's cwd and no config store. The upper boundary probe for `browse`. */
  function homeSiblingProbe(): string {
    expect(os.homedir()).toBe(sandboxHome);
    const probe = path.join(path.dirname(sandboxHome), 'wks-contract-probe-sibling-of-home');
    expect(fs.existsSync(probe)).toBe(false);
    return probe;
  }

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

  const killSweep = new SweepTally();
  for (const entry of killSwitchOnly) {
    describe(entry.method, () => {
      it('is registered when the kill switch is on', () => {
        killSweep.ran('other');
        expect(registered.has(entry.method)).toBe(true);
      });

      it(`denies a ${entry.field} outside the ${entry.rootSet} roots`, async () => {
        // `browse` reaches the whole home tree, so the out-of-roots value has to
        // come from outside it entirely.
        const target = entry.rootSet === 'browse' ? '/etc' : outside;
        const msg = await attempt(entry.method, { ...entry.params, [entry.field]: target });
        expect(msg).toMatch(/outside the allowed workspace/);

        // The UPPER boundary of `browse`, which nothing probed. Both deny values
        // above are outside $HOME's PARENT too, so widening browseRoots() to
        // `path.dirname(os.homedir())` — every other user's home — changed no
        // assertion in either sweep while fs.listDir enumerated a stranger's
        // home and library.list handed back the bodies of its library items.
        const sibMsg = await attempt(entry.method, {
          ...entry.params,
          [entry.field]: homeSiblingProbe(),
        });
        expect(sibMsg, 'neither root set reaches outside the home tree').toMatch(
          /outside the allowed workspace/,
        );
      });

      it(`allows a ${entry.field} inside a live agent cwd`, async () => {
        // A live agent cwd is in both root sets, so one value serves either.
        const msg = await attempt(entry.method, { ...entry.params, [entry.field]: agentCwd });
        expect(msg).not.toMatch(/outside the allowed workspace/);
      });

      it(`consults the ${entry.rootSet} roots and not the other set`, async () => {
        const msg = await attempt(entry.method, { ...entry.params, [entry.field]: homeProbe() });
        if (entry.rootSet === 'browse') {
          expect(msg).not.toMatch(/outside the allowed workspace/);
        } else {
          expect(msg).toMatch(/outside the allowed workspace/);
        }
      });
    });
  }

  // derivedRootSet is the SECOND allow-list: library.* confine the caller's cwd
  // against `rootSet` and then confine every path DERIVED from it against
  // [<configDir>/library, cwd] — strictly narrower than either root set, because
  // a library item may only live in the global store or the project the caller
  // named. Conflating the two is what the brain did (it used the WORKSPACE roots
  // for the derived write), so one library.save with a
  // `<projA>/.workspacer/library -> <projB>` symlink wrote into a second agent's
  // project and into <configDir>/sessions there and was refused here. This pins
  // the side that was already right, so the two cannot re-diverge in either
  // direction. The Go half is TestLibraryDerivedRootSetIsTheItemRoots.
  // The filter is a place this sweep can silently become zero tests: a fixture
  // that renamed `derivedRootSet` or its value would register nothing here and
  // the file would still be green. Assert the selection before using it.
  const derivedItemMethods = killSwitchOnly.filter((m) => m.derivedRootSet === 'item');
  it('[floor] the fixture still names methods whose derived guard uses the item roots', () => {
    expect(
      derivedItemMethods.map((m) => m.method),
      "no methods carry derivedRootSet 'item' — this sweep just became zero tests",
    ).not.toEqual([]);
  });

  const derivedSweep = new SweepTally();
  for (const entry of derivedItemMethods) {
    it(`${entry.method}'s per-file guard uses the item roots, not the ${entry.rootSet} roots`, () => {
      derivedSweep.ran('other');
      const secondAgentCwd = mkTmp('wks-projB-');
      getAllSnapshots.mockReturnValue([{ cwd: agentCwd }, { cwd: secondAgentCwd }] as never);
      const mock = (libraryMock as unknown as Record<string, { mock: { calls: unknown[][] } }>)[
        entry.method.split('.')[1]
      ];
      call(entry.method, { ...entry.params, cwd: agentCwd });
      const guard = mock.mock.calls[0].at(-1) as (p: string) => string | null;
      expect(typeof guard).toBe('function');
      // Both cwds are live agents, so both are inside the workspace roots. Only
      // the one the caller named is inside the item roots.
      expect(guard(path.join(secondAgentCwd, 'pwn.md'))).toBeNull();
      expect(guard(path.join(cfg.dir, 'sessions', 'pwn.md'))).toBeNull();
      expect(guard(path.join(cfg.dir, 'layouts', 'pwn.md'))).toBeNull();
      // …while the two places an item legitimately lives still pass.
      const inProject = path.join(agentCwd, '.workspacer', 'library', 'ok.md');
      expect(guard(inProject)).toBe(inProject);
      const inStore = path.join(cfg.dir, 'library', 'ok.md');
      expect(guard(inStore)).toBe(inStore);
    });
  }

  // BINDING DECISION 1 at the HANDLERS. The corpus's six tilde cases all call
  // the PREDICATE, so they pin only that canonicalizePath treats '~' as an
  // ordinary name; nothing pinned that a handler hands the predicate the
  // caller's string UNMODIFIED, and the fixture's own prose names that layer as
  // the hazard ("the brain used to expandTilde() every guarded path while
  // TypeScript did not"). A tilde pre-pass inserted at any of these handlers
  // survived the whole suite: under it `fs.listDir({path:'~'})` returned the
  // $HOME listing and `library.list({cwd:'~'})` returned the BODIES of
  // $HOME/.claude/{skills,agents,commands}/*.md.
  //
  // $HOME is pointed at the live agent cwd so the expanded form is inside BOTH
  // root sets — otherwise a `workspace`-rootSet method refuses the expansion for
  // the wrong reason and the mutant survives. An agent whose cwd IS $HOME is
  // what a bare `agents.spawn({})` produces.
  //
  // TWIN: the same sweep over the providers:["main"] entries in
  // hubCapabilitiesGuards.test.ts.
  const tildeSweep = new SweepTally();
  for (const entry of killSwitchOnly) {
    it(`${entry.method} does not expand '~' before the guard sees it`, async () => {
      tildeSweep.ran('other');
      const saved = { HOME: process.env.HOME, USERPROFILE: process.env.USERPROFILE };
      process.env.HOME = agentCwd;
      process.env.USERPROFILE = agentCwd;
      try {
        expect(os.homedir(), 'the probe needs $HOME to be an allowed root').toBe(agentCwd);
        for (const spelling of ['~', '~/notes.txt', '~/']) {
          const msg = await attempt(entry.method, { ...entry.params, [entry.field]: spelling });
          expect(
            msg,
            `${entry.method} accepted ${JSON.stringify(spelling)} — '~' is an ordinary filename here, so a '~'-prefixed path is not absolute and must be refused (BINDING DECISION 1)`,
          ).toMatch(/outside the allowed workspace/);
        }
      } finally {
        process.env.HOME = saved.HOME;
        process.env.USERPROFILE = saved.USERPROFILE;
      }
    });
  }

  // Ratcheted to the sizes the fixture carries today. Both `.length > 0` checks
  // above read the FIXTURE; these read the run.
  itSweptTheWholeCorpus(
    tildeSweep,
    'the no-tilde-expansion sweep over the kill-switch methods',
    7,
    {
      allow: 0,
      deny: 0,
    },
  );
  itSweptTheWholeCorpus(killSweep, 'the kill-switch-owned method sweep', 7, {
    allow: 0,
    deny: 0,
  });
  itSweptTheWholeCorpus(derivedSweep, "the derivedRootSet 'item' sweep", 3, {
    allow: 0,
    deny: 0,
  });
});

/**
 * terminals.create's `shell` is argv[0] of a process spawned on the host, taken
 * verbatim from a bus caller. capspec leaves the capability unscoped and its
 * recorded reason named ONE param (`cwd`); nothing checked the other one in
 * either provider. Pair it with an fs.write over an existing executable inside
 * the caller's own agent cwd — writeFileSync preserves the 0755 — and
 * terminals.create alone was arbitrary host code execution.
 *
 * TWIN: TestTerminalsCreateRefusesAShellThatIsNotOne in the Go brain.
 */
describe('terminals.create — shell is an allowlist, not a passthrough', () => {
  const realEtcShells = shellConfig.etcShellsPath;
  const realShell = process.env.SHELL;
  let sandbox: string;

  beforeEach(() => {
    sandbox = mkTmp('wks-term-');
    shellConfig.etcShellsPath = path.join(sandbox, 'shells');
    fs.writeFileSync(shellConfig.etcShellsPath, '/bin/bash\n');
    process.env.SHELL = '/bin/zsh';
    clientMock.spawn.mockClear();
  });
  afterEach(() => {
    shellConfig.etcShellsPath = realEtcShells;
    if (realShell === undefined) delete process.env.SHELL;
    else process.env.SHELL = realShell;
    fs.rmSync(sandbox, { recursive: true, force: true });
  });

  it('refuses an arbitrary executable, before any spawn is attempted', async () => {
    const planted = path.join(sandbox, 'node_modules', '.bin', 'tsc');
    fs.mkdirSync(path.dirname(planted), { recursive: true });
    fs.writeFileSync(planted, '#!/bin/sh\nid > /tmp/pwned\n', { mode: 0o755 });

    await expect(
      call('terminals.create', { shell: planted, cwd: sandbox }) as Promise<unknown>,
    ).rejects.toThrow(/login shells/);
    expect(clientMock.spawn).not.toHaveBeenCalled();
  });

  it('still spawns a real login shell, and defaults when none is named', async () => {
    await call('terminals.create', { shell: '/bin/zsh', cwd: sandbox });
    expect(clientMock.spawn).toHaveBeenCalledWith(expect.objectContaining({ argv: ['/bin/zsh'] }));

    clientMock.spawn.mockClear();
    await call('terminals.create', { cwd: sandbox });
    expect(clientMock.spawn).toHaveBeenCalledWith(expect.objectContaining({ argv: ['/bin/zsh'] }));
  });
});

/**
 * workspaceRoots() is the allow-list every workspace-confined capability uses —
 * fs.read / fs.write / fs.listEntries / fs.readImage / fs.watch / fs.unwatch /
 * search.project / all ten git.* / library.save / library.remove / replay.open —
 * so its MEMBERSHIP is the security boundary and has to be pinned in both
 * directions.
 *
 * It was pinned in neither. Adding one line (`roots.add(process.cwd())`) and
 * deleting one line (the configStoreRoots loop) each left 84 files / 1213 tests
 * green. Neither fixture-driven sweep can see it: they probe {agent cwd},
 * {os.tmpdir() sibling}, {$HOME}, {$HOME sibling} — none of which is
 * process.cwd() — and the one test that touches a config store sets
 * `agentCwd = path.dirname(cfg.dir)`, so the store is inside the agent cwd
 * anyway and the configStoreRoots leg is never required.
 */
describe('what workspaceRoots() is made of', () => {
  let agentCwd: string;
  beforeEach(() => {
    agentCwd = mkTmp('wks-roots-');
    getAllSnapshots.mockReturnValue([{ cwd: agentCwd }] as never);
  });

  it('does not grant the desktop process its OWN working directory', () => {
    // For a packaged Electron main, process.cwd() is wherever the app was
    // launched from — routinely $HOME, and on a macOS Dock launch, '/'.
    const probe = path.join(fs.realpathSync(process.cwd()), 'wks-not-an-agent-cwd.txt');
    expect(
      () => call('fs.read', { path: probe }),
      "fs.read of a file under the desktop process's own cwd was ALLOWED",
    ).toThrow(/outside the allowed workspace/);
    expect(readTextFile).not.toHaveBeenCalled();
  });

  it('keeps the config stores in, with NO agent running', () => {
    // No live agents at all, so the only thing that can admit these is the
    // configStoreRoots leg. The web/remote UI reads and writes its own
    // library/layouts/sessions this way.
    getAllSnapshots.mockReturnValue([] as never);
    for (const store of ['library', 'layouts', 'sessions']) {
      const item = path.join(cfg.dir, store, 'entry.yaml');
      expect(() => call('fs.read', { path: item }), store).not.toThrow();
    }
    expect(readTextFile).toHaveBeenCalledTimes(3);
  });

  it('does not grant the rest of the config dir along with them', () => {
    getAllSnapshots.mockReturnValue([] as never);
    for (const rel of ['remote-token', 'config.yaml', path.join('plugins', 'p', 'manifest.json')]) {
      expect(() => call('fs.read', { path: path.join(cfg.dir, rel) }), rel).toThrow(
        /outside the allowed workspace/,
      );
    }
  });
});

/**
 * guardLibraryCwd's absent-cwd posture. With no `cwd`, the guard returns
 * undefined, so libraryItemRoots is just `[<configDir>/library]` — and
 * libraryService's own internal `input.cwd || process.cwd()` fallback then
 * composes a project destination the PER-FILE guard refuses. That pairing is
 * deliberate and fail-closed, and nothing exercised it: `if (!cwd) return
 * process.cwd();` left the whole suite green while turning the app's working
 * directory into a writable and deletable library root.
 */
describe('library.* with no cwd at all', () => {
  beforeEach(() => {
    getAllSnapshots.mockReturnValue([] as never);
  });

  it('passes NO cwd through to the service rather than substituting one', () => {
    call('library.save', { scope: 'project', title: 't', kind: 'prompt', body: 'b' });
    expect(libraryMock.save).toHaveBeenCalledTimes(1);
    const [input] = libraryMock.save.mock.calls[0] as [{ cwd?: string }];
    expect(
      input.cwd,
      'a missing cwd was replaced with a real directory — that directory becomes a library item root',
    ).toBeUndefined();
  });

  it('refuses a project-scope write derived from a substituted cwd', () => {
    // The pairing made behavioural: the per-file guard the handler builds must
    // not admit anything under the process working directory.
    call('library.save', { scope: 'project', title: 't', kind: 'prompt', body: 'b' });
    const [, guard] = libraryMock.save.mock.calls[0] as [unknown, (p: string) => string | null];
    expect(typeof guard).toBe('function');
    const underProcessCwd = path.join(
      fs.realpathSync(process.cwd()),
      '.workspacer',
      'library',
      't.md',
    );
    expect(
      guard(underProcessCwd),
      'the per-file guard admitted a path under the process working directory',
    ).toBeNull();
    // Floor: the global store IS admitted, or the assertion above is satisfied
    // by a guard that refuses everything.
    expect(guard(path.join(cfg.dir, 'library', 't.md'))).not.toBeNull();
  });
});
