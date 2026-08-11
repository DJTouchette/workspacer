/**
 * Regressions in libraryService's handling of Claude-scoped items whose on-disk
 * name is not slug-stable (e.g. a directory with an uppercase or dotted name):
 *
 *  - list() keyed claude items by slug(name), so two distinct on-disk names that
 *    slugify to the same id collided in the Map and one was silently dropped.
 *  - saveClaude/remove rebuilt the target path from slug(id) rather than the
 *    real on-disk name, so an edit wrote a NEW slugified dir (duplicate) and a
 *    delete unlinked a path that didn't exist (no-op).
 */
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { itRanEveryGatedTest, gatedIt, CAN_SYMLINK } from '../../../tests/support/sweepTally';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';

/** Can this process create symlinks? (Windows without developer mode cannot.)
 *  Every test below that plants one is gated on this and COUNTED, because the
 *  form it replaces — `try { fs.symlinkSync(...) } catch { return }` — reports a
 *  PASS while asserting nothing at all. */
const CAN_SYMLINK_LIB = CAN_SYMLINK;

// Seed a real temp configDir at hoist time: the libraryService singleton runs
// seedGlobalIfEmpty() in its constructor at import, before beforeEach, so an
// empty configDir would write the seed into a repo-relative ./library dir.
const h = vi.hoisted(() => {
  const nodeFs = require('fs') as typeof import('fs');
  const nodeOs = require('os') as typeof import('os');
  const nodePath = require('path') as typeof import('path');
  return { configDir: nodeFs.mkdtempSync(nodePath.join(nodeOs.tmpdir(), 'wks-lib-cfg-')) };
});
vi.mock('./configService', () => ({ getConfigDir: () => h.configDir }));
// Recorded, not swallowed: `library.changed` on the bus is the observable the
// derived-watch test below asserts on — a watch outside the roots is only a leak
// because that event reaches a remote caller.
const busEvents = vi.hoisted(() => [] as unknown[]);
vi.mock('./hubClient', () => ({ publishToHub: (e: unknown) => void busEvents.push(e) }));

import { libraryService } from './libraryService';
import { assertPathAllowed } from '../lib/pathConfinement';

let cwd: string;
beforeEach(() => {
  h.configDir = fs.mkdtempSync(path.join(os.tmpdir(), 'wks-lib-cfg-'));
  cwd = fs.mkdtempSync(path.join(os.tmpdir(), 'wks-lib-cwd-'));
});
afterEach(() => {
  fs.rmSync(h.configDir, { recursive: true, force: true });
  fs.rmSync(cwd, { recursive: true, force: true });
});

function writeSkill(dirName: string, name: string, body: string): void {
  const dir = path.join(cwd, '.claude', 'skills', dirName);
  fs.mkdirSync(dir, { recursive: true });
  fs.writeFileSync(
    path.join(dir, 'SKILL.md'),
    `---\nname: ${name}\ndescription: d\n---\n\n${body}\n`,
    'utf-8',
  );
}

describe('libraryService — claude items with slug-colliding on-disk names', () => {
  it('list() keeps both skills whose directory names slugify to the same id', () => {
    writeSkill('My.Skill', 'Dotted', 'one');
    writeSkill('my-skill', 'Dashed', 'two');

    const skills = libraryService
      .list(cwd)
      .filter((it) => it.scope === 'claude' && it.kind === 'skill');
    expect(skills).toHaveLength(2);
    expect(skills.map((s) => s.title).sort()).toEqual(['Dashed', 'Dotted']);
  });
});

describe('libraryService — title fallback trims the shared ASCII-whitespace set', () => {
  // A frontmatter name of a lone U+FEFF (BOM) is NOT blank: `.trim()` strips the
  // BOM and used to drop the title to the id here, while the Go brain
  // (firstNonEmpty) kept it — a different picker row label and byteCompare sort
  // slot depending on DELEGATE_CATALOG_TO_BRAIN. Both now use the shared
  // asciiWhitespace predicate. Reverting this call site to `data.name.trim()`
  // turns this red. TWIN: cmd/brain TestLibraryItemFieldsMatchTheDesktop.
  it('keeps a BOM-only claude name as the title instead of falling back to the id', () => {
    const BOM = '\uFEFF';
    const dir = path.join(cwd, '.claude', 'skills', 'bomskill');
    fs.mkdirSync(dir, { recursive: true });
    fs.writeFileSync(
      path.join(dir, 'SKILL.md'),
      `---\nname: "${BOM}"\ndescription: d\n---\n\nbody\n`,
      'utf-8',
    );
    const item = libraryService
      .list(cwd)
      .find((it) => it.scope === 'claude' && it.kind === 'skill' && it.id === 'bomskill');
    expect(item).toBeDefined();
    expect(item!.title).toBe(BOM);
  });
});

describe('libraryService — saveClaude/remove target the real on-disk path', () => {
  it('editing a skill whose dir name is not slug-stable updates it in place (no duplicate)', () => {
    writeSkill('MySkill', 'MySkill', 'old');
    const item = libraryService
      .list(cwd)
      .find((it) => it.scope === 'claude' && it.kind === 'skill' && it.title === 'MySkill');
    expect(item).toBeDefined();

    libraryService.save({
      scope: 'claude',
      id: item!.id,
      title: 'MySkill',
      kind: 'skill',
      body: 'updated',
      cwd,
    });

    const original = fs.readFileSync(
      path.join(cwd, '.claude', 'skills', 'MySkill', 'SKILL.md'),
      'utf-8',
    );
    expect(original).toContain('updated');
    // No slugified duplicate directory was created.
    expect(fs.existsSync(path.join(cwd, '.claude', 'skills', 'myskill'))).toBe(false);
  });

  it('removing a skill whose dir name is not slug-stable deletes the real directory', () => {
    writeSkill('MySkill', 'MySkill', 'x');
    const item = libraryService
      .list(cwd)
      .find((it) => it.scope === 'claude' && it.kind === 'skill' && it.title === 'MySkill');
    expect(item).toBeDefined();

    libraryService.remove('claude', item!.id, cwd, 'skill');
    expect(fs.existsSync(path.join(cwd, '.claude', 'skills', 'MySkill'))).toBe(false);
  });
});

