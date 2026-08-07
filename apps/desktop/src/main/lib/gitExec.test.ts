// git must not be a command-execution primitive on the bus surface.
//
// Every `git` this process runs takes its cwd from a caller: fs.listEntries
// lists a directory the caller named, git.* run in an agent work tree, replay
// and worktree run in a repo derived from one. All of those are writable through
// `fs.write` — `<configDir>/library` is a configStoreRoot and the write path
// creates missing parents, so a bus client with nothing but fs.write can mint a
// `.git` skeleton where no repository existed. git discovers it at the cwd and
// `core.fsmonitor` in that config is a command git RUNS, as the desktop user,
// with no agent approval and no plugin sandbox. The path guard is irrelevant:
// the whole chain lives inside an allowed root.
//
// Two halves, because either alone rots: a behavioural probe through the real
// listDir, and a static sweep so the next `execFile('git', …)` cannot forget.

import { describe, it, expect } from 'vitest';
import { execFileSync } from 'child_process';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import { listDir } from '../services/fileService';
import { GIT_NO_EXEC_CONFIG, gitArgs } from './gitExec';

const HAS_GIT = (() => {
  try {
    execFileSync('git', ['--version'], { stdio: 'ignore' });
    return true;
  } catch {
    return false;
  }
})();

describe('git invocations carry the no-exec config prefix', () => {
  it('prefixes the subcommand, because -c outranks every config file', () => {
    expect(GIT_NO_EXEC_CONFIG).toContain('core.fsmonitor=');
    expect(gitArgs(['status'])).toEqual([...GIT_NO_EXEC_CONFIG, 'status']);
  });

  (HAS_GIT ? it : it.skip)('listDir does not execute .git/config core.fsmonitor', () => {
    const sandbox = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'wks-gitexec-')));
    const repo = path.join(sandbox, 'store');
    const marker = path.join(sandbox, 'PWNED');
    // A repository skeleton written with nothing but ordinary file writes — the
    // shape a caller reaches through fs.write inside an allowed root.
    fs.mkdirSync(path.join(repo, '.git', 'refs', 'heads'), { recursive: true });
    fs.mkdirSync(path.join(repo, '.git', 'objects', 'info'), { recursive: true });
    fs.writeFileSync(path.join(repo, '.git', 'HEAD'), 'ref: refs/heads/main\n');
    fs.writeFileSync(path.join(repo, '.git', 'objects', 'info', 'packs'), '');
    fs.writeFileSync(
      path.join(repo, '.git', 'config'),
      `[core]\n\trepositoryformatversion = 0\n\tfsmonitor = "sh -c 'touch ${marker}; echo'"\n`,
    );
    fs.writeFileSync(path.join(repo, 'a.txt'), 'hello');

    listDir(repo);

    expect(fs.existsSync(marker), `ARBITRARY COMMAND EXECUTED via listDir: ${marker}`).toBe(false);
    fs.rmSync(sandbox, { recursive: true, force: true });
  });

  it('every git invocation in main/ goes through gitArgs', () => {
    const root = path.join(__dirname, '..');
    const offenders: string[] = [];
    let seen = 0;
    const walk = (dir: string): void => {
      for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
        const full = path.join(dir, e.name);
        if (e.isDirectory()) {
          walk(full);
          continue;
        }
        if (!e.name.endsWith('.ts') || e.name.includes('.test.')) continue;
        const src = fs.readFileSync(full, 'utf-8');
        // Prettier wraps `execFileSync(\n  'git',\n  args,` across lines, so the
        // match has to be over the whole file, not per line — a per-line regex
        // silently swept zero of the wrapped call sites while still reporting a
        // non-zero `seen` from the unwrapped ones.
        const re = /(?:execFile|execFileSync|spawn|spawnSync)\(\s*'git'\s*,/g;
        for (let m = re.exec(src); m !== null; m = re.exec(src)) {
          seen += 1;
          const line = src.slice(0, m.index).split('\n').length;
          // The argv follows the binary name; look at a small window rather than
          // trying to parse TypeScript.
          const window = src.slice(m.index, m.index + 400);
          if (!window.includes('gitArgs(')) offenders.push(`${path.relative(root, full)}:${line}`);
        }
      }
    };
    walk(root);
    expect(
      seen,
      'swept zero git invocations — this guard has stopped guarding anything',
    ).toBeGreaterThan(0);
    expect(offenders, 'git invoked without gitArgs()').toEqual([]);
  });
});
