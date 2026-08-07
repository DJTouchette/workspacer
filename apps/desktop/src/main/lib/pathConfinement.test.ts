// Cross-language path-containment drift guard.
//
// contracts/path-containment-cases.json is the SHARED fixture: Go tests over
// cmd/brain/fsguard.go and internal/bus/policy.go consume the exact same file.
// Three copies of this predicate ship (the brain answers fs.*/library.* under
// the default DELEGATE_CATALOG_TO_BRAIN, this one when delegation is off, the
// bus confines each plugin grant), and fsguard.go's header says they "must stay
// word for word" — nothing enforced that until this fixture.
//
// Each case runs against a REAL sandbox on disk, because the predicate resolves
// through the filesystem: symlinks, ENOTDIR and unreadable directories cannot be
// faked, and macOS /var -> /private/var alone would make literal absolute
// strings pass or fail for the wrong reason.

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';

// getConfigDir is the only collaborator. Importing configService for real
// instantiates its module-level singleton, whose constructor reads and writes
// the REAL config dir (the same reason deepMergeContract.test.ts mocks it), so
// the mock stands in for the whole module and points the secret gate at the
// per-case sandbox.
const state = vi.hoisted(() => ({ configDir: '' }));
vi.mock('../services/configService', () => ({ getConfigDir: () => state.configDir }));

import {
  assertPathAllowed,
  configStoreRoots,
  containsCanonical,
  isWithin,
  pathWithinRoots,
  SECRET_BASENAMES,
} from './pathConfinement';

interface Case {
  name: string;
  group: 'containment' | 'secrets';
  tree?: {
    dirs?: string[];
    files?: Record<string, string>;
    symlinks?: Record<string, string>;
    /** Link bodies written VERBATIM — the only way to reach the walk's
     *  relative-link arm, which every absolutized `symlinks` case leaves
     *  unexecuted. */
    relativeSymlinks?: Record<string, string>;
    modes?: Record<string, string>;
  };
  /** A sandbox-relative symlink to the config HOME that getConfigDir is pointed
   *  through, while ${CONFIG} keeps naming the real path — so the case passes
   *  only if the secret gate canonicalizes its own config dir. */
  configDirVia?: string;
  roots: string[];
  target: string;
  expect: 'allow' | 'deny';
  /** The token-substituted path assertPathAllowed must RETURN on an allow — the
   *  string every call site then hands to the filesystem (BINDING DECISION 2).
   *  Mandatory on every allow case; a deny returns no path. */
  resolvesTo?: string;
  why?: string;
  needsSymlinks?: boolean;
  posixOnly?: boolean;
  needsUnreadableDir?: boolean;
  needsHome?: boolean;
}

interface Fixture {
  owners: Record<string, string[]>;
  secretBasenames: string[];
  configStoreSubdirs: string[];
  checkUse: { owner: string; requirement: string; callSites?: string[] }[];
  cases: Case[];
}

const OWNER = 'apps/desktop/src/main/lib/pathConfinement.ts';

// apps/desktop/src/main/lib/ → five levels below the repo root, where contracts/ sits.
const fixture: Fixture = JSON.parse(
  fs.readFileSync(
    path.join(__dirname, '../../../../../contracts/path-containment-cases.json'),
    'utf-8',
  ),
);

const WIN32 = process.platform === 'win32';

/** Can this process create symlinks? (Windows without developer mode cannot.)
 *  A case that needs them is SKIPPED — reported as a skip, never as a pass. */
const CAN_SYMLINK = (() => {
  const probe = fs.mkdtempSync(path.join(os.tmpdir(), 'wks-symprobe-'));
  try {
    fs.symlinkSync(probe, path.join(probe, 'l'));
    return true;
  } catch {
    return false;
  } finally {
    fs.rmSync(probe, { recursive: true, force: true });
  }
})();

/** root reads a 0o000 directory regardless of its mode, so that case proves
 *  nothing when the tests run as root (containers, CI images). */
const CAN_HAVE_UNREADABLE_DIR = !WIN32 && (process.getuid?.() ?? 0) !== 0;

/** The two tokens that deliberately point OUTSIDE the per-case sandbox.
 *
 *  Every other case keeps root and target inside one temp dir, and that is
 *  exactly what made the tilde and bad-root cases vacuous: a '~' that expands to
 *  $HOME, or an empty root that resolves to the process cwd, still failed to
 *  contain a target under /tmp, so the deny verdict never moved and a copy that
 *  re-introduced either widening passed the whole corpus. These place the probe
 *  where the widening would actually land. Resolved, because every other path
 *  here is realpath'd and on macOS /var -> /private/var alone would make the
 *  comparison wrong. No case using them expects `allow`. */
