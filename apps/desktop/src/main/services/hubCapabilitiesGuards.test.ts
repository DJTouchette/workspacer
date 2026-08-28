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

import { describe, it, expect, vi, beforeEach, afterEach, afterAll } from 'vitest';
import {
  itRanEveryGatedTest,
  gatedIt,
  CAN_SYMLINK,
  SweepTally,
  itSweptTheWholeCorpus,
} from '../../../tests/support/sweepTally';
import * as os from 'os';
import * as path from 'path';
import * as fs from 'fs';

/** Can this process create symlinks? (Windows without developer mode cannot.)
 *  A test that needs them is SKIPPED and then COUNTED — reported as a skip,
 *  never as a pass, and never as a silent zero. */
const CAN_SYMLINK_HERE = CAN_SYMLINK;

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
  checkAllProvidersCached: (...a: unknown[]) => checkAllProviders(...a),
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
// Recorded, not just stubbed: replay.open's confinement is one line, and what
// that line is worth depends on WHICH string reaches the worktree cutter.
const replayOpenCalls = vi.hoisted(() => [] as string[]);
vi.mock('./timelineReplayService', () => ({
  timelineReplay: {
    open: (cwd: string) => {
      replayOpenCalls.push(cwd);
      return { ok: true };
    },
    seek: vi.fn(),
    close: vi.fn(),
    read: vi.fn(),
    diff: vi.fn(),
  },
}));

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
let sandboxHome: string;
const realHome = { HOME: process.env.HOME, USERPROFILE: process.env.USERPROFILE };

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
  // A HOME of our own. browseRoots() is [os.homedir(), ...workspaceRoots()], so
  // the only probe that can tell the two root sets apart lives under the home
  // tree — and pointing that probe at the developer's real home would mean the
  // sweep writes there the moment a handler is widened. os.homedir() reads $HOME
  // on POSIX and %USERPROFILE% on Windows; both are redirected, and the
  // assertion below refuses to run if the redirect did not take.
  sandboxHome = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'wks-guard-home-')));
  process.env.HOME = sandboxHome;
  process.env.USERPROFILE = sandboxHome;
  getAllSnapshots.mockReturnValue([{ cwd: agentCwd }] as never);
});

afterEach(() => {
  process.env.HOME = realHome.HOME;
  process.env.USERPROFILE = realHome.USERPROFILE;
  for (const d of [agentCwd, outside, sandboxHome]) fs.rmSync(d, { recursive: true, force: true });
});

afterAll(() => {
  fs.rmSync(cfg.dir, { recursive: true, force: true });
});

/**
 * The probe that separates `workspace` from `browse`, and the reason this file
 * needed one.
 *
 * browse = workspace roots + os.homedir(). Both fixture-driven sweeps used to
 * probe out-of-roots with an os.tmpdir() path (workspace) or '/etc' (browse) —
 * and os.tmpdir() is outside BOTH sets while a live agent cwd is inside both, so
 * no probe here could ever tell the two apart. The fixture's `rootSet` column
 * was therefore decorative on this side: fs.read, fs.readImage, fs.listEntries,
 * fs.watch, fs.unwatch and search.project could each be swapped from
 * workspaceRoots() to browseRoots() with the entire desktop suite green, which
 * turns bus-reachable capabilities into readers of ~/.ssh/id_rsa,
 * ~/.aws/credentials, ~/.claude/.credentials.json and ~/.netrc (none of which is
 * a secret BASENAME or inside the config dir, so the second gate does not catch
 * them either). fs.readImage, fs.watch and fs.unwatch are main-only in
 * production, so for those three this is the ONLY oracle that exists.
 *
 * A path under the sandbox home that is nobody's cwd is inside `browse` and
 * outside `workspace`, by construction.
 */
function homeProbe(): string {
  // If the redirect did not take, the probe would name the real home — fail
  // rather than silently assert against it.
  expect(os.homedir()).toBe(sandboxHome);
  const probe = path.join(sandboxHome, 'wks-contract-probe-not-an-agent-cwd');
  // A fresh sandbox per test, so a collision means the sandbox is not fresh and
  // the probe proves nothing. Never a skip: the Go twin of this sweep skipped on
  // exactly this condition, and a widened fs.write created the very file that
  // made it skip — permanently, for all eight of its methods.
  expect(fs.existsSync(probe)).toBe(false);
  return probe;
}

