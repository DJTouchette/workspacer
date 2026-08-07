/**
 * loadSession / deleteSession take a caller-supplied filename and
 * are reachable from the hub bus (the sessions.load / sessions.delete caps), so a
 * traversal like "../../.ssh/id_rsa" must be rejected before touching the disk.
 */

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import {
  SweepTally,
  itRanEveryGatedTest,
  gatedIt,
  CAN_SYMLINK,
  itSweptTheWholeCorpus,
} from '../../../tests/support/sweepTally';
import * as os from 'os';
import * as path from 'path';
import * as fs from 'fs';

// A per-test temp config dir stands in for ~/.workspacer; sessions live under it.
let configDir: string;
vi.mock('./configService', () => ({
  getConfigDir: () => configDir,
}));
// The client is only imported for getCwd (session enrichment); stub it out.
vi.mock('./claudemonSessionClient', () => ({
  claudemonSessionClient: { getCwd: () => undefined },
}));

const { sessionService, resolveWithinSessionsDir } = await import('./sessionService');

let sessionsDir: string;
let secretOutside: string;

beforeEach(() => {
  configDir = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'wks-cfg-')));
  sessionsDir = path.join(configDir, 'sessions');
  fs.mkdirSync(sessionsDir, { recursive: true });
  fs.writeFileSync(path.join(sessionsDir, 'real.yaml'), 'name: real\ntimestamp: t\n');
  // A file that sits OUTSIDE the sessions dir (sibling of it) — the traversal target.
  secretOutside = path.join(configDir, 'secret.yaml');
  fs.writeFileSync(secretOutside, 'name: secret\n');
});

afterEach(() => {
  fs.rmSync(configDir, { recursive: true, force: true });
});

describe('loadSession — containment', () => {
  it('loads a legitimate session file inside the sessions dir', () => {
    expect(sessionService.loadSession('real.yaml')).toMatchObject({ name: 'real' });
  });

  it('returns null for a missing-but-contained filename (not a hard reject)', () => {
    expect(sessionService.loadSession('nope.yaml')).toBeNull();
  });

  it('rejects a traversal escaping the sessions dir', () => {
    expect(() => sessionService.loadSession('../secret.yaml')).toThrow(
      /escapes the sessions directory/,
    );
    // The out-of-tree file is untouched and unread.
    expect(fs.existsSync(secretOutside)).toBe(true);
  });

  it('rejects an absolute path', () => {
    expect(() => sessionService.loadSession('/etc/passwd')).toThrow(
      /escapes the sessions directory/,
    );
  });
});

describe('saveSession — filename slug collisions', () => {
  it('does not clobber a different session whose name slugs to the same file', () => {
    // 'Feature: Auth' and 'Feature Auth' both slug to feature-auth.yaml.
    const a = { name: 'Feature: Auth', timestamp: '2026-01-01T00:00:00Z' } as any;
    const b = { name: 'Feature Auth', timestamp: '2026-01-02T00:00:00Z' } as any;

    const fileA = sessionService.saveSession(a);
    const fileB = sessionService.saveSession(b);

    // Distinct names must land in distinct files — otherwise B overwrites A.
    expect(fileB).not.toBe(fileA);

    // Both sessions survive and are individually loadable with their own data.
    expect(sessionService.loadSession(fileA)).toMatchObject({ name: 'Feature: Auth' });
    expect(sessionService.loadSession(fileB)).toMatchObject({ name: 'Feature Auth' });

    // The picker lists both (plus the beforeEach 'real' fixture) — no silent loss.
    const names = sessionService
      .listSessions()
      .map((s) => s.name)
      .sort();
    expect(names).toEqual(['Feature Auth', 'Feature: Auth', 'real']);
  });

  it('re-saving the SAME session overwrites in place (stable filename across autosaves)', () => {
    const s1 = { name: 'Feature: Auth', timestamp: '2026-01-01T00:00:00Z' } as any;
    const first = sessionService.saveSession(s1);
    const second = sessionService.saveSession({ ...s1, timestamp: '2026-01-03T00:00:00Z' });

    // An autosave of the same session must reuse its file, not spawn a suffix.
    expect(second).toBe(first);
    expect(sessionService.listSessions().filter((s) => s.name === 'Feature: Auth')).toHaveLength(1);
  });
});

