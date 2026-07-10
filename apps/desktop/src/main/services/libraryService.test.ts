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