describe('libraryService — a claude id must be a basename, not a path', () => {
  // The id is used verbatim to preserve non-slug-stable on-disk names (above),
  // which makes it a path injection point on the bus-reachable save/remove.
  it('saveClaude refuses a traversal id instead of writing outside .claude', () => {
    expect(() =>
      libraryService.save({
        scope: 'claude',
        id: '../../config',
        title: 'pwn',
        kind: 'agent',
        body: 'x',
        cwd,
      }),
    ).toThrow(/invalid library item id/);
    expect(fs.existsSync(path.join(cwd, '..', '..', 'config.md'))).toBe(false);
  });

  it('saveClaude refuses an absolute id', () => {
    const target = path.join(cwd, 'escaped.md');
    expect(() =>
      libraryService.save({
        scope: 'claude',
        id: target,
        title: 'pwn',
        kind: 'agent',
        body: 'x',
        cwd,
      }),
    ).toThrow(/invalid library item id/);
    expect(fs.existsSync(target)).toBe(false);
  });

  it('remove refuses a traversal id rather than recursively deleting it', () => {
    const victim = path.join(cwd, 'precious');
    fs.mkdirSync(victim, { recursive: true });
    fs.writeFileSync(path.join(victim, 'file.txt'), 'keep me', 'utf-8');

    // '../../precious' from <cwd>/.claude/skills would land exactly on it.
    expect(() => libraryService.remove('claude', '../../precious', cwd, 'skill')).toThrow(
      /invalid library item id/,
    );
    expect(fs.existsSync(path.join(victim, 'file.txt'))).toBe(true);
  });

  it('still edits a legitimate non-slug-stable basename in place', () => {
    writeSkill('My.Skill', 'My.Skill', 'old');
    libraryService.save({
      scope: 'claude',
      id: 'My.Skill',
      title: 'My.Skill',
      kind: 'skill',
      body: 'updated',
      cwd,
    });
    expect(
      fs.readFileSync(path.join(cwd, '.claude', 'skills', 'My.Skill', 'SKILL.md'), 'utf-8'),
    ).toContain('updated');
  });
});

// ── The DERIVED destination, not just the caller's cwd ──────────────────────
//
// save() composes `<cwd>/.workspacer/library/<slug>.md` and
// `<cwd>/.claude/skills/<id>/SKILL.md` AFTER the bus handler has checked the
// cwd, and handed the raw string to writeFileSync — which follows a symlink. So
// one ordinary permitted fs.write into the (allowed) project overwrote
// <configDir>/config.yaml with caller-controlled body content, and
// `updates.channel` is concatenated into the electron-updater feed URL. fs.write
// of the identical path is refused, and so is library.save in the Go brain
// (library.go saveLibrary guards the derived path) — the two shipping providers
// disagreed about the same call. The guard is the same predicate the fs.*
// handlers use; a refusal fails the call rather than skipping, because save
// returns the path it claims to have written.
describe('libraryService — save confines the path it actually opens', () => {
  const guardFor = (...roots: string[]): ((p: string) => string | null) => {
    return (p) => {
      try {
        return assertPathAllowed('library.save', p, roots);
      } catch {
        return null;
      }
    };
  };

  it('refuses to write through a symlink planted inside the allowed cwd', () => {
    const root = fs.realpathSync(cwd);
    const cfgFile = path.join(fs.realpathSync(h.configDir), 'config.yaml');
    fs.writeFileSync(cfgFile, 'updates:\n  channel: latest\n', 'utf-8');
    const libDir = path.join(root, '.workspacer', 'library');
    fs.mkdirSync(libDir, { recursive: true });
    fs.symlinkSync(cfgFile, path.join(libDir, 'pwn.md'));

    expect(() =>
      libraryService.save(
        {
          scope: 'project',
          id: 'pwn',
          title: 'pwn',
          kind: 'prompt',
          body: 'updates:\n  channel: http://evil.example/feed\n',
          cwd: root,
        },
        guardFor(root),
      ),
    ).toThrow(/outside the allowed workspace/);
    expect(fs.readFileSync(cfgFile, 'utf-8')).toBe('updates:\n  channel: latest\n');
  });

  it('refuses a claude-scope write through a .claude/skills directory symlink', () => {
    const root = fs.realpathSync(cwd);
    const outside = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'wks-lib-out-')));
    fs.mkdirSync(path.join(root, '.claude'), { recursive: true });
    fs.symlinkSync(outside, path.join(root, '.claude', 'skills'));

    expect(() =>
      libraryService.save(
        { scope: 'claude', id: 'evil', title: 'evil', kind: 'skill', body: 'x', cwd: root },
        guardFor(root),
      ),
    ).toThrow(/outside the allowed workspace/);
    expect(fs.existsSync(path.join(outside, 'evil', 'SKILL.md'))).toBe(false);
    fs.rmSync(outside, { recursive: true, force: true });
  });

  it('leaves no directories behind when the destination is refused', () => {
    const root = fs.realpathSync(cwd);
    expect(() =>
      libraryService.save(
        { scope: 'project', id: 'x', title: 'x', kind: 'prompt', body: 'b', cwd: root },
        () => null,
      ),
    ).toThrow(/outside the allowed workspace/);
    expect(fs.existsSync(path.join(root, '.workspacer', 'library'))).toBe(false);
  });

  it('still saves — and reports the CANONICAL path — for an ordinary project item', () => {
    // The regression floor. A guard that refused everything would satisfy the
    // three cases above; this one fails unless legitimate saves still land, and
    // it pins that the returned path is the resolved one (a symlinked but
    // legitimate project dir is the common monorepo case).
    const root = fs.realpathSync(cwd);
    const real = path.join(root, 'real');
    fs.mkdirSync(real, { recursive: true });
    fs.symlinkSync(real, path.join(root, 'link'));

    const item = libraryService.save(
      {
        scope: 'project',
        id: 'notes',
        title: 'Notes',
        kind: 'prompt',
        body: 'hello',
        cwd: path.join(root, 'link'),
      },
      guardFor(root),
    );
    expect(item.path).toBe(path.join(real, '.workspacer', 'library', 'notes.md'));
    expect(fs.readFileSync(item.path, 'utf-8')).toContain('hello');
  });
});