describe('deleteSession — containment', () => {
  it('deletes a legitimate session file', () => {
    sessionService.deleteSession('real.yaml');
    expect(fs.existsSync(path.join(sessionsDir, 'real.yaml'))).toBe(false);
  });

  it('rejects a traversal and leaves the out-of-tree file intact', () => {
    expect(() => sessionService.deleteSession('../secret.yaml')).toThrow(
      /escapes the sessions directory/,
    );
    expect(fs.existsSync(secretOutside)).toBe(true);
  });

  it('is a no-op (no throw) for a missing-but-contained filename', () => {
    expect(() => sessionService.deleteSession('nope.yaml')).not.toThrow();
  });
});

// ---------------------------------------------------------------------------
// contracts/path-containment-cases.json → sessionFilenames
//
// The hand-written cases above cover the two LEXICAL escapes ('../secret.yaml',
// '/etc/passwd') and nothing else, which is how this copy came to disagree with
// its Go twin (cmd/brain stores.go sessionFilePath, the provider that answers
// under the default catalog delegation) without either suite noticing: Go
// required a bare basename, this side accepted any multi-segment name that
// textually sat under the sessions dir, and a directory symlink then made
// loadSession return — and deleteSession unlink — a file outside it. The shared
// block is what keeps the two answering the same question the same way.
// ---------------------------------------------------------------------------

interface SessionFilenameCase {
  name: string;
  filename: string;
  expect: 'accept' | 'refuse';
  /** The RIGHT-REASON half of a refusal, from the fixture's
   *  `vocabulary.sessionRefuseReasons`. `expect: 'refuse'` alone is satisfied by
   *  a refusal for ANY reason — including a resolver that refuses everything —
   *  and it hid something specific: the case named "a multi-segment name that
   *  traverses a symlink out of the sessions dir" is refused by the BASENAME
   *  rule and never reaches the symlink. Naming the reason says which of the two
   *  rules each case exercises, and keeps the single case that exercises the
   *  second one from being deleted by accident. Twin: cmd/brain/stores_test.go. */
  refusedBy?: string;
  resolvesTo?: string;
  needsSymlinks?: boolean;
  tree?: { dirs?: string[]; files?: Record<string, string>; symlinks?: Record<string, string> };
  why?: string;
}

/**
 * The INDEPENDENT oracle for WHY a session filename must be refused, written
 * from the two rules rather than from the implementation: rule 1 is "a plain
 * basename", rule 2 is "canonicalizes inside the sessions dir". It shares no
 * code with resolveWithinSessionsDir, so a copy that collapsed the two rules
 * into one (which is exactly what this side shipped) cannot satisfy it.
 */
function sessionRefusalReason(filename: string): string {
  if (
    filename.trim() === '' ||
    filename === '.' ||
    filename === '..' ||
    path.isAbsolute(filename) ||
    filename !== path.basename(filename)
  ) {
    return 'not-a-basename';
  }
  const dir = path.join(configDir, 'sessions');
  let real: string;
  let realDir: string;
  try {
    real = fs.realpathSync(path.join(dir, filename));
    realDir = fs.realpathSync(dir);
  } catch {
    return 'unresolvable (the fixture declares no such reason — a case that lands here is one nothing can attribute)';
  }
  if (real === realDir || real.startsWith(realDir + path.sep)) {
    return 'inside the sessions dir (so this case is refused by neither rule, and the fixture is wrong to expect a refusal)';
  }
  return 'escapes-sessions-dir';
}

const sessionFixture: { sessionFilenames: { cases: SessionFilenameCase[] } } = JSON.parse(
  fs.readFileSync(
    path.join(__dirname, '../../../../../contracts/path-containment-cases.json'),
    'utf-8',
  ),
);

/** Windows without developer mode cannot create symlinks; report a skip, never a pass. */
const CAN_SYMLINK_SESSIONS = CAN_SYMLINK;

