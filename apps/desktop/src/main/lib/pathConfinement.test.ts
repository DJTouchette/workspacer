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
import {
  SweepTally,
  itRanEveryGatedTest,
  gatedIt,
  CAN_SYMLINK,
  itSweptTheWholeCorpus,
} from '../../../tests/support/sweepTally';
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
  canonicalizePath,
  configStoreRoots,
  containsCanonical,
  isWithin,
  pathWithinRoots,
  SECRET_BASENAMES,
  asciiLower,
  isSecretPath,
  traversesGitDir,
  resolveStoreEntry,
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
  /** The RIGHT-REASON half of a deny, named from the fixture's
   *  `vocabulary.denyReasons`. `expect: 'deny'` on its own is satisfied by a
   *  refusal for ANY reason — including "the token did not substitute, so the
   *  target was a relative literal" — so every deny case says which of the four
   *  outcomes it exercises and `denyReason()` has to land on it. Mandatory on a
   *  deny, forbidden on an allow (which carries resolvesTo instead). */
  deniedBy?: string;
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

/** The fixture's declared vocabulary: the token names a loader may substitute,
 *  the `group` names a case may belong to, and the reasons a deny may be denied
 *  for. Every one of the three used to be validated by nothing at all. */
interface Vocabulary {
  tokens: Record<string, string>;
  groups: Record<string, string>;
  denyReasons: Record<string, string>;
}

interface Fixture {
  vocabulary: Vocabulary;
  owners: Record<string, string[]>;
  secretBasenames: string[];
  configStoreSubdirs: string[];
  checkUse: { owner: string; requirement: string; callSites?: string[] }[];
  asciiFold: { cases: { in: string; out: string }[] };
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
let savedXdg: string | undefined;

beforeEach(() => {
  // realpath: on macOS os.tmpdir() is itself a symlink, and every expectation
  // below is about the RESOLVED answer.
  sandbox = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'wks-contain-')));
  restoreModes = [];
  // The fixture's ${CONFIG} token — every config-dir resolver in the repo
  // appends the 'workspacer' segment, so the mock returns it verbatim.
  state.configDir = path.join(sandbox, 'config', 'workspacer');
  // …and the OTHER config home the gate reads directly from the environment
  // rather than through getConfigDir: git's own, `$XDG_CONFIG_HOME/git`. The two
  // Go loaders already point XDG_CONFIG_HOME at ${SANDBOX}/config, so without
  // this the corpus could not express a case about git's per-user config
  // directory that all three copies answer the same way.
  savedXdg = process.env.XDG_CONFIG_HOME;
  process.env.XDG_CONFIG_HOME = path.join(sandbox, 'config');
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
  if (savedXdg === undefined) delete process.env.XDG_CONFIG_HOME;
  else process.env.XDG_CONFIG_HOME = savedXdg;
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
 *  a root written without a token ("/", "", "~", "root") is literal on purpose.
 *
 *  An UNRECOGNISED `${TOKEN}` is passed through verbatim by every substituter in
 *  every loader, and what comes out is then a RELATIVE string, which all three
 *  copies refuse for not being absolute. So a one-character typo — `${CONFI}` for
 *  `${CONFIG}` — turns a deny case into a case that passes while exercising
 *  nothing, silently and in all three languages at once. Applying that to all 64
 *  deny targets left every suite 100% green. Allow cases are immune (a bogus
 *  target fails `resolvesTo`), so this throw is the negative half's only
 *  protection. */
function subst(s: string): string {
  const table = tokenTable();
  // Sorted for determinism only: no token's VALUE can contain another token.
  let out = s;
  for (const name of Object.keys(table).sort()) out = out.split(`\${${name}}`).join(table[name]);
  const residual = /\$\{[^}]*\}?/.exec(out);
  if (residual) {
    throw new Error(
      `unsubstituted token ${residual[0]} in ${JSON.stringify(out)} — the token set is ` +
        "DECLARED in the fixture's `vocabulary.tokens` block and closed by 'the fixture " +
        "vocabulary is closed'; an undeclared one passes through verbatim and silently " +
        'defangs the case',
    );
  }
  return out;
}