/**
 * The guard legs the file above never reached.
 *
 * libraryService's per-file guard is THREADED through list/save/remove, and only
 * `save` had a test that made the guard refuse — so `readDir`, `claudeItem` and
 * `remove` could all drop it and the whole main suite stayed green.
 * hubCapabilitiesKillSwitch.test.ts mocks './libraryService' and asserts only the
 * guard FUNCTION it hands over, so the service never applies it there either.
 *
 * Everything below drives the REAL service with the REAL production guard.
 */
describe('libraryService — every leg applies the guard it was handed', () => {
  const prodGuard =
    (cap: string, ...roots: string[]) =>
    (p: string): string | null => {
      try {
        return assertPathAllowed(cap, p, roots);
      } catch {
        return null;
      }
    };

  // The two directories a library item may live in, which is what the bus
  // handler passes (libraryItemRoots in the Go twin).
  const itemGuard = (cap: string, root: string) =>
    prodGuard(cap, path.join(fs.realpathSync(h.configDir), 'library'), root);

  const legs = [
    {
      name: 'global store',
      plant: () => path.join(fs.realpathSync(h.configDir), 'library', 'pwn.md'),
      real: () => path.join(fs.realpathSync(h.configDir), 'library', 'ok.md'),
      title: 'GlobalFloor',
    },
    {
      name: 'project store',
      plant: (root: string) => path.join(root, '.workspacer', 'library', 'pwn.md'),
      real: (root: string) => path.join(root, '.workspacer', 'library', 'ok.md'),
      title: 'ProjectFloor',
    },
    {
      name: 'claude skills',
      plant: (root: string) => path.join(root, '.claude', 'skills', 'pwn', 'SKILL.md'),
      real: (root: string) => path.join(root, '.claude', 'skills', 'okskill', 'SKILL.md'),
      title: 'okskill',
    },
    {
      name: 'claude agents',
      plant: (root: string) => path.join(root, '.claude', 'agents', 'pwn.md'),
      real: (root: string) => path.join(root, '.claude', 'agents', 'okagent.md'),
      title: 'okagent',
    },
    {
      name: 'claude commands',
      plant: (root: string) => path.join(root, '.claude', 'commands', 'pwn.md'),
      real: (root: string) => path.join(root, '.claude', 'commands', 'okcmd.md'),
      title: 'okcmd',
    },
  ];

  for (const leg of legs) {
    it(`list() does not read through a symlink planted in the ${leg.name}`, () => {
      const root = fs.realpathSync(cwd);
      const cfg = fs.realpathSync(h.configDir);
      const token = path.join(cfg, 'remote-token');
      fs.writeFileSync(token, 'SUPERSECRET-REMOTE-TOKEN');

      const plant = leg.plant(root);
      fs.mkdirSync(path.dirname(plant), { recursive: true });
      fs.symlinkSync(token, plant);

      const real = leg.real(root);
      fs.mkdirSync(path.dirname(real), { recursive: true });
      fs.writeFileSync(real, `---\ntitle: ${leg.title}\nname: ${leg.title}\n---\n\nbody\n`);

      const items = libraryService.list(root, itemGuard('library.list', root));
      expect(JSON.stringify(items)).not.toContain('SUPERSECRET-REMOTE-TOKEN');
      // The floor for the same leg: a guard that refuses everything must not
      // satisfy the assertion above.
      expect(items.map((i) => i.title)).toContain(leg.title);
    });
  }

  it('remove() unlinks only what the guard returned', () => {
    const root = fs.realpathSync(cwd);
    const outside = fs.mkdtempSync(path.join(os.tmpdir(), 'wks-lib-victim-'));
    fs.mkdirSync(path.join(outside, 'precious'), { recursive: true });
    fs.writeFileSync(path.join(outside, 'precious', 'keep.txt'), 'precious');
    fs.mkdirSync(path.join(root, '.claude'), { recursive: true });
    // An ordinary permitted fs.write inside the allowed project.
    fs.symlinkSync(outside, path.join(root, '.claude', 'skills'));

    libraryService.remove('claude', 'precious', root, 'skill', itemGuard('library.remove', root));
    expect(fs.existsSync(path.join(outside, 'precious', 'keep.txt'))).toBe(true);

    // The floor: a real skill inside the project is still removable.
    const clean = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'wks-lib-clean-')));
    const skill = path.join(clean, '.claude', 'skills', 'keeper');
    fs.mkdirSync(skill, { recursive: true });
    fs.writeFileSync(path.join(skill, 'SKILL.md'), 'x');
    libraryService.remove('claude', 'keeper', clean, 'skill', itemGuard('library.remove', clean));
    expect(fs.existsSync(skill)).toBe(false);
    fs.rmSync(outside, { recursive: true, force: true });
    fs.rmSync(clean, { recursive: true, force: true });
  });

  // list() is READ-ONLY by contract, which is why it is given the BROWSE roots
  // (the whole home tree). ensureProjectWatch's default branch ran
  // fs.mkdirSync(<cwd>/.workspacer/library) after the cwd check and without
  // resolving it — a derived write from the one capability with the widest root
  // set, following a symlinked `.workspacer` out of every allowed root.
  it('list() creates nothing, even for a project that has no library dir', () => {
    const root = fs.realpathSync(cwd);
    libraryService.list(root, itemGuard('library.list', root));
    expect(fs.existsSync(path.join(root, '.workspacer'))).toBe(false);

    // …and does not follow a symlinked `.workspacer` component either.
    const outside = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'wks-lib-out-')));
    fs.symlinkSync(outside, path.join(root, '.workspacer'));
    libraryService.list(root, itemGuard('library.list', root));
    expect(fs.existsSync(path.join(outside, 'library'))).toBe(false);
    fs.rmSync(outside, { recursive: true, force: true });
  });

  // The WRITE leg of the identical defect. The fix above landed on list() only;
  // saveClaude ended with `ensureProjectWatch(cwd, true)` — two arguments, so
  // mayCreate defaulted to true — which mkdir'd `<cwd>/.workspacer/library`, a
  // path derived after the guard and never resolved. saveClaude writes into
  // `.claude/…` and has no reason to touch the project library dir at all; the
  // Go twin (saveLibraryClaude) creates no watch directory.
  it('save({scope:claude}) creates nothing through a symlinked .workspacer', () => {
    const root = fs.realpathSync(cwd);
    const outside = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'wks-lib-save-out-')));
    fs.symlinkSync(outside, path.join(root, '.workspacer'));

    const item = libraryService.save(
      { scope: 'claude', id: 'my-skill', title: 'My Skill', kind: 'skill', body: 'b', cwd: root },
      itemGuard('library.save', root),
    );

    // The floor: the claude item itself is still written, inside the project.
    expect(item.path).toBe(path.join(root, '.claude', 'skills', 'my-skill', 'SKILL.md'));
    expect(fs.existsSync(item.path)).toBe(true);
    expect(fs.existsSync(path.join(outside, 'library'))).toBe(false);
    fs.rmSync(outside, { recursive: true, force: true });
  });

  // The same argument WITHOUT a symlink, which is the half that was free.
  //
  // With `.workspacer` symlinked out of the root the per-file guard refuses the
  // derived directory anyway, so the test above passes whether mayCreate is true
  // or false — reverting `ensureProjectWatch(cwd, true, false, guard)` to
  // `(cwd, true, true, guard)` left libraryService.test.ts 33/33 and the whole
  // main suite green, while the sibling fix on the identical argument in list()
  // IS pinned. Two legs of one fix, opposite coverage.
  //
  // Ordinarily — no symlink — the mkdir lands INSIDE the allowed cwd, passes the
  // guard, and happens: a claude-scope library.save silently creates and watches
  // `<cwd>/.workspacer/library`, where the Go twin (saveLibraryClaude) creates
  // nothing at all. That is a provider-visible on-disk side effect and an extra
  // `library.changed` emitter, decided by which provider answered.
  it('save({scope:claude}) does not create the project library dir at all', () => {
    const root = fs.realpathSync(cwd);
    const item = libraryService.save(
      { scope: 'claude', id: 'my-skill', title: 'My Skill', kind: 'skill', body: 'b', cwd: root },
      itemGuard('library.save', root),
    );
    // The floor: the claude item itself is still written.
    expect(fs.existsSync(item.path)).toBe(true);
    expect(
      fs.existsSync(path.join(root, '.workspacer')),
      'saveClaude writes into .claude/… and has no business mkdir-ing the project library dir — the Go twin creates nothing',
    ).toBe(false);
  });
});