describe('session filenames — cross-language contract', () => {
  const cases = sessionFixture.sessionFilenames?.cases ?? [];
  // What this sweep EXECUTED, as opposed to what the fixture carries. A
  // needsSymlinks case skips itself on a host without the privilege, and
  // `cases.length > 0` below cannot tell the difference.
  const tally = new SweepTally();

  it('the fixture block loads', () => {
    expect(cases.length, 'sessionFilenames.cases must not be empty').toBeGreaterThan(0);
  });

  for (const c of cases) {
    const skipped = c.needsSymlinks && !CAN_SYMLINK_SESSIONS;
    const run = skipped ? it.skip : it;
    if (skipped) tally.skip('needsSymlinks');
    run(`${c.name}${skipped ? ' (skipped: needsSymlinks)' : ''}`, () => {
      // Counted in the BODY: an enumerated case that skipped asserted nothing.
      tally.ran(c.expect);
      // A sandbox of its own: the shared beforeEach configDir is a different
      // shape (no config/workspacer nesting) and these cases place files
      // OUTSIDE the config dir on purpose.
      const sandbox = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'wks-sessfx-')));
      const previousConfigDir = configDir;
      configDir = path.join(sandbox, 'config', 'workspacer');
      try {
        fs.mkdirSync(path.join(configDir, 'sessions'), { recursive: true });
        fs.mkdirSync(path.join(sandbox, 'outside'), { recursive: true });
        const abs = (rel: string): string => path.join(sandbox, ...rel.split('/'));
        for (const d of c.tree?.dirs ?? []) fs.mkdirSync(abs(d), { recursive: true });
        for (const [f, body] of Object.entries(c.tree?.files ?? {})) {
          fs.mkdirSync(path.dirname(abs(f)), { recursive: true });
          fs.writeFileSync(abs(f), body, 'utf-8');
        }
        for (const [link, target] of Object.entries(c.tree?.symlinks ?? {})) {
          fs.mkdirSync(path.dirname(abs(link)), { recursive: true });
          fs.symlinkSync(abs(target), abs(link));
        }

        if (c.expect === 'refuse') {
          // The right reason, from an oracle that shares no code with
          // resolveWithinSessionsDir (rule 2 is answered with realpath).
          expect(sessionRefusalReason(c.filename), `${c.name}: ${c.why ?? ''}`).toBe(c.refusedBy);
          // Both bus-reachable verbs, because a copy that answered "no" and then
          // opened the file anyway would be green on the resolver alone.
          expect(() => sessionService.loadSession(c.filename), c.why).toThrow(
            /escapes the sessions directory/,
          );
          expect(() => sessionService.deleteSession(c.filename), c.why).toThrow(
            /escapes the sessions directory/,
          );
          for (const f of Object.keys(c.tree?.files ?? {})) {
            expect(fs.existsSync(abs(f)), `a refused delete must not remove ${f}`).toBe(true);
          }
          return;
        }

        expect(c.resolvesTo, 'an accept case must pin `resolvesTo`').toBeTruthy();
        const resolved = abs(c.resolvesTo as string);
        // The PATH the resolver returns, which is the string every call site
        // then hands to the filesystem (BINDING DECISION 2) — and the assertion
        // this loader was missing. Content alone cannot see the difference: the
        // one accept case with a symlink resolves to a file whose content is
        // identical, and the case with nothing on disk asserts only toBeNull(),
        // which any wrong-but-nonexistent path also satisfies. The Go twin has
        // always compared the path.
        expect(resolveWithinSessionsDir(c.filename), c.why).toBe(resolved);
        // loadSession returns the parsed file, so the strongest observable of
        // "which path did it open" is the content of the file resolvesTo names.
        if (fs.existsSync(resolved)) {
          expect(sessionService.loadSession(c.filename), c.why).toEqual(
            require('js-yaml').load(fs.readFileSync(resolved, 'utf-8')),
          );
        } else {
          expect(sessionService.loadSession(c.filename), c.why).toBeNull();
        }
        expect(() => sessionService.deleteSession(c.filename)).not.toThrow();
      } finally {
        configDir = previousConfigDir;
        fs.rmSync(sandbox, { recursive: true, force: true });
      }
    });
  }

  // Declared last so it runs after every case above. Accepts and refuses
  // separately: a resolver that refused everything satisfies every refuse case
  // in this block, and one that accepted everything satisfies every accept case.
  // RATCHETED to the block's size: both verdict classes AND the whole corpus.
  // TWIN: cmd/brain/stores_test.go's sessionFilenameFloor.
  itSweptTheWholeCorpus(tally, 'the sessionFilenames corpus', 12);
});