/** This loader's substitution table, and the ONE place the token names it
 *  understands are written down. `subst` expands out of it and the vocabulary
 *  suite compares its key set against the fixture's `vocabulary.tokens` in BOTH
 *  directions — which is what makes the declaration binding: legalizing a
 *  mis-spelled token by adding it to the fixture fails in all three loaders,
 *  because not one of them substitutes it. */
function tokenTable(): Record<string, string> {
  return {
    SANDBOX: sandbox,
    ROOT: path.join(sandbox, 'root'),
    OUTSIDE: path.join(sandbox, 'outside'),
    CONFIG: path.join(sandbox, 'config', 'workspacer'),
    HOME: REAL_HOME,
    PROCESS_CWD,
  };
}

/** `denyReason`'s declared range, pinned against `vocabulary.denyReasons` so a
 *  reason can neither be declared without a classifier arm nor classified
 *  without being declared. */
const DENY_REASON_NAMES = ['not-absolute', 'unresolvable', 'outside-roots', 'secret'];

/** Classify a refusal by re-running assertPathAllowed's own three gates in
 *  assertPathAllowed's own order. The guard collapses all of them into one
 *  message (7.5), so the reason has to be recomputed rather than parsed out.
 *
 *  'allowed' is deliberately NOT a declared reason: a deny case that reaches it
 *  fails with the mismatch spelled out. */
function denyReason(target: string, roots: string[]): string {
  let canonical: string;
  try {
    canonical = canonicalizePath(target);
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err);
    // The two pre-syscall refusals canonicalizePath raises by hand.
    if (message === 'path is empty' || message === 'path is not absolute') return 'not-absolute';
    // Anything else came out of the WALK (ENOTDIR, EACCES, the hop limit) — but
    // only if it came out of canonicalizePath at all. A TypeError is the
    // classifier itself being broken (a missing import answered 'unresolvable'
    // for all 63 deny cases while this test was being written), and a classifier
    // that fails soft is exactly the vacuous guard this whole file is closing.
    if (err instanceof TypeError) throw err;
    return 'unresolvable';
  }
  if (!pathWithinRoots(roots, canonical)) return 'outside-roots';
  if (isSecretPath(canonical)) return 'secret';
  return 'allowed';
}

/** Every token reference in a string. `unterminated` reports a '${' with no
 *  closing brace, which the substituter leaves verbatim exactly like a
 *  mis-spelled name does. */
function tokenRefs(s: string): { names: string[]; unterminated: boolean } {
  const names: string[] = [];
  let i = 0;
  for (;;) {
    const j = s.indexOf('${', i);
    if (j < 0) return { names, unterminated: false };
    const end = s.indexOf('}', j + 2);
    if (end < 0) return { names, unterminated: true };
    names.push(s.slice(j + 2, end));
    i = end + 1;
  }
}

/** Visit every string in the decoded fixture — object VALUES and object KEYS,
 *  prose `_comment` blocks included. Comments are in scope on purpose: a
 *  mis-spelling in the prose is how a mis-spelling in a case gets written, and
 *  the fixture's own vocabulary block says so. */
function walkStrings(v: unknown, where: string, visit: (where: string, s: string) => void): void {
  if (typeof v === 'string') {
    visit(where, v);
    return;
  }
  if (Array.isArray(v)) {
    v.forEach((e, i) => walkStrings(e, `${where}[${i}]`, visit));
    return;
  }
  if (v && typeof v === 'object') {
    for (const [k, e] of Object.entries(v as Record<string, unknown>)) {
      visit(`${where}.${k} (key)`, k);
      walkStrings(e, `${where}.${k}`, visit);
    }
  }
}

/** The fixture spells `resolvesTo` with '/' separators; the tokens inside it
 *  substitute to native ones. Only the fixture-authored separators need
 *  translating, and on POSIX this is a no-op. */
function nativeSep(s: string): string {
  return WIN32 ? s.split('/').join(path.sep) : s;
}