/**
 * The two id gates on the non-claude scopes, which had no injection test at all:
 * `slug(id)` is the ONLY thing confining a project/global write or delete to
 * .workspacer/library/, and the path guard cannot help — `<cwd>/.workspacer/
 * library/../../CLAUDE.md` canonicalizes to `<cwd>/CLAUDE.md`, which is INSIDE
 * the allowed agent cwd. Writing a project's CLAUDE.md is prompt injection into
 * every agent that opens the repo; deleting it is destructive.
 *
 * And the `.`/`..` half of assertPlainBasename, which the existing block cannot
 * see because every id it tries also contains a separator.
 */
describe('libraryService — a caller-supplied id cannot escape its directory', () => {
  const anyGuard = (p: string): string => p;

  it('project/global save slugs the id rather than concatenating it', () => {
    const root = fs.realpathSync(cwd);
    fs.writeFileSync(path.join(root, 'CLAUDE.md'), 'ORIGINAL PROJECT INSTRUCTIONS\n');
    const item = libraryService.save(
      {
        scope: 'project',
        id: '../../CLAUDE',
        title: 'x',
        kind: 'prompt',
        body: 'OWNED',
        cwd: root,
      },
      anyGuard,
    );
    expect(item.path.startsWith(path.join(root, '.workspacer', 'library'))).toBe(true);
    expect(fs.readFileSync(path.join(root, 'CLAUDE.md'), 'utf-8')).toBe(
      'ORIGINAL PROJECT INSTRUCTIONS\n',
    );
  });

  it('project/global remove slugs the id rather than concatenating it', () => {
    const root = fs.realpathSync(cwd);
    fs.writeFileSync(path.join(root, 'CLAUDE.md'), 'ORIGINAL PROJECT INSTRUCTIONS\n');
    libraryService.remove('project', '../../CLAUDE', root, undefined, anyGuard);
    expect(fs.existsSync(path.join(root, 'CLAUDE.md'))).toBe(true);
  });

  // The CREATE leg — the only one a caller can reach without already knowing an
  // existing item id, and the one every test in this block skipped by always
  // supplying `id`. On both legs the fallback is the caller's TITLE, and on both
  // legs slug() is then the whole defence: the path guard is structurally
  // incapable of helping, because `<cwd>/.workspacer/library/../../CLAUDE.md`
  // canonicalizes to `<cwd>/CLAUDE.md`, which is INSIDE the allowed agent cwd.
  it('slugs the TITLE too when no id is supplied (project scope)', () => {
    const root = fs.realpathSync(cwd);
    fs.writeFileSync(path.join(root, 'CLAUDE.md'), 'ORIGINAL PROJECT INSTRUCTIONS\n');
    const item = libraryService.save(
      { scope: 'project', title: '../../CLAUDE', kind: 'prompt', body: 'OWNED', cwd: root },
      anyGuard,
    );
    expect(item.path.startsWith(path.join(root, '.workspacer', 'library'))).toBe(true);
    expect(fs.readFileSync(path.join(root, 'CLAUDE.md'), 'utf-8')).toBe(
      'ORIGINAL PROJECT INSTRUCTIONS\n',
    );
  });

  it('slugs the TITLE too when no id is supplied (claude scope)', () => {
    const root = fs.realpathSync(cwd);
    fs.writeFileSync(path.join(root, 'CLAUDE.md'), 'ORIGINAL PROJECT INSTRUCTIONS\n');
    const item = libraryService.save(
      { scope: 'claude', title: '../../CLAUDE', kind: 'agent', body: 'OWNED', cwd: root },
      anyGuard,
    );
    expect(item.path.startsWith(path.join(root, '.claude'))).toBe(true);
    expect(fs.readFileSync(path.join(root, 'CLAUDE.md'), 'utf-8')).toBe(
      'ORIGINAL PROJECT INSTRUCTIONS\n',
    );
  });

  it("a claude id of '..' does not rm -rf <cwd>/.claude", () => {
    const root = fs.realpathSync(cwd);
    const settings = path.join(root, '.claude', 'settings.json');
    fs.mkdirSync(path.join(root, '.claude', 'skills'), { recursive: true });
    fs.writeFileSync(settings, '{"hooks":{}}');
    // `<cwd>/.claude/skills/..` = `<cwd>/.claude`, which is INSIDE the allowed
    // cwd — so assertPathAllowed allows it and this string check is the only
    // thing between a bus caller and a recursive force rmSync of settings.json,
    // hooks, agents and commands.
    expect(() => libraryService.remove('claude', '..', root, 'skill', anyGuard)).toThrow(
      /invalid library item id/,
    );
    expect(fs.existsSync(settings)).toBe(true);
    expect(() => libraryService.remove('claude', '.', root, 'skill', anyGuard)).toThrow(
      /invalid library item id/,
    );
    expect(fs.existsSync(settings)).toBe(true);
    expect(() =>
      libraryService.save(
        { scope: 'claude', id: '..', title: 'x', kind: 'skill', body: 'y', cwd: root },
        anyGuard,
      ),
    ).toThrow(/invalid library item id/);
  });

  // The BACKSLASH half of the separator check, which nothing reached: every id
  // the block above tries either carries a forward slash or is '.'/'..', so
  // narrowing the regex to /\// alone was green. The clause is justified in the
  // source ("a backslash is only a separator on win32, but a Windows-shaped id
  // is never a legitimate item name on any platform") and it guards a path that
  // ends in a recursive force rmSync — on Windows, `..\..` IS a traversal.
  it('a Windows-shaped id is refused on every platform', () => {
    const root = fs.realpathSync(cwd);
    for (const id of ['..\\..', 'a\\b', '\\\\server\\share', 'sub\\SKILL']) {
      expect(() => libraryService.remove('claude', id, root, 'skill', anyGuard)).toThrow(
        /invalid library item id/,
      );
      expect(() =>
        libraryService.save(
          { scope: 'claude', id, title: 'x', kind: 'skill', body: 'y', cwd: root },
          anyGuard,
        ),
      ).toThrow(/invalid library item id/);
    }
    // The floor: an ordinary non-slug-stable claude id still works, which is the
    // whole reason these ids are not slugged.
    expect(() =>
      libraryService.save(
        { scope: 'claude', id: 'My.Skill', title: 'x', kind: 'skill', body: 'y', cwd: root },
        anyGuard,
      ),
    ).not.toThrow();
  });
});