// listSessions derives `<sessionsDir>/<readdir entry>` itself, so it needs the
// same rule resolveWithinSessionsDir applies to a caller-supplied filename: a
// symlink named like a session is a legal entry in a bus-writable directory, and
// its `name:` field would come back in the listing. Twin: cmd/brain/stores.go
// listSavedSessions.
describe('listSessions — derived entries stay inside the sessions dir', () => {
  const listGate = { ran: 0 };
  const itLinks = gatedIt(CAN_SYMLINK_SESSIONS, listGate);
  // (CAN_SYMLINK_SESSIONS ? it : it.skip), not `try { symlink } catch { return }`.
  // The swallowing form reports a PASS on a host that cannot create symlinks —
  // Windows without developer mode, some container and CI mounts — while
  // asserting nothing at all, and these four tests are the ENTIRE oracle for the
  // sessions store's derived-entry and save-leg containment (the fixture's
  // sessionFilenames block only ever covers the caller-supplied `filename`, never
  // a name that came back from readdir). Deleting the containment from
  // listSessions made this file 24 passed / 3 skipped on such a host. The
  // file's own comment on CAN_SYMLINK_SESSIONS already states the rule: a missing
  // privilege is "reported as a skip, never as a pass".
  itLinks('skips an entry that resolves out of the sessions dir', () => {
    fs.symlinkSync(secretOutside, path.join(sessionsDir, 'pwn.yaml'));
    expect(sessionService.listSessions().map((s) => s.name)).toEqual(['real']);
  });

  itLinks('still lists a symlink that stays inside the sessions dir', () => {
    fs.symlinkSync(path.join(sessionsDir, 'real.yaml'), path.join(sessionsDir, 'alias.yaml'));
    expect(sessionService.listSessions()).toHaveLength(2);
  });

  // A config-dir sibling whose NAME starts with the store's — the prefix
  // collision the Go twin covers for both listers and this side did not. The
  // case above plants its victim at <configDir>/secret.yaml, so a containment
  // that drops the separator boundary (`canonical.startsWith(dir)`) passes it.
  itLinks("skips an entry resolving into a sibling whose name starts with the store's", () => {
    const sibling = path.join(configDir, 'sessions-backup');
    fs.mkdirSync(sibling, { recursive: true });
    const loot = path.join(sibling, 'loot.yaml');
    fs.writeFileSync(loot, 'name: LOOT-OUTSIDE-THE-SESSIONS-DIR\nagents: []\n', 'utf-8');
    fs.symlinkSync(loot, path.join(sessionsDir, 'pwn.yaml'));
    expect(sessionService.listSessions().map((s) => s.name)).not.toContain(
      'LOOT-OUTSIDE-THE-SESSIONS-DIR',
    );
  });

  // These three are the ENTIRE oracle for derived-entry containment: the
  // fixture's sessionFilenames block only ever covers a caller-supplied
  // `filename`, never a name that came back from readdir. A host that cannot
  // create symlinks must therefore be RED, not a green run whose skip count
  // nobody reads — deleting the containment from listSessions once left this
  // file at 24 passed / 3 skipped.
  itRanEveryGatedTest(listGate, 'listSessions derived-entry containment', 3);
});

/**
 * saveSession is the THIRD path through the sessions store, and the only one that
 * did not go through resolveWithinSessionsDir — it composed
 * `path.join(getSessionsDir(), filename)` directly. capspec's own record says the
 * filename is "re-checked by the same resolver", and the Go twin honours that
 * literally (stores.go saveSavedSession → sessionFilePath). Two consequences of
 * the bare join, both through an ordinary permitted fs.write into
 * <configDir>/sessions (a configStoreRoot):
 *
 *  1. an out-of-store CONTENT ORACLE — the collision loop readFileSync'd through
 *     the planted symlink, so the returned filename depended on whether that
 *     file's YAML `name` matched the caller's (my-session.yaml vs
 *     my-session-2.yaml). That is a bus-visible read of a file sessions.load
 *     refuses outright.
 *  2. the entry was silently replaced.
 */
