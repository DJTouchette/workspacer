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
    (libraryService as unknown as { seedGlobalIfEmpty: () => void }).seedGlobalIfEmpty();
    const names = fs.readdirSync(seedDir);
    for (const want of ['ship-task.md', 'scout-task.md', 'two-explanations.md']) {
      expect(names).toContain(want);
    }
    const items = libraryService.list();
    const ship = items.find((i) => i.id === 'ship-task');
    expect(ship!.kind).toBe('dispatch');
    expect(ship!.resultSchema).toMatchObject({ type: 'object', required: ['commit'] });
    // The hard rule rides the seeds too: the task slot is a REQUIRED placeholder.
    expect(ship!.body).toContain('{{task}}');
    const scout = items.find((i) => i.id === 'scout-task');
    expect(scout!.resultSchema).toMatchObject({ required: ['findings'] });
    const two = items.find((i) => i.id === 'two-explanations');
    expect(two!.body).toContain('{{explanationA}}');
    expect(two!.body).toContain('{{explanationB}}');
  });
});
