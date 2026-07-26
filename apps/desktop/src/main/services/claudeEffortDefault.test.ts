/**
 * Resolving what "Default" effort means for a Claude session, mirroring the
 * CLI's own resolver (read out of the 2.1.219 binary):
 *
 *     if (--effort given) return it;
 *     if (settings.ultracode === true) return "xhigh";
 *     return settings.effortLevel;   // enum low|medium|high|xhigh, else dropped
 *
 * These tests pin the two rules that are easy to get wrong: `ultracode` beating
 * the persisted level, and a level outside the persistable enum being ignored
 * rather than reported (the CLI's `.catch(undefined)`).
 */
import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';

import { resolveClaudeDefaultEffort } from './claudeEffortDefault';

let tmp: string;
/** A fake `--config-dir`-style claude home, and a fake project cwd. */
let home: string;
let cwd: string;

function writeSettings(dir: string, body: unknown, name = 'settings.json'): void {
  fs.mkdirSync(dir, { recursive: true });
  fs.writeFileSync(path.join(dir, name), JSON.stringify(body));
}

beforeEach(() => {
  tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'wks-effort-'));
  home = path.join(tmp, 'claude-home');
  cwd = path.join(tmp, 'repo');
  fs.mkdirSync(home, { recursive: true });
  fs.mkdirSync(cwd, { recursive: true });
});

afterEach(() => {
  fs.rmSync(tmp, { recursive: true, force: true });
});

describe('resolveClaudeDefaultEffort', () => {
  it('reads the user-scope effortLevel from the profile config dir', () => {
    writeSettings(home, { effortLevel: 'high' });
    expect(resolveClaudeDefaultEffort(cwd, home)).toBe('high');
  });

  it('project-local settings win over project, which win over user', () => {
    writeSettings(home, { effortLevel: 'low' });
    writeSettings(path.join(cwd, '.claude'), { effortLevel: 'medium' });
    expect(resolveClaudeDefaultEffort(cwd, home)).toBe('medium');

    writeSettings(path.join(cwd, '.claude'), { effortLevel: 'xhigh' }, 'settings.local.json');
    expect(resolveClaudeDefaultEffort(cwd, home)).toBe('xhigh');
  });

  it('ultracode:true resolves to xhigh, ahead of the persisted level', () => {
    writeSettings(home, { effortLevel: 'low', ultracode: true });
    expect(resolveClaudeDefaultEffort(cwd, home)).toBe('xhigh');
  });

  it('ignores a level the CLI would not persist, and keeps looking', () => {
    // `max` is valid for the --effort *flag* but outside the settings enum, so
    // the CLI drops it. Reporting it would name a level the session isn't using.
    writeSettings(path.join(cwd, '.claude'), { effortLevel: 'max' });
    writeSettings(home, { effortLevel: 'high' });
    expect(resolveClaudeDefaultEffort(cwd, home)).toBe('high');
  });

  it('is undefined when nothing pins a level — the model default has no name', () => {
    writeSettings(home, { theme: 'dark' });
    expect(resolveClaudeDefaultEffort(cwd, home)).toBeUndefined();
  });

  it('survives a missing or malformed settings file', () => {
    expect(resolveClaudeDefaultEffort(cwd, home)).toBeUndefined();
    fs.writeFileSync(path.join(home, 'settings.json'), '{ not json');
    expect(resolveClaudeDefaultEffort(cwd, home)).toBeUndefined();
  });

  it('works with no cwd (user scope only)', () => {
    writeSettings(home, { effortLevel: 'medium' });
    expect(resolveClaudeDefaultEffort(undefined, home)).toBe('medium');
  });
});
