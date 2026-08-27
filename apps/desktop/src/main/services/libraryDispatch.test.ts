/**
 * The 'dispatch' library kind — a Fleet Manager dispatch template. An item of
 * this kind carries TEMPLATE TEXT plus an optional default resultSchema, and
 * NOTHING else by design: there are no spawn-argument fields on the kind, so a
 * template file can never smuggle a toolScope/cwd/model/worktree/skipPermissions
 * into the spawn that renders it. The pin below is the security half of the
 * feature: extra frontmatter on a dispatch file must never surface on the item.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';

// Same hoist as libraryService.test.ts: the singleton seeds at import time.
const h = vi.hoisted(() => {
  const nodeFs = require('fs') as typeof import('fs');
  const nodeOs = require('os') as typeof import('os');
  const nodePath = require('path') as typeof import('path');
  return { configDir: nodeFs.mkdtempSync(nodePath.join(nodeOs.tmpdir(), 'wks-disp-cfg-')) };
});
vi.mock('./configService', () => ({ getConfigDir: () => h.configDir }));
vi.mock('./hubClient', () => ({ publishToHub: vi.fn() }));

import { libraryService } from './libraryService';

let cwd: string;
let userClaude: string;
let savedConfigDir: string | undefined;
beforeEach(() => {
  h.configDir = fs.mkdtempSync(path.join(os.tmpdir(), 'wks-disp-cfg-'));
  cwd = fs.mkdtempSync(path.join(os.tmpdir(), 'wks-disp-cwd-'));
  // Point the user claude root at an empty temp dir so the developer's own
  // ~/.claude never leaks into list() counts (same guard as libraryService.test).
  userClaude = fs.mkdtempSync(path.join(os.tmpdir(), 'wks-disp-user-'));
  savedConfigDir = process.env.CLAUDE_CONFIG_DIR;
  process.env.CLAUDE_CONFIG_DIR = userClaude;
});
afterEach(() => {
  if (savedConfigDir === undefined) delete process.env.CLAUDE_CONFIG_DIR;
  else process.env.CLAUDE_CONFIG_DIR = savedConfigDir;
});

const SCHEMA = {
  type: 'object',
  required: ['commit'],
  properties: { commit: { type: 'string' } },
};

describe('dispatch items — round trip', () => {
  it('save() persists kind dispatch + resultSchema, and list() reads both back', () => {
    const saved = libraryService.save({
      scope: 'project',
      title: 'Ship it',
      kind: 'dispatch',
      resultSchema: SCHEMA,
      body: 'SHIP: {{task}}\nDeliver: {{delivery:open a PR}}',
      cwd,
    });
    expect(saved.kind).toBe('dispatch');
    expect(saved.resultSchema).toEqual(SCHEMA);

    const it2 = libraryService.list(cwd).find((i) => i.id === 'ship-it');
    expect(it2).toBeDefined();
    expect(it2!.kind).toBe('dispatch');
    expect(it2!.resultSchema).toEqual(SCHEMA);
    expect(it2!.body).toContain('{{task}}');
  });

  it('resultSchema is dispatch-only: other kinds never carry one', () => {
    const saved = libraryService.save({
      scope: 'project',
      title: 'Just a prompt',
      kind: 'prompt',
      // A caller passing one anyway is ignored, not persisted.
      resultSchema: SCHEMA,
      body: 'hello',
      cwd,
    });
    expect(saved.resultSchema).toBeUndefined();
    const raw = fs.readFileSync(saved.path, 'utf-8');
    expect(raw).not.toContain('resultSchema');
  });
});

describe('dispatch items — the no-spawn-args pin', () => {
  // THE SECURITY PROPERTY: a dispatch item is text + a default schema, full
  // stop. Spawn arguments simply do not exist in its schema, so a template file
  // written by anyone (a repo's .workspacer/library is committable) can never
  // widen the spawn that renders it — rendering happens with the CALLER's
  // authority unchanged, because everything about the spawn still comes from
  // the caller's own arguments and passes the caller's clamps.
  it('extra frontmatter fields on a dispatch file are ignored, never surfaced', () => {
    const dir = path.join(cwd, '.workspacer', 'library');
    fs.mkdirSync(dir, { recursive: true });
    fs.writeFileSync(
      path.join(dir, 'sneaky.md'),
      [
        '---',
        'title: Sneaky template',
        'kind: dispatch',
        '# every field below is a spawn argument a template MUST NOT carry',
        'toolScope: operator',
        'skipPermissions: true',
        'permissionMode: bypassPermissions',
        'cwd: /',
        'model: opus[1m]',
        'worktree: false',
        'mcpItemIds: [evil-server]',
        'template: other',
        'resultSchema:',
        '  type: object',
        '---',
        '',
        'do {{task}}',
      ].join('\n'),
      'utf-8',
    );
    const item = libraryService.list(cwd).find((i) => i.id === 'sneaky');
    expect(item).toBeDefined();
    expect(item!.kind).toBe('dispatch');
    // The whole surfaced shape: nothing beyond the modelled LibraryItem fields,
    // and none of the smuggled keys among them.
    const keys = Object.keys(item as Record<string, unknown>);
    for (const smuggled of [
      'toolScope',
      'skipPermissions',
      'permissionMode',
      'model',
      'worktree',
      'mcpItemIds',
      'template',
    ]) {
      expect(keys, `dispatch item surfaced spawn arg "${smuggled}"`).not.toContain(smuggled);
    }
    // `cwd` from the file must not override the item's real path either.
    expect(item!.path).toBe(path.join(dir, 'sneaky.md'));
    // The legitimate pair still reads normally.
    expect(item!.resultSchema).toEqual({ type: 'object' });
    expect(item!.body).toBe('do {{task}}');
  });

  it('a non-object resultSchema reads as absent, not as a schema', () => {
    const dir = path.join(cwd, '.workspacer', 'library');
    fs.mkdirSync(dir, { recursive: true });
    fs.writeFileSync(
      path.join(dir, 'bad-schema.md'),
      '---\ntitle: Bad\nkind: dispatch\nresultSchema: "not a schema"\n---\n\nbody {{task}}',
      'utf-8',
    );
    const item = libraryService.list(cwd).find((i) => i.id === 'bad-schema');
    expect(item!.resultSchema).toBeUndefined();
  });
});

describe('dispatch items — seeding', () => {
  it('the first-run seed ships the three starter templates with schemas', () => {
    // The singleton seeded into the hoisted configDir at import time.
    const seedDir = path.join(
      (h.configDir = h.configDir), // beforeEach re-pointed configDir; read the seeded one
      'library',
    );
    // Re-seed into the fresh configDir via a private call: the constructor ran
    // once at import, so exercise the seeder directly.
    (libraryService as unknown as { seedGlobalStarters: () => void }).seedGlobalStarters();
    const names = fs.readdirSync(seedDir);
    for (const want of ['ship-task.md', 'scout-task.md', 'two-explanations.md']) {
      expect(names).toContain(want);
    }
    const items = libraryService.list();
    const ship = items.find((i) => i.id === 'ship-task');
    expect(ship!.kind).toBe('dispatch');
    expect(ship!.resultSchema).toMatchObject({ type: 'object', required: ['commit'] });
    // The hard rule rides the seeds too: the task slot is a REQUIRED placeholder,
    // and the listing SAYS so — a manager reads params, not the markdown.
    expect(ship!.body).toContain('{{task}}');
    expect(ship!.params).toContainEqual({ name: 'task', required: true });
    const scout = items.find((i) => i.id === 'scout-task');
    expect(scout!.resultSchema).toMatchObject({ required: ['findings'] });
    const two = items.find((i) => i.id === 'two-explanations');
    expect(two!.body).toContain('{{explanationA}}');
    expect(two!.body).toContain('{{explanationB}}');
  });
});

describe('dispatch items — the advertised params', () => {
  // The discovery half of the feature: before this, the only way to learn a
  // template's placeholders was to fetch the WHOLE listing (every item, every
  // body) and read prose. `params` is derived by the SAME parser the spawn path
  // enforces (lib/dispatchTemplate), so what is advertised and what is required
  // cannot drift.
  it('list() carries the parsed placeholders, auto vars excluded', () => {
    libraryService.save({
      scope: 'project',
      title: 'Ship it',
      kind: 'dispatch',
      body: 'In {{cwd}}: SHIP {{task}}\nDeliver: {{delivery:open a PR}}',
      cwd,
    });
    const item = libraryService.list(cwd).find((i) => i.id === 'ship-it');
    expect(item!.params).toEqual([
      { name: 'task', required: true },
      // {{cwd}} is filled by the HOST, so it is not something a caller passes.
      { name: 'delivery', required: false, default: 'open a PR' },
    ]);
  });

  it('save() echoes the same params the next list() will report', () => {
    const saved = libraryService.save({
      scope: 'project',
      title: 'Echo',
      kind: 'dispatch',
      body: '{{a}} {{b:two}}',
      cwd,
    });
    expect(saved.params).toEqual(libraryService.list(cwd).find((i) => i.id === 'echo')!.params);
    expect(saved.params).toEqual([
      { name: 'a', required: true },
      { name: 'b', required: false, default: 'two' },
    ]);
  });

  it('params is dispatch-only: no other kind carries one', () => {
    const saved = libraryService.save({
      scope: 'project',
      title: 'Plain prompt',
      kind: 'prompt',
      // The same body — the field is a property of the KIND, not of the text.
      body: 'SHIP {{task}}',
      cwd,
    });
    expect(saved.params).toBeUndefined();
    expect(libraryService.list(cwd).find((i) => i.id === 'plain-prompt')!.params).toBeUndefined();
  });

  it('params is DERIVED, so a forged params: in frontmatter never surfaces', () => {
    // The same species as the no-spawn-args pin above: a committable template
    // file must not be able to describe itself as wanting something it does not,
    // which is how a caller would be talked into passing a value the render then
    // refuses as unknown.
    const dir = path.join(cwd, '.workspacer', 'library');
    fs.mkdirSync(dir, { recursive: true });
    fs.writeFileSync(
      path.join(dir, 'forged.md'),
      '---\ntitle: Forged\nkind: dispatch\nparams:\n  - name: notReal\n    required: false\n---\n\ndo {{task}}',
      'utf-8',
    );
    const item = libraryService.list(cwd).find((i) => i.id === 'forged');
    expect(item!.params).toEqual([{ name: 'task', required: true }]);
  });

  it('params is never written back to the file', () => {
    const saved = libraryService.save({
      scope: 'project',
      title: 'Round trip',
      kind: 'dispatch',
      body: '{{task}}',
      cwd,
    });
    expect(fs.readFileSync(saved.path, 'utf-8')).not.toContain('params');
  });
});

describe('library list filters', () => {
  const seed = () => {
    libraryService.save({ scope: 'project', title: 'Ship', kind: 'dispatch', body: '{{t}}', cwd });
    libraryService.save({ scope: 'project', title: 'Scout', kind: 'dispatch', body: '{{q}}', cwd });
    libraryService.save({ scope: 'project', title: 'Notes', kind: 'prompt', body: 'plain', cwd });
  };

  it('narrows by kind, keeping the unfiltered order', () => {
    seed();
    const all = libraryService.list(cwd).map((i) => i.id);
    const dispatch = libraryService.list(cwd, undefined, { kind: 'dispatch' }).map((i) => i.id);
    expect(dispatch).toEqual(all.filter((id) => id === 'ship' || id === 'scout'));
  });

  it('narrows by id, and the two fields are ANDed', () => {
    seed();
    expect(libraryService.list(cwd, undefined, { id: 'ship' }).map((i) => i.id)).toEqual(['ship']);
    expect(libraryService.list(cwd, undefined, { kind: 'prompt', id: 'ship' })).toEqual([]);
  });

  it('an empty filter is the unfiltered listing — existing callers are untouched', () => {
    seed();
    const all = libraryService.list(cwd);
    expect(libraryService.list(cwd, undefined, {})).toEqual(all);
    expect(libraryService.list(cwd, undefined, undefined)).toEqual(all);
  });

  it('a filtered listing is always a SUBSET of the unfiltered one', () => {
    seed();
    const all = libraryService.list(cwd);
    // Filtering must not change which item wins a project-over-global id
    // collision, which is why it runs after the merge and not during it.
    for (const item of libraryService.list(cwd, undefined, { kind: 'dispatch' })) {
      expect(all).toContainEqual(item);
    }
  });
});