/**
 * A directory that is a SIBLING of the sandbox home: inside `dirname($HOME)`,
 * outside `$HOME`, not a live agent cwd and not a config store. It exists so the
 * `browse` root set has an upper boundary probe at all — see the deny case.
 */
function homeSiblingProbe(): string {
  expect(os.homedir()).toBe(sandboxHome);
  const probe = path.join(path.dirname(sandboxHome), 'wks-contract-probe-sibling-of-home');
  expect(fs.existsSync(probe)).toBe(false);
  return probe;
}

describe('fixture-driven guard coverage — path capabilities main owns in production', () => {
  it('the fixture lists methods this owner serves', () => {
    // A fixture that stopped naming 'main' would silently turn every assertion
    // below into zero assertions.
    expect(mainOwned.length).toBeGreaterThan(0);
    expect(brainOwned.length).toBeGreaterThan(0);
  });

  // `mainOwned.length > 0` above counts what the FIXTURE lists. This counts what
  // ran: a filter that stopped matching, or a describe whose bodies all threw
  // past their assertions, leaves the length check green.
  const mainSweep = new SweepTally();
  for (const entry of mainOwned) {
    describe(entry.method, () => {
      it('is registered with the brain owning the catalog', () => {
        mainSweep.ran('other');
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

        // The UPPER boundary of `browse`, which nothing probed. browse is "the
        // home tree plus the workspace roots", and BOTH deny probes above sit
        // outside $HOME's PARENT as well — so widening browseRoots() to
        // `path.dirname(os.homedir())` (every other user's home) changed no
        // assertion in either sweep, while fs.listDir then enumerated a
        // stranger's home and library.list handed back the BODIES of its
        // .workspacer/library items. A SIBLING of $HOME is inside the widened
        // set and outside the real one, so it must be refused for either rootSet.
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
        // See homeProbe: this is the only assertion in either sweep that can
        // distinguish workspaceRoots() from browseRoots().
        const msg = await attempt(entry.method, { ...entry.params, [entry.field]: homeProbe() });
        if (entry.rootSet === 'browse') {
          expect(msg).not.toMatch(/outside the allowed workspace/);
        } else {
          expect(msg).toMatch(/outside the allowed workspace/);
        }
      });
    });
  }

  // replay.open cannot live in the `methods` block — that set must equal
  // capspec.PathParam exactly, and replay.open is deliberately NOT bus-scoped:
  // capspec.unscopedByDecision grants it to a plugin with no fsRoots at all, on
  // the stated grounds that "the provider confines it to the same workspace
  // roots git.* uses (assertPathAllowed in hubCapabilities.ts)". So this one
  // line is the ONLY confinement the method has, on either side.
  //
  // capspec_test.go's TestUnscopedByDecisionProviderClaimsAreTrue greps this
  // file for that call, which catches a deletion — but a grep only proves the
  // call was WRITTEN. A guard behind an `if`, or one whose return value is
  // dropped, passes it and confines nothing. replay.open cuts a git worktree
  // from the repo at `cwd` and replay.read/replay.diff then read files out of
  // it — bytes fs.read would refuse — so the claim gets a behavioural test too.
  describe('replay.open (bus-unscoped by decision; the provider is the whole guard)', () => {
    const replayGate = { ran: 0 };
    const itLinks = gatedIt(CAN_SYMLINK_HERE, replayGate);
    it('denies a cwd outside the workspace roots', async () => {
      const msg = await attempt('replay.open', { cwd: outside, sessionId: 's1' });
      expect(msg).toMatch(/outside the allowed workspace/);
    });

    it('lets a live agent cwd past the confinement check', async () => {
      // Asserting the negative is what keeps a guard that refuses everything
      // from passing.
      const msg = await attempt('replay.open', { cwd: agentCwd, sessionId: 's1' });
      expect(msg).not.toMatch(/outside the allowed workspace/);
    });

    // BINDING DECISION 2 on the one path handoff that had no assertion. The two
    // tests above assert the DENY and the not-DENY and nothing else, so
    // `(assertPathAllowed('replay.open', cwd, workspaceRoots()), cwd)` — keep the
    // check, hand the service the caller's raw string — passed the whole suite.
    // The same substitution on fs.read/fs.write/fs.readImage/fs.listEntries/
    // fs.watch/fs.unwatch/fs.listDir/search.project/library.list IS killed;
    // replay.open was the one left. It cuts a git worktree from the string it is
    // handed, and replay.read/replay.diff then serve files out of that worktree.
    // `(CAN_SYMLINK ? it : it.skip)`, not `try { symlink } catch { return }`.
    // The swallowing form REPORTS A PASS on a host with no symlink privilege
    // while asserting nothing, and this is the only test of replay.open's
    // check-path/use-path identity. The floor below then makes an all-skipped
    // group red rather than green.
    itLinks('cuts the worktree from the CANONICAL cwd, not the callerstring', async () => {
      const link = path.join(path.dirname(agentCwd), `${path.basename(agentCwd)}-link`);
      fs.symlinkSync(agentCwd, link);
      replayOpenCalls.length = 0;
      const msg = await attempt('replay.open', { cwd: link, sessionId: 's1' });
      fs.rmSync(link, { force: true });
      expect(msg).toBe('');
      expect(
        replayOpenCalls,
        'replay.open handed the replay service the string it was given rather than the one the guard validated',
      ).toEqual([agentCwd]);
    });

    itRanEveryGatedTest(replayGate, "replay.open's canonical-cwd test", 1);
  });

  // The mirror image. These are `cat`-door methods: with delegation on the Go
  // brain is the single provider, and main registering them too would collide on
  // a router that is single-owner per method.
  const brainSweep = new SweepTally();
  for (const entry of brainOwned) {
    it(`${entry.method} is left to the brain (not registered here)`, () => {
      brainSweep.ran('other');
      expect(registered.has(entry.method)).toBe(false);
    });
  }

  // BINDING DECISION 1 at the HANDLERS, which is where it was unpinned.
  //
  // "NO TILDE EXPANSION, at any layer that handles a caller-supplied path" —
  // fsguard.go's header, and the corpus carries six tilde cases for it. But
  // every one of them calls the PREDICATE directly, so they pin only that
  // canonicalizePath treats '~' as an ordinary name. Nothing pinned that a
  // handler hands the predicate the caller's string UNMODIFIED, and the
  // fixture's own prose names that as the hazard: "the brain used to
  // expandTilde() every guarded path while TypeScript did not" — a LAYER above
  // the predicate. Inserting one back at any of these handlers survived the
  // whole 86-file suite; under it `fs.listDir({path:'~'})` returned the $HOME
  // listing and `library.list({cwd:'~'})` returned the BODIES of
  // $HOME/.claude/{skills,agents,commands}/*.md.
  //
  // $HOME is pointed at the live agent cwd for the probe, so the expanded form
  // is inside BOTH root sets: without that, a `workspace`-rootSet method refuses
  // the expansion for the wrong reason and the mutant survives at five of the
  // ten sites. An agent whose cwd IS $HOME is not contrived — it is what a bare
  // `agents.spawn({})` produces, since normalizeSpawnCwd('') returns the home
  // directory.
  const tildeSweep = new SweepTally();
  for (const entry of mainOwned) {
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

  // Ratcheted to the number of methods each owner serves today. A fixture whose
  // `providers` lists shrank would otherwise still satisfy `.length > 0`.
  itSweptTheWholeCorpus(tildeSweep, "the no-tilde-expansion sweep over 'main'", 6, {
    allow: 0,
    deny: 0,
  });
  itSweptTheWholeCorpus(mainSweep, "the methods 'main' owns in production", 6, {
    allow: 0,
    deny: 0,
  });
  itSweptTheWholeCorpus(brainSweep, 'the methods left to the brain', 7, { allow: 0, deny: 0 });
});