// The ORDER a list comes back in is part of the same bus answer. Go sorts
// `out[i].Title < out[j].Title` (raw bytes) and this side used localeCompare, so
// library.list came back in a stably different order depending on which provider
// ran — and the order is what the picker shows and what "first" means in it.
// Fixture: contracts/provider-parity-cases.json. Twin:
// TestListersUseTheFixtureOrdering in the Go brain.
describe('libraryService — list ordering matches the Go provider', () => {
  it('sorts titles byte-wise, not by locale', () => {
    const fixture = JSON.parse(
      fs.readFileSync(
        path.resolve(__dirname, '../../../../../contracts/provider-parity-cases.json'),
        'utf-8',
      ),
    ) as { order: { input: string[]; expected: string[] }[] };
    const c = fixture.order[0];

    const dir = path.join(fs.realpathSync(h.configDir), 'library');
    fs.mkdirSync(dir, { recursive: true });
    c.input.forEach((title, i) => {
      fs.writeFileSync(
        path.join(dir, `${String.fromCharCode(97 + i)}.md`),
        `---\ntitle: ${title}\n---\n\nx\n`,
      );
    });

    expect(libraryService.list().map((i) => i.title)).toEqual(c.expected);
  });
});

// library.list is READ-ONLY by contract, which is why it is handed the WIDEST
// root set — and the fs.watch it installs is neither a read nor a write, so the
// per-file guard the previous pass threaded through readDir/claudeItem never
// reached it. Every one of the four watched directories is DERIVED from the
// caller's cwd after the cwd check, and fs.watch follows symlinks.
//
// The result was a bus-visible change ORACLE on any directory on the host: every
// write to the symlink's target publishes {type:'library.changed'}, so aiming
// `<cwd>/.claude/agents` at ~/.config/workspacer tells a remote caller exactly
// when remote-token, tokens.json and config.yaml are written. The skills watch
// is recursive, so on macOS one link covers a whole subtree.
describe('libraryService — the derived watch paths go through the same guard', () => {
  // `try { symlink } catch { return }` used to stand in for a skip here: it
  // REPORTS A PASS on a host with no symlink privilege while asserting nothing.
  const watchGate = { ran: 0 };
  const itLinks = gatedIt(CAN_SYMLINK_LIB, watchGate);
  const itemGuard = (root: string, ...extra: string[]) => {
    const roots = [path.join(fs.realpathSync(h.configDir), 'library'), root, ...extra];
    return (p: string): string | null => {
      try {
        return assertPathAllowed('library.list', p, roots);
      } catch {
        return null;
      }
    };
  };

  const settle = (): Promise<void> => new Promise((r) => setTimeout(r, 400));

  // ALL FOUR legs, not just the agents one.
  //
  // ensureProjectWatch installs four watches — projectDir, claudeSkillsDir
  // (recursive), claudeAgentsDir, claudeCommandsDir — and this test used to
  // create `outside/agents` and nothing else, so THREE of the four `guard`
  // arguments could be deleted with 88 files / 1379 tests green: the legs were
  // never reached, because the directories they watch did not exist. The skills
  // one is the recursive watch, i.e. a change oracle over a whole subtree.
  itLinks('does not turn an out-of-root directory into a library.changed oracle', async () => {
    const realCwd = fs.realpathSync(cwd);
    const outside = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'wks-lib-outside-')));
    const outsideProj = fs.realpathSync(
      fs.mkdtempSync(path.join(os.tmpdir(), 'wks-lib-outsidep-')),
    );
    // Ordinary permitted writes inside the allowed root: symlinked components of
    // the DERIVED paths. Both are shapes a git clone carries verbatim.
    fs.symlinkSync(outside, path.join(realCwd, '.claude'));
    fs.symlinkSync(outsideProj, path.join(realCwd, '.workspacer'));
    for (const d of ['agents', 'skills', 'skills/deep', 'commands']) {
      fs.mkdirSync(path.join(outside, ...d.split('/')), { recursive: true });
    }
    fs.mkdirSync(path.join(outsideProj, 'library'), { recursive: true });

    libraryService.list(realCwd, itemGuard(realCwd));
    await settle();

    // Stand-in for remote-token: a file in neither the item roots nor the cwd
    // root, one that assertPathAllowed refuses to read. One probe per leg, so a
    // failure names the leg that leaked.
    for (const [leg, target] of [
      ['claudeAgentsDir', path.join(outside, 'agents', 'remote-token')],
      ['claudeSkillsDir (recursive)', path.join(outside, 'skills', 'deep', 'remote-token')],
      ['claudeCommandsDir', path.join(outside, 'commands', 'remote-token')],
      ['projectDir', path.join(outsideProj, 'library', 'remote-token')],
    ] as const) {
      busEvents.length = 0;
      fs.writeFileSync(target, 'tok');
      await settle();
      expect(
        busEvents,
        `the ${leg} watch was installed outside every allowed root — a write there reached the bus as library.changed, which is a change oracle on whatever the link points at`,
      ).toEqual([]);
    }
    fs.rmSync(outside, { recursive: true, force: true });
    fs.rmSync(outsideProj, { recursive: true, force: true });
  });

  it('still reports changes in the ordinary derived directories inside the root', async () => {
    const realCwd = fs.realpathSync(cwd);
    for (const d of [
      '.claude/agents',
      '.claude/skills/deep',
      '.claude/commands',
      '.workspacer/library',
    ]) {
      fs.mkdirSync(path.join(realCwd, ...d.split('/')), { recursive: true });
    }

    libraryService.list(realCwd, itemGuard(realCwd));
    await settle();

    // The floor, per leg: without this the test above is satisfied by watching
    // nothing at all, which is how three of the four guards were free.
    for (const rel of [
      '.claude/agents/a.md',
      '.claude/skills/deep/SKILL.md',
      '.claude/commands/c.md',
      '.workspacer/library/p.md',
    ]) {
      busEvents.length = 0;
      fs.writeFileSync(path.join(realCwd, ...rel.split('/')), 'x');
      await settle();
      expect(busEvents, `no library.changed for an ordinary write at ${rel}`).toContainEqual({
        type: 'library.changed',
      });
    }
  });

  // THE MAP KEY, and the de-duplication that depends on it.
  //
  // `watch()` stores the watcher under the DERIVED name (`dir`), not under the
  // RESOLVED one, and the source says why: the teardown loop recomputes the same
  // derived names for the PREVIOUS cwd, so a resolved key would never be found
  // there. Both halves killed zero tests. Keyed by `resolved`,
  // `this.watchers.has(dir)` is false forever — so every library.list on the
  // same cwd opens ANOTHER fs.watch on the same four directories (unbounded fd
  // growth in a long-lived Electron main process), and the previous project's
  // directories keep publishing `library.changed` to the bus after the user
  // switches projects. That is the same remote-visible activity oracle the
  // derived-watch guard closed, reached through a stale watcher instead of a
  // symlink.
  // BINDING DECISION 2 at this sink: watch() must hand fs.watch/mkdir the string
  // the guard RESOLVED, not the caller-derived one it was asked about. Making it
  // `if (guard(dir) === null) return; const resolved = dir;` — keep the verdict,
  // discard the answer — left the whole desktop suite green, because every other
  // test here uses a guard whose answer is the same inode as its question. This
  // one hands back a DIFFERENT directory, which is the only way to tell the two
  // strings apart from outside.
  itLinks('every watch leg watches the directory the guard ANSWERED', async () => {
    const realCwd = fs.realpathSync(cwd);
    const answers = new Map<string, string>();
    for (const derived of [
      path.join(realCwd, '.workspacer', 'library'),
      path.join(realCwd, '.claude', 'skills'),
      path.join(realCwd, '.claude', 'agents'),
      path.join(realCwd, '.claude', 'commands'),
    ]) {
      const real = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'wks-lib-ans-')));
      answers.set(derived, real);
      fs.mkdirSync(derived, { recursive: true });
    }
    const relocating = (p: string): string | null => answers.get(p) ?? itemGuard(realCwd)(p);

    libraryService.list(realCwd, relocating);
    await settle();

    for (const [derived, answered] of answers) {
      busEvents.length = 0;
      fs.writeFileSync(path.join(answered, 'a.md'), 'x');
      await settle();
      expect(
        busEvents,
        `watch() ignored the guard's answer for ${derived}: a change in ${answered}, the path the guard RESOLVED to, produced no library.changed`,
      ).toContainEqual({ type: 'library.changed' });

      busEvents.length = 0;
      fs.writeFileSync(path.join(derived, 'b.md'), 'x');
      await settle();
      expect(
        busEvents,
        `watch() opened the caller-derived ${derived} instead of the guard's answer — the checked path and the opened path are two different strings again`,
      ).toEqual([]);
    }
    for (const answered of answers.values()) fs.rmSync(answered, { recursive: true, force: true });
  });

  // THE MAP KEY, and the de-duplication that depends on it.
  //
  // watch() stores the watcher under the DERIVED name, not the RESOLVED one, and
  // the source says why: the teardown loop recomputes the same derived names for
  // the PREVIOUS cwd, so a resolved key would never be found there. Both halves
  // killed zero tests. Keyed by `resolved`, `this.watchers.has(dir)` is false
  // forever — so every library.list on the same cwd opens ANOTHER fs.watch on
  // the same four directories, the map only ever holds the last one, and the
  // teardown closes one of five. The previous project's directories then keep
  // publishing `library.changed` to the bus after the user switches projects:
  // the same remote-visible activity oracle the derived-watch guard closed,
  // reached through a leaked watcher instead of a symlink.
  itLinks('switching project closes EVERY watcher the previous project opened', async () => {
    const realCwd = fs.realpathSync(cwd);
    // A symlinked component, so the derived name and the resolved name differ —
    // without that a resolved key coincides with the derived one and the
    // mutation is invisible. The store is an allowed root, so the guard answers.
    const store = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'wks-lib-store-')));
    fs.mkdirSync(path.join(store, 'agents'), { recursive: true });
    fs.symlinkSync(store, path.join(realCwd, '.claude'));
    const other = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'wks-lib-other-')));

    // list() alone cannot grow the set — ensureProjectWatch returns early when
    // the cwd has not changed. `library.save` scope=claude passes force=true and
    // is therefore the path that reaches watch() again and again, which is what
    // makes watch()'s own de-duplication load-bearing rather than redundant.
    const guard = itemGuard(realCwd, store);
    libraryService.list(realCwd, guard);
    for (let i = 0; i < 5; i++) {
      libraryService.save(
        { scope: 'claude', id: `dup${i}`, title: `T${i}`, kind: 'agent', body: 'b', cwd: realCwd },
        guard,
      );
    }
    await settle();
    // The floor: the watch really is live before the switch.
    busEvents.length = 0;
    fs.writeFileSync(path.join(store, 'agents', 'live.md'), 'x');
    await settle();
    expect(busEvents).toContainEqual({ type: 'library.changed' });

    libraryService.list(other, itemGuard(other));
    await settle();
    busEvents.length = 0;
    fs.writeFileSync(path.join(store, 'agents', 'stale.md'), 'x');
    await settle();
    expect(
      busEvents,
      "the previous project's directories are still publishing library.changed after the switch — either the teardown loop cannot find the watcher (a resolved map key) or list() opened more of them than the map holds (no de-duplication)",
    ).toEqual([]);
    fs.rmSync(store, { recursive: true, force: true });
    fs.rmSync(other, { recursive: true, force: true });
  });

  itRanEveryGatedTest(watchGate, 'the derived-watch containment test', 3);
});