describe('saveSession — containment (the write leg of the same resolver)', () => {
  const saveGate = { ran: 0 };
  const itLinks = gatedIt(CAN_SYMLINK_SESSIONS, saveGate);
  itLinks('refuses an entry that resolves out of the sessions dir, and does not read it', () => {
    fs.writeFileSync(secretOutside, 'name: my-session\n');
    const link = path.join(sessionsDir, 'my-session.yaml');
    fs.symlinkSync(secretOutside, link);

    // The control: load refuses the identical entry.
    expect(() => sessionService.loadSession('my-session.yaml')).toThrow(/escapes/);

    expect(() => sessionService.saveSession({ name: 'my-session' } as never)).toThrow(/escapes/);
    // Untouched: still a symlink, and the file it points at is unchanged.
    expect(fs.lstatSync(link).isSymbolicLink()).toBe(true);
    expect(fs.readFileSync(secretOutside, 'utf-8')).toBe('name: my-session\n');

    // The ORACLE, which is the part a "did it write" assertion cannot see: the
    // returned filename must not depend on the bytes of a file outside the store.
    fs.writeFileSync(secretOutside, 'name: something-else\n');
    expect(() => sessionService.saveSession({ name: 'my-session' } as never)).toThrow(/escapes/);
  });

  // The COLLISION-SUFFIX leg. The test above plants its symlink at the BASE
  // filename, so it only ever exercises the first resolveWithinSessionsDir call;
  // the one inside the loop — which produces every path the loop then reads and
  // the atomic write finally opens — was unexercised, and reverting it alone to a
  // bare path.join left the whole suite green while sessions.save read through a
  // symlinked `<base>-2.yaml` and leaked that file's `name` field as the RETURN
  // VALUE.
  itLinks(
    'refuses a collision-suffix entry that resolves out of the store, and does not read it',
    () => {
      // The base name must be OCCUPIED by a real file, or the loop never runs.
      fs.writeFileSync(path.join(sessionsDir, 'my-session.yaml'), 'name: someone-else\n');
      fs.writeFileSync(secretOutside, 'name: LOOT\n');
      fs.symlinkSync(secretOutside, path.join(sessionsDir, 'my-session-2.yaml'));

      expect(() => sessionService.saveSession({ name: 'my-session' } as never)).toThrow(/escapes/);
      // And the oracle: the answer must not move when the out-of-store bytes do.
      fs.writeFileSync(secretOutside, 'name: my-session\n');
      expect(() => sessionService.saveSession({ name: 'my-session' } as never)).toThrow(/escapes/);
      expect(fs.readFileSync(secretOutside, 'utf-8')).toBe('name: my-session\n');
    },
  );

  it('still saves an ordinary session, and still picks the next free suffix', () => {
    const first = sessionService.saveSession({ name: 'Feature: Auth' } as never);
    const second = sessionService.saveSession({ name: 'Feature Auth' } as never);
    expect(first).toBe('feature-auth.yaml');
    expect(second).toBe('feature-auth-2.yaml');
    // Re-saving the SAME name reuses its file rather than minting another.
    expect(sessionService.saveSession({ name: 'Feature: Auth' } as never)).toBe(first);
  });

  // The base-name leg and the collision-suffix leg. Nothing else covers the
  // save path's containment, so an all-skipped group must fail.
  itRanEveryGatedTest(saveGate, 'the saveSession write leg', 2);
});

/**
 * The resolver's ANSWER, and its unverifiable arm.
 *
 * The previous pass pinned what resolveWithinSessionsDir RETURNS (mutating
 * `return canonical!` to `return path.join(dir, filename)` IS killed). Nothing
 * pinned that the two CALL SITES use it: each could independently go back to the
 * join while keeping the resolver call for its verdict, with 84 files / 1213
 * tests green. For a symlinked entry that stays inside the store, `unlinkSync`
 * removes the LINK and leaves the target, while the Go twin's deleteSavedSession
 * does `os.Remove(canonical)` and removes the TARGET — one sessions.delete, two
 * different files destroyed depending on which provider answered.
 */