const realpathOf = (p: string): string => {
  try {
    return fs.realpathSync(p);
  } catch {
    return p;
  }
};
const REAL_HOME = (() => {
  try {
    return realpathOf(os.homedir());
  } catch {
    return '';
  }
})();
const PROCESS_CWD = realpathOf(process.cwd());

function skipReason(c: Case): string | null {
  if (c.posixOnly && WIN32) return 'posixOnly';
  if (c.needsSymlinks && !CAN_SYMLINK) return 'needsSymlinks';
  if (c.needsUnreadableDir && !CAN_HAVE_UNREADABLE_DIR) return 'needsUnreadableDir';
  if (c.needsHome && !REAL_HOME) return 'needsHome';
  return null;
}

let sandbox = '';
let restoreModes: string[] = [];

beforeEach(() => {
  // realpath: on macOS os.tmpdir() is itself a symlink, and every expectation
  // below is about the RESOLVED answer.
  sandbox = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'wks-contain-')));
  restoreModes = [];
  // The fixture's ${CONFIG} token — every config-dir resolver in the repo
  // appends the 'workspacer' segment, so the mock returns it verbatim.
  state.configDir = path.join(sandbox, 'config', 'workspacer');
  for (const d of ['root', 'outside', path.join('config', 'workspacer')]) {
    fs.mkdirSync(path.join(sandbox, d), { recursive: true });
  }
});

afterEach(() => {
  // Restore anything `modes` locked down, or the teardown cannot recurse into it.
  for (const p of restoreModes) {
    try {
      fs.chmodSync(p, 0o700);
    } catch {}
  }
  if (sandbox) fs.rmSync(sandbox, { recursive: true, force: true });
  sandbox = '';
  state.configDir = '';
});

/** Sandbox-relative → absolute, without touching the caller-facing tokens. */
function abs(rel: string): string {
  return path.join(sandbox, ...rel.split('/'));
}

/** Materialize `tree` in exactly the fixture's order: dirs, files, symlinks, modes. */
function materialize(tree: Case['tree']): void {
  if (!tree) return;
  for (const d of tree.dirs ?? []) fs.mkdirSync(abs(d), { recursive: true });
  for (const [f, contents] of Object.entries(tree.files ?? {})) {
    const p = abs(f);
    fs.mkdirSync(path.dirname(p), { recursive: true });
    fs.writeFileSync(p, contents, 'utf-8');
  }
  for (const [link, target] of Object.entries(tree.symlinks ?? {})) {
    const p = abs(link);
    fs.mkdirSync(path.dirname(p), { recursive: true });
    // Absolutized here; `relativeSymlinks` below is the arm that is not.
    fs.symlinkSync(abs(target), p);
  }
  for (const [link, target] of Object.entries(tree.relativeSymlinks ?? {})) {
    const p = abs(link);
    fs.mkdirSync(path.dirname(p), { recursive: true });
    // Verbatim: resolving a relative link body is the implementation's job.
    fs.symlinkSync(target, p);
  }
  for (const [target, mode] of Object.entries(tree.modes ?? {})) {
    const p = abs(target);
    restoreModes.push(p);
    fs.chmodSync(p, parseInt(mode, 8));
  }
}

/** The ONE refusal message, spelled out because step 7.5 of the algorithm makes
 *  the text itself normative: all three refusal reasons produce it, and it names
 *  neither the target, the canonical path, the matched root, nor which gate
 *  fired. The Go twin pins the identical string (bus policy_test.go). */
const REFUSAL = 'contract: path is outside the allowed workspace (agent cwds + config stores)';

/** Run `fn`, require it to throw, and return the message. A deny case that
 *  RETURNED would otherwise be indistinguishable from one that threw the wrong
 *  error, and `.toThrow(/substring/)` would accept a message that echoed the
 *  denied path alongside it. */
function refusalMessage(fn: () => unknown): string {
  try {
    fn();
  } catch (err) {
    return err instanceof Error ? err.message : String(err);
  }
  throw new Error('expected a refusal, but the call returned a path');
}

/** Step 2.6's shape guarantee: what comes back is absolute and fully resolved —
 *  no '.', no '..', no repeated or trailing separator. Returns a complaint, or
 *  null when the string is well-formed. */