/**
 * The guard's ANSWER at every leg, not just its verdict.
 *
 * `guardLibraryFile` returns the canonical path (BINDING DECISION 2), and the
 * killswitch tests pin that the guard REFUSES an out-of-roots path — but nothing
 * pinned that the string it returned is the string that gets opened, written or
 * unlinked. Making the guard `assertPathAllowed(...); return filePath;`, or any
 * one of the six consumer legs re-open the raw join, left 84 files / 1213 tests
 * green.
 *
 * The divergence is concrete on the remove leg: `fs.rmSync(rawTarget)` removes
 * only the SYMLINK `<cwd>/.claude/skills/x`, while cmd/brain/library.go
 * removeLibrary does `os.RemoveAll(canonical)` and destroys the directory it
 * resolves to — same library.remove, two different outcomes depending on which
 * provider answered. Every other leg is the same check-then-open window on a
 * path a bus caller can plant symlinks in.
 */
describe('libraryService — every leg opens the path the guard RESOLVED', () => {
  const resolvedGate = { ran: 0 };
  const itLinks = gatedIt(CAN_SYMLINK_LIB, resolvedGate);
  const itemGuard = (root: string) => {
    const roots = [path.join(fs.realpathSync(h.configDir), 'library'), root];
    return (p: string): string | null => {
      try {
        return assertPathAllowed('library', p, roots);
      } catch {
        return null;
      }
    };
  };

  itLinks('remove deletes what the entry resolves to (parity with the brain)', () => {
    const root = fs.realpathSync(cwd);
    const real = path.join(root, 'real-skill-dir');
    fs.mkdirSync(real, { recursive: true });
    fs.writeFileSync(path.join(real, 'SKILL.md'), '---\nname: x\n---\n\nb\n');
    fs.mkdirSync(path.join(root, '.claude', 'skills'), { recursive: true });
    const link = path.join(root, '.claude', 'skills', 'aliased');
    fs.symlinkSync(real, link);

    libraryService.remove('claude', 'aliased', root, 'skill', itemGuard(root));

    expect(
      fs.existsSync(real),
      'remove unlinked the LINK; removeLibrary in the brain RemoveAll s the resolved directory, so the same call destroys a different tree per provider',
    ).toBe(false);
  });

  itLinks('save writes through an in-root symlink rather than replacing it', () => {
    const root = fs.realpathSync(cwd);
    const dir = path.join(root, '.workspacer', 'library');
    fs.mkdirSync(dir, { recursive: true });
    const target = path.join(root, 'target.md');
    fs.writeFileSync(target, 'old');
    const link = path.join(dir, 'note.md');
    fs.symlinkSync(target, link);

    libraryService.save(
      { scope: 'project', id: 'note', title: 'Note', kind: 'prompt', body: 'NEW', cwd: root },
      itemGuard(root),
    );

    expect(fs.lstatSync(link).isSymbolicLink(), 'the write replaced the link with a file').toBe(
      true,
    );
    expect(fs.readFileSync(target, 'utf-8')).toContain('NEW');
  });

  itLinks('list reads through an in-root symlink to the resolved file', () => {
    const root = fs.realpathSync(cwd);
    const dir = path.join(root, '.workspacer', 'library');
    fs.mkdirSync(dir, { recursive: true });
    const target = path.join(root, 'target.md');
    fs.writeFileSync(target, '---\ntitle: RESOLVED\nkind: prompt\n---\n\nbody\n');
    fs.symlinkSync(target, path.join(dir, 'alias.md'));
    const items = libraryService.list(root, itemGuard(root));
    const item = items.find((i) => i.title === 'RESOLVED');
    expect(item).toBeDefined();
    // The `path` a list item carries IS the guard's answer, and it goes on the
    // wire — so this is the observable that separates "opened the canonical
    // path" from "re-opened the raw join". (For the bytes alone the two are
    // equivalent: readFileSync follows the link either way. The difference is
    // the string the caller is handed, and the check-then-open window.)
    expect(item!.path).toBe(target);
  });

  itLinks('a claude item reports the resolved path too', () => {
    const root = fs.realpathSync(cwd);
    const skills = path.join(root, '.claude', 'skills', 'aliased');
    fs.mkdirSync(skills, { recursive: true });
    const target = path.join(root, 'realskill.md');
    fs.writeFileSync(target, '---\nname: aliased\ndescription: d\n---\n\nbody\n');
    fs.symlinkSync(target, path.join(skills, 'SKILL.md'));
    const item = libraryService.list(root, itemGuard(root)).find((i) => i.id === 'aliased');
    expect(item).toBeDefined();
    expect(item!.path).toBe(target);
  });

  // THE COUNTER WAS NEVER READ. This group built a GateCounter, incremented it
  // four times, and asserted nothing about it — so on a host without symlink
  // privilege all four of the tests that are the ENTIRE oracle for "the leg
  // opens the path the guard resolved" turned into `it.skip`, and the file
  // reported green. A counter with no assertion is not a floor, it is a comment
  // that costs a line; the whole reason gatedIt returns a wrapper is that
  // itRanEveryGatedTest reads what it wrote.
  itRanEveryGatedTest(resolvedGate, 'the every-leg-opens-the-resolved-path tests', 4);
});

