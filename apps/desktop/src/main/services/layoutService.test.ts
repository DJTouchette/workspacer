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

beforeEach(() => { h.dir = fs.mkdtempSync(path.join(os.tmpdir(), 'wks-layout-')); });
afterEach(() => { fs.rmSync(h.dir, { recursive: true, force: true }); });

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