describe('sessions.load / sessions.delete open the resolver ANSWER', () => {
  const answerGate = { ran: 0 };
  const itLinks = gatedIt(CAN_SYMLINK_SESSIONS, answerGate);
  itLinks('deleteSession removes what the entry RESOLVES to, not the link', () => {
    const target = path.join(sessionsDir, 'target.yaml');
    const link = path.join(sessionsDir, 'alias.yaml');
    fs.writeFileSync(target, 'name: target\nagents: []\n');
    fs.symlinkSync(target, link);

    sessionService.deleteSession('alias.yaml');

    expect(
      fs.existsSync(target),
      'deleteSession unlinked the LINK; the Go twin removes the TARGET, so the same call destroys a different file per provider',
    ).toBe(false);
    expect(fs.lstatSync(link).isSymbolicLink()).toBe(true);
  });

  itLinks('loadSession reads through to the resolved file', () => {
    const target = path.join(sessionsDir, 'target.yaml');
    fs.writeFileSync(target, 'name: RESOLVED\nagents: []\n');
    fs.symlinkSync(target, path.join(sessionsDir, 'alias.yaml'));
    expect(sessionService.loadSession('alias.yaml')?.name).toBe('RESOLVED');
  });

  itRanEveryGatedTest(answerGate, 'the load/delete resolver-answer tests', 2);
});

/**
 * The `unverifiable -> deny` arm. The comment states the posture explicitly
 * ("same posture as the fs.* guard"), the Go twin returns `"", false` on a
 * canonicalize error, and the fs.* corpus pins the equivalent branch with a
 * `needsUnreadableDir` case — but the fixture's `sessionFilenames` block has no
 * entry whose canonicalization FAILS, so the whole catch arm was unexecuted, in a
 * resolver that hands its answer to readFileSync and unlinkSync. Flipping it to
 * fail OPEN was 1213/1213 green.
 */
describe('resolveWithinSessionsDir on an unverifiable entry', () => {
  const unverifiableGate = { ran: 0 };
  const itLinks = gatedIt(CAN_SYMLINK_SESSIONS, unverifiableGate);
  itLinks('denies an ELOOP symlink cycle', () => {
    // a -> b -> a. canonicalizePath's hop counter throws; the catch must refuse.
    fs.symlinkSync(path.join(sessionsDir, 'b.yaml'), path.join(sessionsDir, 'a.yaml'));
    fs.symlinkSync(path.join(sessionsDir, 'a.yaml'), path.join(sessionsDir, 'b.yaml'));
    expect(() => resolveWithinSessionsDir('a.yaml')).toThrow(/escapes/);
    expect(() => sessionService.loadSession('a.yaml')).toThrow(/escapes/);
    expect(() => sessionService.deleteSession('a.yaml')).toThrow(/escapes/);
  });

  it('denies an entry whose parent is a FILE (ENOTDIR)', () => {
    // The sessions dir itself is replaced by a regular file, so every lstat on
    // the way in fails with ENOTDIR rather than ENOENT — the error class the
    // walk must NOT swallow.
    fs.rmSync(sessionsDir, { recursive: true, force: true });
    fs.writeFileSync(sessionsDir, 'not a directory');
    try {
      expect(() => resolveWithinSessionsDir('x.yaml')).toThrow(/escapes/);
    } finally {
      fs.rmSync(sessionsDir, { force: true });
      fs.mkdirSync(sessionsDir, { recursive: true });
    }
  });

  it('still resolves an ordinary entry (the floor)', () => {
    expect(resolveWithinSessionsDir('real.yaml')).toBe(path.join(sessionsDir, 'real.yaml'));
  });

  // The ELOOP case is the only executor of the catch arm; flipping that arm to
  // fail OPEN was once 1213/1213 green, so a host that skips it must be red.
  itRanEveryGatedTest(unverifiableGate, 'the unverifiable-entry (ELOOP) test', 1);
});
