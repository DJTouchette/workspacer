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