/**
 * The guard for the class of defect that made every deny case in this corpus
 * individually unfalsifiable.
 *
 * A one-character typo in a `${TOKEN}` name defangs a deny case in ALL THREE
 * loaders at once and in silence: the name does not substitute, the target
 * becomes a relative literal, every copy refuses it for not being absolute, and
 * the case passes while exercising nothing. Applying that to all 64 deny targets
 * left all three suites green. The sibling defect is a typo in a case's `group`:
 * both Go loaders filter with `if !groups[c.Group] { continue }`, so the case
 * silently stops running there while THIS loader, which does not filter, keeps
 * running it — the three copies quietly stop being held to the same corpus,
 * which is the one thing the fixture exists to prevent.
 *
 * `subst`'s residual check is not enough on its own: it only fires for cases
 * that actually RUN, so a typo in a case this platform skips (needsSymlinks,
 * needsUnreadableDir, needsHome), or in one a `group` typo already dropped from
 * the Go loaders, is never seen there. Everything below is STATIC and holds
 * whether or not a single case executes.
 *
 * TWINS: cmd/brain/fsguard_test.go and internal/bus/policy_test.go both run
 * TestFixtureVocabularyIsClosed. A check only ONE loader runs is how
 * secretBasenames drifted.
 */
describe('the fixture vocabulary is closed', () => {
  const vocab = fixture.vocabulary;

  it('declares tokens, groups and deny reasons at all', () => {
    expect(
      [
        Object.keys(vocab?.tokens ?? {}).length,
        Object.keys(vocab?.groups ?? {}).length,
        Object.keys(vocab?.denyReasons ?? {}).length,
      ].every((n) => n > 0),
      'an empty vocabulary block makes every check below vacuous',
    ).toBe(true);
  });

  it('declares exactly the tokens this loader substitutes', () => {
    // Both directions. A token the fixture declares and this loader cannot
    // expand makes every case using it test a literal; a token this loader
    // expands that the fixture does not declare is drift in the other
    // direction, and is also the obvious way to legalize a typo.
    expect(Object.keys(tokenTable()).sort()).toEqual(Object.keys(vocab.tokens).sort());
  });

  it('uses no undeclared token ANYWHERE in the document, prose included', () => {
    const declared = new Set(Object.keys(vocab.tokens));
    const problems: string[] = [];
    walkStrings(fixture, '', (where, s) => {
      const { names, unterminated } = tokenRefs(s);
      if (unterminated) {
        problems.push(`${where}: a '\${' with no closing '}' in ${JSON.stringify(s)}`);
      }
      for (const name of names) {
        if (!declared.has(name)) {
          problems.push(
            `${where}: '\${${name}}' is not a declared token — it passes through verbatim ` +
              `and silently defangs whatever uses it, in ${JSON.stringify(s)}`,
          );
        }
      }
    });
    expect(problems).toEqual([]);
  });

  it('declares no token that no case actually uses', () => {
    // A token only prose mentions is a token no loader is proved to substitute
    // — and declaring a mis-spelling is the cheapest way to smuggle one in.
    const used = new Set<string>();
    for (const c of fixture.cases) {
      for (const s of [...c.roots, c.target, c.resolvesTo ?? '']) {
        for (const name of tokenRefs(s).names) used.add(name);
      }
    }
    expect([...used].sort()).toEqual(Object.keys(vocab.tokens).sort());
  });

  it('puts every case, and every owner layer, in a declared group', () => {
    const problems: string[] = [];
    const caseCount: Record<string, number> = {};
    for (const c of fixture.cases) {
      if (!(c.group in vocab.groups)) {
        problems.push(
          `case "${c.name}" is in group "${c.group}", which vocabulary.groups does not ` +
            'declare — both Go loaders skip it silently and the three copies stop being ' +
            'held to the same corpus',
        );
        continue;
      }
      caseCount[c.group] = (caseCount[c.group] ?? 0) + 1;
    }
    const ownedGroups = new Set<string>();
    for (const [owner, layers] of Object.entries(fixture.owners)) {
      for (const g of layers) {
        if (!(g in vocab.groups)) {
          problems.push(
            `owner ${owner} claims group "${g}", which vocabulary.groups does not declare`,
          );
        }
        ownedGroups.add(g);
      }
    }
    for (const g of Object.keys(vocab.groups)) {
      if (!caseCount[g]) problems.push(`group "${g}" is declared but no case belongs to it`);
      if (!ownedGroups.has(g)) {
        problems.push(
          `group "${g}" is declared but no owner implements it, so every loader skips its cases`,
        );
      }
    }
    expect(problems).toEqual([]);
  });

  it('makes every deny case name the reason it is denied FOR', () => {
    const problems: string[] = [];
    const reasonCount: Record<string, number> = {};
    for (const c of fixture.cases) {
      if (c.expect === 'deny') {
        if (!c.deniedBy) {
          problems.push(
            `deny case "${c.name}" names no deniedBy — expect: 'deny' alone is satisfied by ` +
              'a refusal for ANY reason, which is exactly how a defanged case keeps passing',
          );
          continue;
        }
        if (!(c.deniedBy in vocab.denyReasons)) {
          problems.push(
            `deny case "${c.name}" claims reason "${c.deniedBy}", which vocabulary.denyReasons does not declare`,
          );
          continue;
        }
        reasonCount[c.deniedBy] = (reasonCount[c.deniedBy] ?? 0) + 1;
      } else if (c.expect === 'allow') {
        if (c.deniedBy) {
          problems.push(
            `allow case "${c.name}" carries deniedBy "${c.deniedBy}"; an allow is pinned by resolvesTo instead`,
          );
        }
      } else {
        problems.push(`case "${c.name}" has expect "${c.expect}", which is neither allow nor deny`);
      }
    }
    for (const r of Object.keys(vocab.denyReasons)) {
      if (!reasonCount[r]) {
        problems.push(
          `deny reason "${r}" is declared but no case names it — an unexercised ` +
            'classification arm is one nothing holds to the other copies',
        );
      }
    }
    expect(problems).toEqual([]);
  });

  it("declares exactly the reasons this loader's classifier can return", () => {
    expect([...DENY_REASON_NAMES].sort()).toEqual(Object.keys(vocab.denyReasons).sort());
  });
});