// ─── the library writer must not destroy an existing item ────────────────────
// Its Go twin (services/hub/cmd/brain/library.go) has used writeFileAtomic
// since it was written, and every other file-backed store on this side
// (layoutService, sessionService, configService) uses atomicWriteFileSync.
// libraryService is the one that drifted: a bare fs.writeFileSync opens the
// EXISTING target with O_TRUNC, so a write that dies partway (ENOSPC/EDQUOT in
// the field) leaves the user's saved prompt truncated with the original bytes
// already gone. An atomic write goes to a temp file in the same directory and
// RENAMES over the target — observable here as a new inode.

describe('libraryService writes atomically, like its Go twin', () => {
  it('save() replaces the item by rename (new inode), never in place', () => {
    const first = libraryService.save({
      scope: 'global',
      title: 'Atomic',
      kind: 'prompt',
      body: 'one',
      cwd,
    });
    const file = path.join(h.configDir, 'library', `${first.id}.md`);
    const before = fs.statSync(file).ino;

    libraryService.save({
      scope: 'global',
      id: first.id,
      title: 'Atomic',
      kind: 'prompt',
      body: 'two',
      cwd,
    });

    expect(fs.readFileSync(file, 'utf-8')).toContain('two');
    expect(
      fs.statSync(file).ino,
      'the item was rewritten IN PLACE — a write that dies partway destroys the original',
    ).not.toBe(before);
    expect(fs.readdirSync(path.dirname(file)).filter((f) => f.includes('.tmp-'))).toEqual([]);
  });

  it("saveClaude() replaces the user's own .claude file by rename too", () => {
    writeSkill('MySkill', 'MySkill', 'one');
    const item = libraryService
      .list(cwd)
      .find((it) => it.scope === 'claude' && it.kind === 'skill' && it.title === 'MySkill')!;
    const file = path.join(cwd, '.claude', 'skills', 'MySkill', 'SKILL.md');
    const before = fs.statSync(file).ino;

    libraryService.save({
      scope: 'claude',
      id: item.id,
      title: 'MySkill',
      kind: 'skill',
      body: 'two',
      cwd,
    });

    expect(fs.readFileSync(file, 'utf-8')).toContain('two');
    expect(
      fs.statSync(file).ino,
      'a partial write would truncate a file Workspacer did not author',
    ).not.toBe(before);
  });
});
