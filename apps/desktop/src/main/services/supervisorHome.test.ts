import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import * as os from 'os';
import * as fs from 'fs';
import * as path from 'path';

// os.homedir() has to be mocked at the module boundary: an ESM namespace object
// is not configurable, so vi.spyOn cannot reach it. Same shape as
// analyticsBackfill.test.ts.
const homeState = { dir: '' };
vi.mock('os', async (importOriginal) => {
  const actual = await importOriginal<typeof import('os')>();
  const homedir = () => homeState.dir || actual.homedir();
  return { ...actual, default: { ...actual, homedir }, homedir };
});

import { ensureSupervisorHome } from './supervisorSkill';

/**
 * ~/.workspacer stopped being a scratch space the moment it started holding
 * things the user is meant to keep: model-rates.json, the fleet brief, and now
 * the scripts a job spec points at. Its README used to end "Safe to delete; it
 * is recreated on the next supervisor spawn", which is a sentence that, taken
 * at its word, loses settings. These pin the replacement, and the upgrade path
 * for the copy already sitting on every existing install.
 */
describe('ensureSupervisorHome README', () => {
  let home: string;

  beforeEach(() => {
    home = fs.mkdtempSync(path.join(os.tmpdir(), 'wks-home-'));
    homeState.dir = home;
  });

  afterEach(() => {
    homeState.dir = '';
    fs.rmSync(home, { recursive: true, force: true });
  });

  const readme = () => fs.readFileSync(path.join(home, '.workspacer', 'README.md'), 'utf8');

  it('does not tell the user the directory is safe to delete', () => {
    ensureSupervisorHome();
    expect(readme()).not.toMatch(/safe to delete/i);
    expect(readme()).not.toMatch(/recreated on the next supervisor spawn/i);
  });

  it('names what lives there and where job specs live instead', () => {
    ensureSupervisorHome();
    const text = readme();
    expect(text).toContain('model-rates.json');
    expect(text).toContain('scripts/');
    expect(text).toContain('brief.md');
    // The one file people reasonably expect here and that is deliberately
    // somewhere else, because it belongs to the hub process.
    expect(text).toContain('~/.config/workspacer-hub/jobs.json');
  });

  it('upgrades the old README in place on an existing install', () => {
    const dir = path.join(home, '.workspacer');
    fs.mkdirSync(dir, { recursive: true });
    fs.writeFileSync(
      path.join(dir, 'README.md'),
      '# Workspacer supervisor home\n\n' +
        'This directory is the working directory for fleet **supervisor** agents\n' +
        'spawned from Workspacer (Ask the Fleet). They coordinate your other\n' +
        'Claude Code agents via the workspacer MCP tools and use this folder as a\n' +
        'neutral scratch space — notes, digests, etc. Safe to delete; it is\n' +
        'recreated on the next supervisor spawn.\n',
      'utf8',
    );

    ensureSupervisorHome();
    expect(readme()).not.toMatch(/safe to delete/i);
  });

  it('leaves a README the user has edited alone', () => {
    const dir = path.join(home, '.workspacer');
    fs.mkdirSync(dir, { recursive: true });
    fs.writeFileSync(path.join(dir, 'README.md'), 'my own notes\n', 'utf8');

    ensureSupervisorHome();
    expect(readme()).toBe('my own notes\n');
  });
});