describe('path containment — cross-language contract', () => {
  // What the sweep below actually EXECUTED, as opposed to what it enumerated.
  const corpusTally = new SweepTally();

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

  // asciiLower is the fold the secret gate runs on every guarded path, and all
  // three copies say in a comment that it is deliberately NOT toLowerCase /
  // strings.ToLower because they have to fold IDENTICALLY. Nothing pinned it:
  // `return s.toLowerCase()` passed the whole corpus and all 1170 desktop tests,
  // because every case-variant CASE uses pure A-Z spellings that both folds
  // agree on. These vectors carry code points where they do not.
  it('folds ASCII only, exactly as the fixture vectors say', () => {
    expect(fixture.asciiFold.cases.length).toBeGreaterThan(0);
    expect(
      fixture.asciiFold.cases.some((v) => [...v.in].some((ch) => ch.codePointAt(0)! > 127)),
      'every asciiFold vector is pure ASCII, so toLowerCase() would pass them all',
    ).toBe(true);
    for (const v of fixture.asciiFold.cases) {
      expect(asciiLower(v.in), `asciiLower(${JSON.stringify(v.in)})`).toBe(v.out);
    }
  });

  it('carves out exactly the config stores the fixture names, in that order', () => {
    // The carve-out is order-insensitive (any store that contains the target
    // wins), but toEqual pins the order too, and the Go twin's
    // configStoreSubdirs is the same literal list in the same order.
    expect(configStoreRoots()).toEqual(
      fixture.configStoreSubdirs.map((s) => path.join(state.configDir, s)),
    );
  });

  // The BEHAVIOURAL half of the test above, and the half that guards the
  // escalation. `configStoreRoots()` returning the right three strings is not
  // the same claim as `isSecretPath` ITERATING them: the gate holds its own
  // loop, so a fourth hardcoded carve-out there re-admits <configDir>/plugins/**
  // — every installed plugin's manifest, cache and state, next door to the
  // .bus-token the basename list covers — with the constant, the fixture and all
  // 106 cases green. Every secrets case names one of the three real stores, so a
  // gate with FOUR carve-outs satisfies all of them.
  it('the GATE exempts exactly those stores, and nothing else in the config dir', () => {
    for (const store of fixture.configStoreSubdirs) {
      fs.mkdirSync(path.join(state.configDir, store), { recursive: true });
      expect(
        isSecretPath(path.join(state.configDir, store, 'item.md')),
        `the fixture carves out ${store} but the gate still refuses it`,
      ).toBe(false);
    }
    for (const name of ['plugins', 'cache', 'logs', 'handoffs', 'backups', 'supervisor']) {
      if (fixture.configStoreSubdirs.includes(name)) continue;
      fs.mkdirSync(path.join(state.configDir, name), { recursive: true });
      expect(
        isSecretPath(path.join(state.configDir, name, 'anything.json')),
        `the gate exempts <configDir>/${name}, which the fixture does not list`,
      ).toBe(true);
    }
    expect(isSecretPath(path.join(state.configDir, 'remote-token'))).toBe(true);
  });

  // The `.git` rule is a PRIMITIVE, and the fixture cases only ever reach it
  // through a full guard verdict. These vectors pin its shape directly: exact
  // component match, folded, final component included, no prefix behaviour. A
  // `startsWith('.git')` spelling passes every deny case in the corpus and blanks
  // .gitignore / .gitattributes / .github out of the UI.
  it('treats .git as a whole path COMPONENT, not a prefix', () => {
    const abs = (p: string): string => path.join(path.sep + 'r', ...p.split('/'));
    for (const p of ['.git', '.git/config', 'proj/.git/config', '.GIT/config', 'a/.Git/b']) {
      expect(traversesGitDir(abs(p)), p).toBe(true);
    }
    for (const p of [
      '.gitignore',
      '.gitattributes',
      '.gitmodules',
      '.github/workflows/ci.yml',
      'git/config',
      'x.git/y',
    ]) {
      expect(traversesGitDir(abs(p)), p).toBe(false);
    }
  });

  for (const c of fixture.cases) {
    const reason = skipReason(c);
    const run = reason ? it.skip : it;
    if (reason) corpusTally.skip(reason);
    run(`[${c.group}] ${c.name}${reason ? ` (skipped: ${reason})` : ''}`, () => {
      // Counted HERE, not at registration: the loop above enumerates the
      // fixture, and an enumerated case that skipped asserted nothing. The
      // floor below is what turns a host with no symlink privilege from a green
      // run with a skip count into a red one.
      corpusTally.ran(c.expect);
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
        // THE RIGHT REASON. A deny that happens for the wrong reason is a case
        // that tests nothing while reporting green — a mangled `${TOKEN}` makes
        // the target a relative literal that every copy refuses for not being
        // absolute, with the case's name still claiming a symlink escape.
        // deniedBy is the fixture's independent statement of which gate must
        // fire, and it is NOT derivable from `group`: 'a symlink out of an
        // allowed root into the config dir' is a secrets case whose target
        // resolves clean out of the only granted root, so containment refuses
        // it before the secret gate is ever consulted.
        expect(denyReason(target, roots), `${c.name}: denied for the wrong reason — ${c.why}`).toBe(
          c.deniedBy,
        );
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

  // Declared last so it runs after every case above. Both classes, separately:
  // an allow-only sweep says the guard lets things through and nothing else,
  // and a deny-only sweep is satisfied by a guard that refuses everything.
  // RATCHETED. A floor of one allow and one deny is met by a 107-case corpus
  // that lost 105 of them — the same "asserted almost nothing while green"
  // failure arriving through a bad merge instead of a bad host. The enumerated
  // count is host-independent, so this number means the same thing on a machine
  // that skips most of the sweep. TWINS: cmd/brain/fsguard_test.go's
  // containmentCorpusFloor and internal/bus's busContainmentCorpusFloor.
  itSweptTheWholeCorpus(corpusTally, 'the desktop containment corpus', 112);
});

describe('the canonical path assertPathAllowed returns', () => {
  // checkUse, the other half of BINDING DECISION 2: the guard's answer is what
  // gets opened, so it has to be the RESOLVED path and not the caller's string.
  // Reported as a skip, never as a pass: `if (!CAN_SYMLINK) return` inside the
  // body would count as green on a Windows box that cannot make symlinks.
  const linkGate = { ran: 0 };
  const itLinks = gatedIt(CAN_SYMLINK, linkGate);

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

  // The two symlink tests are the only executors of checkUse — the corpus above
  // pins the VERDICT, these pin the ANSWER — so a host that cannot make
  // symlinks has to be red rather than green-with-two-skips.
  itRanEveryGatedTest(linkGate, 'the checkUse (canonical-answer) tests', 2);
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

/**
 * resolveStoreEntry's ANSWER, not just its verdict.
 *
 * Its own docstring says "the returned path is the string the caller must open
 * (BINDING DECISION 2)", and its two consumers — sessionService.listSessions and
 * layoutService — readFileSync whatever it hands back. Only the null/non-null
 * half was asserted anywhere: `isWithin(canonical, dir) ? path.join(dir, name) :
 * null` passed 108/108 focused and 84 files / 1213 tests, while a store entry
 * that is a symlink resolving back INSIDE the store got opened through the link
 * rather than at the location the check described — the same check-then-open
 * window the function exists to close, in the one function whose comment names
 * it. (Deleting the containment test entirely IS killed, so only the value half
 * was free.)
 */
describe('resolveStoreEntry returns the path it validated', () => {
  let store: string;
  beforeEach(() => {
    store = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'wks-store-')));
  });
  afterEach(() => fs.rmSync(store, { recursive: true, force: true }));

  // COUNTED, like the checkUse group sixty lines above. These three shipped as
  // `try { fs.symlinkSync(...) } catch { return }`, which is a PASS on a host
  // that cannot make symlinks — Windows without developer mode, a container
  // mount, or WKS_TEST_NO_SYMLINKS=1, the lever built to simulate exactly that.
  // On this host precisely ONE test kills the mutant this block was written for
  // (`resolveStoreEntry` returning the unresolved join), and it sat inside the
  // swallowing catch: under the lever the corpus above skips 30 cases and its
  // floor fires, while these three still reported a green tick. A host that
  // cannot run the only oracle for a fix must be RED.
  const storeGate = { ran: 0 };
  const itLinks = gatedIt(CAN_SYMLINK, storeGate);

  itLinks('resolves an in-store symlink to its target', () => {
    const target = path.join(store, 'target.yaml');
    fs.writeFileSync(target, 'x');
    fs.symlinkSync(target, path.join(store, 'alias.yaml'));
    expect(resolveStoreEntry(store, 'alias.yaml')).toBe(target);
  });

  // Ungated on purpose: it needs no privilege, so it is not part of the group
  // the floor counts. It is also not an oracle for the ANSWER — an unresolved
  // join and a canonical walk agree on a plain file in the store.
  it('returns the canonical path for an ordinary entry', () => {
    fs.writeFileSync(path.join(store, 'a.yaml'), 'x');
    expect(resolveStoreEntry(store, 'a.yaml')).toBe(path.join(store, 'a.yaml'));
  });

  itLinks('still returns null for an entry that leaves the store', () => {
    const outside = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'wks-store-out-')));
    try {
      fs.writeFileSync(path.join(outside, 'loot.yaml'), 'x');
      fs.symlinkSync(path.join(outside, 'loot.yaml'), path.join(store, 'pwn.yaml'));
      expect(resolveStoreEntry(store, 'pwn.yaml')).toBeNull();
    } finally {
      fs.rmSync(outside, { recursive: true, force: true });
    }
  });

  itRanEveryGatedTest(storeGate, 'the resolveStoreEntry canonical-answer tests', 2);
});
