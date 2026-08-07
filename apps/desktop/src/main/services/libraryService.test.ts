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
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';

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
vi.mock('./hubClient', () => ({ publishToHub: () => {} }));

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
