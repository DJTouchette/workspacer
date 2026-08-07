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
  const itemGuard = (root: string) => {
    const roots = [path.join(fs.realpathSync(h.configDir), 'library'), root];
    return (p: string): string | null => {
      try {
        return assertPathAllowed('library.list', p, roots);
      } catch {
        return null;
      }
    };
  };

  const settle = (): Promise<void> => new Promise((r) => setTimeout(r, 400));

  itLinks('does not turn an out-of-root directory into a library.changed oracle', async () => {
    const realCwd = fs.realpathSync(cwd);
    const outside = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'wks-lib-outside-')));
    // An ordinary permitted write inside the allowed root: a symlinked
    // component of the DERIVED path.
    fs.symlinkSync(outside, path.join(realCwd, '.claude'));
    fs.mkdirSync(path.join(outside, 'agents'), { recursive: true });

    libraryService.list(realCwd, itemGuard(realCwd));
    await settle();
    busEvents.length = 0;

    // Stand-in for remote-token: a file in neither the item roots nor the cwd
    // root, one that assertPathAllowed refuses to read.
    fs.writeFileSync(path.join(outside, 'agents', 'remote-token'), 'tok');
    await settle();

    expect(
      busEvents,
      'library.list installed an fs.watch outside every allowed root — a write there reached the bus as library.changed, which is a change oracle on whatever the link points at',
    ).toEqual([]);
    fs.rmSync(outside, { recursive: true, force: true });
  });

  it('still reports changes in the ordinary derived directories inside the root', async () => {
    const realCwd = fs.realpathSync(cwd);
    fs.mkdirSync(path.join(realCwd, '.claude', 'agents'), { recursive: true });

    libraryService.list(realCwd, itemGuard(realCwd));
    await settle();
    busEvents.length = 0;

    fs.writeFileSync(path.join(realCwd, '.claude', 'agents', 'a.md'), 'x');
    await settle();

    // The floor: without this the test above is satisfied by watching nothing.
    expect(busEvents).toContainEqual({ type: 'library.changed' });
  });

  itRanEveryGatedTest(watchGate, 'the derived-watch containment test', 1);
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