function canonicalShapeProblem(p: string): string | null {
  const region = WIN32
    ? p.replace(/^([A-Za-z]:\\|\\\\[^\\]+\\[^\\]+\\)/, '')
    : p.replace(/^\//, '');
  if (region === '') return null; // the bare volume prefix
  for (const part of region.split(path.sep)) {
    if (part === '') return 'a repeated or trailing separator survived';
    if (part === '.' || part === '..') return `an unresolved '${part}' component survived`;
  }
  return null;
}

/** Plain string substitution, applied to `roots` and `target` and nothing else:
 *  a root written without a token ("/", "", "~", "root") is literal on purpose. */
function subst(s: string): string {
  return s
    .split('${SANDBOX}')
    .join(sandbox)
    .split('${ROOT}')
    .join(path.join(sandbox, 'root'))
    .split('${OUTSIDE}')
    .join(path.join(sandbox, 'outside'))
    .split('${CONFIG}')
    .join(path.join(sandbox, 'config', 'workspacer'))
    .split('${HOME}')
    .join(REAL_HOME)
    .split('${PROCESS_CWD}')
    .join(PROCESS_CWD);
}

/** The fixture spells `resolvesTo` with '/' separators; the tokens inside it
 *  substitute to native ones. Only the fixture-authored separators need
 *  translating, and on POSIX this is a no-op. */
function nativeSep(s: string): string {
  return WIN32 ? s.split('/').join(path.sep) : s;
}

describe('path containment — cross-language contract', () => {
  it('the fixture loads and has cases for this owner', () => {
    // Renaming this file without updating the fixture has to FAIL, not silently
    // stop testing anything.
    expect(fixture.owners[OWNER], `contracts fixture must name ${OWNER} as an owner`).toBeDefined();
    expect([...fixture.owners[OWNER]].sort()).toEqual(['containment', 'secrets']);
    expect(fixture.cases.length).toBeGreaterThan(0);
    for (const group of ['containment', 'secrets']) {
      expect(
        fixture.cases.some((c) => c.group === group),
        `fixture has at least one ${group} case`,
      ).toBe(true);
    }
    expect(
      fixture.checkUse.some((e) => e.owner === OWNER),
      'the fixture records what this owner must do with the returned canonical path',
    ).toBe(true);
  });

  // The two constants are the parts of the `secrets` gate the CASES cannot
  // reach: every case names one of the two credential basenames and one of the
  // three stores, so adding a THIRD basename here (or dropping 'layouts') keeps
  // the whole corpus passing while the copies silently drift apart. The fixture carries
  // both lists for exactly this reason; hostTrustedConfig.test.ts pins its
  // section list the same way.
  it('denies exactly the credential basenames the fixture names', () => {
    expect([...SECRET_BASENAMES].sort()).toEqual([...fixture.secretBasenames].sort());
  });

  it('carves out exactly the config stores the fixture names, in that order', () => {
    // The carve-out is order-insensitive (any store that contains the target
    // wins), but toEqual pins the order too, and the Go twin's
    // configStoreSubdirs is the same literal list in the same order.
    expect(configStoreRoots()).toEqual(
      fixture.configStoreSubdirs.map((s) => path.join(state.configDir, s)),
    );
  });

  for (const c of fixture.cases) {
    const reason = skipReason(c);
    const run = reason ? it.skip : it;
    run(`[${c.group}] ${c.name}${reason ? ` (skipped: ${reason})` : ''}`, () => {
      // Before the tree, because the config dir has to be repointed through the
      // link while ${CONFIG} keeps substituting to the real path.
      if (c.configDirVia) {
        fs.symlinkSync(path.join(sandbox, 'config'), path.join(sandbox, c.configDirVia));
        state.configDir = path.join(sandbox, c.configDirVia, 'workspacer');
      }
      materialize(c.tree);
      const roots = c.roots.map(subst);
      const target = subst(c.target);

      if (c.expect === 'deny') {
        const message = refusalMessage(() => assertPathAllowed('contract', target, roots));
        expect(message, c.why).toBe(REFUSAL);
        // Restated as the property it exists for: a remote caller learns
        // nothing about where its path landed. The sandbox path is unique per
        // case, so this catches an echo the equality above would too — it is
        // here so a future rewording cannot quietly reintroduce one.
        expect(message, 'the refusal must not echo the path it denied').not.toContain(sandbox);
      } else {
        // An allow returns the CANONICAL path — the string every call site must
        // then hand to the filesystem operation (checkUse in the fixture).
        const canonical = assertPathAllowed('contract', target, roots);
        expect(typeof canonical, c.why).toBe('string');
        expect(path.isAbsolute(canonical)).toBe(true);
        expect(canonicalShapeProblem(canonical), `canonical form of ${c.target}`).toBeNull();
        // The VALUE, not only its shape. Shape alone is what let a whole-path
        // clean (path.normalize, or Go's filepath.Clean) satisfy every case in
        // this corpus in all three copies while returning a path that points at
        // a different file — Clean's answer is absolute and free of '.' and '..'
        // too. resolvesTo is mandatory on an allow so a future case cannot be
        // added without pinning the answer.
        expect(
          c.resolvesTo,
          `allow case "${c.name}" must carry resolvesTo — the path the guard returns is half the contract`,
        ).toBeTruthy();
        expect(canonical, c.why).toBe(nativeSep(subst(c.resolvesTo as string)));
      }
    });
  }
});

describe('the canonical path assertPathAllowed returns', () => {
  // checkUse, the other half of BINDING DECISION 2: the guard's answer is what
  // gets opened, so it has to be the RESOLVED path and not the caller's string.
  // Reported as a skip, never as a pass: `if (!CAN_SYMLINK) return` inside the
  // body would count as green on a Windows box that cannot make symlinks.
  const itLinks = CAN_SYMLINK ? it : it.skip;

  itLinks('is the resolved path when the target reaches the root through a symlink', () => {
    const real = path.join(sandbox, 'root', 'real');
    fs.mkdirSync(real, { recursive: true });
    fs.writeFileSync(path.join(real, 'x'), 'ok', 'utf-8');
    fs.symlinkSync(real, path.join(sandbox, 'root', 'inner'));

    const canonical = assertPathAllowed('contract', path.join(sandbox, 'root', 'inner', 'x'), [
      path.join(sandbox, 'root'),
    ]);
    expect(canonical).toBe(path.join(real, 'x'));
  });

  itLinks('resolves a symlink-then-".." target instead of collapsing it textually', () => {
    // The defect BINDING DECISION 2 exists for, from the returning side: the
    // fixture's case 'symlink then ".." that leaves and re-enters the root'
    // only asserts ALLOW, so a copy that collapsed '<root>/link/../root/x' to
    // '<root>/root/x' textually would still pass it. Here the answer itself is
    // pinned, which is the string the handler goes on to open.
    fs.mkdirSync(path.join(sandbox, 'outside'), { recursive: true });
    fs.writeFileSync(path.join(sandbox, 'root', 'notes.txt'), 'hello', 'utf-8');
    fs.symlinkSync(path.join(sandbox, 'outside'), path.join(sandbox, 'root', 'link'));

    // join(), not path.join(): path.join would collapse the '..' itself, which
    // is precisely the transformation under test.
    const target = [sandbox, 'root', 'link', '..', 'root', 'notes.txt'].join(path.sep);
    const canonical = assertPathAllowed('contract', target, [path.join(sandbox, 'root')]);
    expect(canonical).toBe(path.join(sandbox, 'root', 'notes.txt'));
  });

  it('keeps a not-yet-existing tail verbatim', () => {
    const root = path.join(sandbox, 'root');
    const canonical = assertPathAllowed('contract', path.join(root, 'a', 'b', 'new.txt'), [root]);
    expect(canonical).toBe(path.join(root, 'a', 'b', 'new.txt'));
  });
});

/**
 * The empty-root arm. `canonicalRoot` discards a root it cannot resolve, so ''
 * should never reach `containsCanonical` — but the LAST LINE OF DEFENCE must not
 * itself be the widest possible grant. Without the explicit test neither branch
 * sees a trailing separator and the comparison falls through to
 * `startsWith('/')`, which is true for every absolute path on the system: one
 * un-canonicalizable root anywhere upstream silently promotes a scoped grant to
 * whole-filesystem reach.
 *
 * Twins: the brain's TestAnEmptyRootContainsNothing and the bus's
 * TestCanonRootsDiscardsWhatItCannotResolve.
 */
describe('an empty root contains nothing', () => {
  it('does not behave as a wildcard', () => {
    for (const target of ['/etc/passwd', '/root/.ssh/id_rsa', path.sep, '/tmp/x/notes.txt']) {
      // Directly, the way the brain's twin asserts containsPath: reaching this
      // through isWithin/pathWithinRoots is not enough, because canonicalRoot('')
      // already fails and those two then answer false for their own reason.
      expect(containsCanonical('', target)).toBe(false);
      expect(pathWithinRoots([''], target)).toBe(false);
      expect(isWithin(target, '')).toBe(false);
    }
  });

  it('but the FILESYSTEM root still contains everything (BINDING DECISION 3)', () => {
    expect(containsCanonical(path.sep, '/etc/passwd')).toBe(true);
    expect(pathWithinRoots([path.sep], '/etc/passwd')).toBe(true);
  });
});
