/**
 * loadSession / deleteSession take a caller-supplied filename and
 * are reachable from the hub bus (the sessions.load / sessions.delete caps), so a
 * traversal like "../../.ssh/id_rsa" must be rejected before touching the disk.
 */

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
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

const { sessionService } = await import('./sessionService');

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
  resolvesTo?: string;
  needsSymlinks?: boolean;
  tree?: { dirs?: string[]; files?: Record<string, string>; symlinks?: Record<string, string> };
  why?: string;
}

const sessionFixture: { sessionFilenames: { cases: SessionFilenameCase[] } } = JSON.parse(
  fs.readFileSync(
    path.join(__dirname, '../../../../../contracts/path-containment-cases.json'),
    'utf-8',
  ),
);

/** Windows without developer mode cannot create symlinks; report a skip, never a pass. */
const CAN_SYMLINK_SESSIONS = (() => {
  const probe = fs.mkdtempSync(path.join(os.tmpdir(), 'wks-sessym-'));
  try {
    fs.symlinkSync(probe, path.join(probe, 'l'));
    return true;
  } catch {
    return false;
  } finally {
    fs.rmSync(probe, { recursive: true, force: true });
  }
})();

describe('session filenames — cross-language contract', () => {
  const cases = sessionFixture.sessionFilenames?.cases ?? [];

  it('the fixture block loads', () => {
    expect(cases.length, 'sessionFilenames.cases must not be empty').toBeGreaterThan(0);
  });

  for (const c of cases) {
    const skipped = c.needsSymlinks && !CAN_SYMLINK_SESSIONS;
    const run = skipped ? it.skip : it;
    run(`${c.name}${skipped ? ' (skipped: needsSymlinks)' : ''}`, () => {
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
});

// listSessions derives `<sessionsDir>/<readdir entry>` itself, so it needs the
// same rule resolveWithinSessionsDir applies to a caller-supplied filename: a
// symlink named like a session is a legal entry in a bus-writable directory, and
// its `name:` field would come back in the listing. Twin: cmd/brain/stores.go
// listSavedSessions.
describe('listSessions — derived entries stay inside the sessions dir', () => {
  it('skips an entry that resolves out of the sessions dir', () => {
    try {
      fs.symlinkSync(secretOutside, path.join(sessionsDir, 'pwn.yaml'));
    } catch {
      return; // no symlink privilege here
    }
    expect(sessionService.listSessions().map((s) => s.name)).toEqual(['real']);
  });

  it('still lists a symlink that stays inside the sessions dir', () => {
    try {
      fs.symlinkSync(path.join(sessionsDir, 'real.yaml'), path.join(sessionsDir, 'alias.yaml'));
    } catch {
      return;
    }
    expect(sessionService.listSessions()).toHaveLength(2);
  });
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
  it('refuses an entry that resolves out of the sessions dir, and does not read it', () => {
    fs.writeFileSync(secretOutside, 'name: my-session\n');
    const link = path.join(sessionsDir, 'my-session.yaml');
    try {
      fs.symlinkSync(secretOutside, link);
    } catch {
      return; // no symlink support
    }

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

  it('still saves an ordinary session, and still picks the next free suffix', () => {
    const first = sessionService.saveSession({ name: 'Feature: Auth' } as never);
    const second = sessionService.saveSession({ name: 'Feature Auth' } as never);
    expect(first).toBe('feature-auth.yaml');
    expect(second).toBe('feature-auth-2.yaml');
    // Re-saving the SAME name reuses its file rather than minting another.
    expect(sessionService.saveSession({ name: 'Feature: Auth' } as never)).toBe(first);
  });
});
