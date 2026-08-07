/**
 * Regression test: a layout must be deletable even when its name slugs to the
 * 64-char limit on a dash boundary. slugLayout trimmed dashes BEFORE truncating
 * to 64, so the cut could leave a trailing '-'. save() wrote `${id}.yaml` with
 * that id, but remove() re-slugged the id (stripping the trailing dash) and
 * unlinked a different filename — so the layout could never be removed.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';

const h = vi.hoisted(() => ({ dir: '' }));
vi.mock('./configService', () => ({ getConfigDir: () => h.dir }));

import { layoutService } from './layoutService';

beforeEach(() => {
  h.dir = fs.mkdtempSync(path.join(os.tmpdir(), 'wks-layout-'));
});
afterEach(() => {
  fs.rmSync(h.dir, { recursive: true, force: true });
});

describe('layoutService — save/remove round trip', () => {
  it('removes a layout whose slug is truncated on a dash boundary', () => {
    // 63 'a's + ' bbbbb' → 'aaa…a-bbbbb' → cut to 64 lands the 64th char on '-'.
    const name = 'a'.repeat(63) + ' bbbbb';
    const layout = layoutService.save({ name, agents: [] });
    expect(layoutService.list().map((l) => l.id)).toContain(layout.id);

    layoutService.remove(layout.id);
    expect(layoutService.list()).toHaveLength(0);
  });

  it('removes an ordinary layout', () => {
    const layout = layoutService.save({ name: 'My Layout', agents: [] });
    layoutService.remove(layout.id);
    expect(layoutService.list()).toHaveLength(0);
  });
});

describe('layoutService — save contains a caller-supplied id (layouts.save is bus-reachable)', () => {
  // The rule has to be the Go brain's rule, not merely "closed against the
  // escape": the brain answers layouts.save by default, so if TS silently slugged
  // `../config` to `config` while Go errored, the same bus call would return
  // success+rename or a failure depending on a delegation flag — and the success
  // would have overwritten an unrelated layout named `config`.
  it('rejects a traversal id instead of writing (or renaming) anything', () => {
    expect(() => layoutService.save({ id: '../config', name: 'pwn', agents: [] })).toThrow(
      /path separator/,
    );
    expect(fs.existsSync(path.join(h.dir, 'config.yaml'))).toBe(false);
    expect(layoutService.list()).toHaveLength(0);
  });

  it('rejects an absolute-path id', () => {
    const target = path.join(h.dir, 'stolen.yaml');
    expect(() => layoutService.save({ id: target, name: 'pwn', agents: [] })).toThrow(
      /path separator/,
    );
    expect(fs.existsSync(target)).toBe(false);
  });

  it('remove ignores a traversal id rather than unlinking the slugged neighbour', () => {
    const victim = layoutService.save({ name: 'config', agents: [] });
    layoutService.remove('../config');
    expect(layoutService.list().map((l) => l.id)).toContain(victim.id);
  });

  it('a non-slug id still round-trips through remove (save/remove agree)', () => {
    const layout = layoutService.save({ id: 'My Layout!', name: 'pwn', agents: [] });
    expect(layoutService.list().map((l) => l.id)).toContain(layout.id);
    layoutService.remove('My Layout!');
    expect(layoutService.list()).toHaveLength(0);
  });
});

// A lister's own `<layoutsDir>/<readdir entry>` is a DERIVED path, and the
// layouts dir is one of the three the bus lets a caller write into. The entry
// name is a bare basename so nothing escapes textually — a symlink named like a
// layout is what escapes, and readFileSync follows it. Twin:
// cmd/brain/stores.go storeEntryPath (where the same gap additionally COPIED the
// bytes to a `.broken-*` sibling that fs.read then handed back).
describe('layoutService.list — derived entries stay inside the store', () => {
  it('skips an entry that resolves out of the layouts dir', () => {
    const layoutsDir = path.join(h.dir, 'layouts');
    fs.mkdirSync(layoutsDir, { recursive: true });
    const outside = path.join(h.dir, 'stolen.yaml');
    fs.writeFileSync(outside, 'id: x\nname: Stolen\ncreatedAt: z\nagents: []\n', 'utf-8');
    try {
      fs.symlinkSync(outside, path.join(layoutsDir, 'pwn.yaml'));
    } catch {
      return; // no symlink privilege here
    }
    expect(layoutService.list().map((l) => l.name)).not.toContain('Stolen');
  });

  it('still lists a symlink that stays inside the layouts dir', () => {
    const real = layoutService.save({ name: 'Real', agents: [] });
    const layoutsDir = path.join(h.dir, 'layouts');
    try {
      fs.symlinkSync(path.join(layoutsDir, `${real.id}.yaml`), path.join(layoutsDir, 'alias.yaml'));
    } catch {
      return;
    }
    expect(layoutService.list()).toHaveLength(2);
  });

  // A sibling of the store whose NAME starts with the store's — the prefix
  // collision the Go twin's TestStoreListersDoNotReadThroughASymlinkOutOfTheStore
  // covers and this side did not. Both existing cases plant their victim at
  // <configDir>/stolen.yaml, so a containment that drops the separator boundary
  // (`canonical.startsWith(dir)`) passes them both.
  it("skips an entry that resolves into a config-dir sibling whose name starts with the store's", () => {
    const layoutsDir = path.join(h.dir, 'layouts');
    const sibling = path.join(h.dir, 'layouts-backup');
    fs.mkdirSync(layoutsDir, { recursive: true });
    fs.mkdirSync(sibling, { recursive: true });
    const loot = path.join(sibling, 'loot.yaml');
    fs.writeFileSync(
      loot,
      'id: x\nname: LOOT-OUTSIDE-THE-STORE\ncreatedAt: z\nagents: []\n',
      'utf-8',
    );
    try {
      fs.symlinkSync(loot, path.join(layoutsDir, 'pwn.yaml'));
    } catch {
      return;
    }
    expect(layoutService.list().map((l) => l.name)).not.toContain('LOOT-OUTSIDE-THE-STORE');
  });
});

// The WRITE and DELETE legs of the same store. list() has always resolved and
// contained (resolveStoreEntry); layoutFilePath was purely LEXICAL —
// `path.dirname(path.resolve(full)) !== path.resolve(layoutsDir())` is a textual
// answer that cannot see a symlink entry — while its Go twin canonicalizes. Same
// bus method, same store, two different opinions about what a legal entry is,
// resolved by whichever provider a delegation flag happened to pick.
// Twin: TestLayoutWriteAndDeleteRefuseAnEntryThatResolvesOutOfTheStore.
describe('layoutService save/remove — a store entry that resolves out is refused', () => {
  const plant = (name: string): string | null => {
    const layoutsDir = path.join(h.dir, 'layouts');
    fs.mkdirSync(layoutsDir, { recursive: true });
    const victim = path.join(h.dir, 'config.yaml');
    fs.writeFileSync(victim, 'ui: {}\n', 'utf-8');
    try {
      fs.symlinkSync(victim, path.join(layoutsDir, `${name}.yaml`));
    } catch {
      return null; // no symlink privilege here
    }
    return victim;
  };

  it('save refuses a layout id whose store entry is a symlink out of the store', () => {
    const victim = plant('evil');
    if (!victim) return;
    expect(() => layoutService.save({ id: 'evil', name: 'x', agents: [] })).toThrow(
      /outside the layouts directory/,
    );
    expect(fs.readFileSync(victim, 'utf-8')).toBe('ui: {}\n');
    expect(fs.lstatSync(path.join(h.dir, 'layouts', 'evil.yaml')).isSymbolicLink()).toBe(true);
  });

  it('remove refuses the same entry rather than unlinking it', () => {
    const victim = plant('evil');
    if (!victim) return;
    layoutService.remove('evil');
    expect(fs.existsSync(path.join(h.dir, 'layouts', 'evil.yaml'))).toBe(true);
    expect(fs.existsSync(victim)).toBe(true);
  });

  it('the floor: an ordinary id still saves and removes', () => {
    const l = layoutService.save({ id: 'ordinary', name: 'Ordinary', agents: [] });
    expect(l.id).toBe('ordinary');
    expect(fs.existsSync(path.join(h.dir, 'layouts', 'ordinary.yaml'))).toBe(true);
    layoutService.remove('ordinary');
    expect(fs.existsSync(path.join(h.dir, 'layouts', 'ordinary.yaml'))).toBe(false);
  });
});
