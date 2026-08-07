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
import { diff as gitDiff, stage as gitStage } from '../services/gitService';
import { assertPathAllowed } from './pathConfinement';
import { GIT_NO_EXEC_CONFIG, GIT_NO_EXEC_KEYS, gitArgs } from './gitExec';

/** A repository skeleton written with nothing but ordinary file writes — the
 *  shape a caller reaches through fs.write inside an allowed root. `extraConfig`
 *  is appended to .git/config verbatim. */
function plantRepo(sandbox: string, extraConfig: string, real = false): string {
  const repo = path.join(sandbox, 'store');
  fs.mkdirSync(repo, { recursive: true });
  if (real) {
    execFileSync('git', ['init', '-q', '.'], { cwd: repo, stdio: 'ignore' });
    execFileSync('git', ['config', 'user.email', 't@t'], { cwd: repo, stdio: 'ignore' });
    execFileSync('git', ['config', 'user.name', 't'], { cwd: repo, stdio: 'ignore' });
    fs.writeFileSync(path.join(repo, 'a.txt'), 'hello\n');
    execFileSync('git', ['add', 'a.txt'], { cwd: repo, stdio: 'ignore' });
    execFileSync('git', ['commit', '-qm', 'one'], { cwd: repo, stdio: 'ignore' });
    fs.writeFileSync(path.join(repo, 'a.txt'), 'changed\n');
  } else {
    fs.mkdirSync(path.join(repo, '.git', 'refs', 'heads'), { recursive: true });
    fs.mkdirSync(path.join(repo, '.git', 'objects', 'info'), { recursive: true });
    fs.writeFileSync(path.join(repo, '.git', 'HEAD'), 'ref: refs/heads/main\n');
    fs.writeFileSync(path.join(repo, '.git', 'objects', 'info', 'packs'), '');
    fs.writeFileSync(path.join(repo, '.git', 'config'), '[core]\n\trepositoryformatversion = 0\n');
    fs.writeFileSync(path.join(repo, 'a.txt'), 'hello');
  }
  fs.appendFileSync(path.join(repo, '.git', 'config'), extraConfig);
  return repo;
}

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

  // `diff.external` cannot be neutralized with `-c` — git treats an empty
  // external diff as a command to RUN and dies with "cannot run :" — so the off
  // switch is a diff OPTION, which means it lives after the subcommand and is
  // therefore gitArgs' job rather than every caller's.
  it('inserts --no-ext-diff after a diff-family subcommand, and nowhere else', () => {
    expect(gitArgs(['diff', '--numstat'])).toEqual([
      ...GIT_NO_EXEC_CONFIG,
      'diff',
      '--no-ext-diff',
      '--numstat',
    ]);
    expect(gitArgs(['show', '--format=', '--patch', 'HEAD'])[GIT_NO_EXEC_CONFIG.length + 1]).toBe(
      '--no-ext-diff',
    );
    expect(gitArgs(['log', '-n', '1'])).toContain('--no-ext-diff');
    // A caller's own leading `-c` must not be mistaken for the subcommand.
    expect(gitArgs(['-c', 'core.quotepath=false', 'diff', '--numstat'])).toEqual([
      ...GIT_NO_EXEC_CONFIG,
      '-c',
      'core.quotepath=false',
      'diff',
      '--no-ext-diff',
      '--numstat',
    ]);
    expect(gitArgs(['status', '--porcelain'])).not.toContain('--no-ext-diff');
    expect(gitArgs(['add', '-A'])).not.toContain('--no-ext-diff');
  });

  // An EMPTY value has to be the "no program" spelling for every key in the list,
  // or the prefix breaks git instead of hardening it — which is exactly what
  // `diff.external=` did (two timelineReplay tests went to an empty patch).
  (HAS_GIT ? it : it.skip)('every neutralized key leaves an ordinary diff intact', () => {
    const sandbox = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'wks-gitexec-keys-')));
    const repo = plantRepo(sandbox, '', true);
    const out = execFileSync('git', gitArgs(['diff']), { cwd: repo, encoding: 'utf-8' });
    expect(out, `the no-exec prefix broke git diff: ${GIT_NO_EXEC_KEYS.join(' ')}`).toContain(
      '+changed',
    );
    fs.rmSync(sandbox, { recursive: true, force: true });
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

  // diff.external is the second key that actually fires, and it fires on the
  // subcommand git.diff runs. Both this and the filter probe below were live
  // against the shipped `-c core.fsmonitor=` prefix.
  (HAS_GIT ? it : it.skip)('git.diff does not execute .git/config diff.external', async () => {
    const sandbox = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'wks-gitexec-ext-')));
    const marker = path.join(sandbox, 'PWNED_DIFF');
    const repo = plantRepo(sandbox, `[diff]\n\texternal = "sh -c 'touch ${marker}' #"\n`, true);

    await gitDiff(repo, 'a.txt');
    await gitDiff(repo, 'a.txt', true);

    expect(fs.existsSync(marker), `ARBITRARY COMMAND EXECUTED via git.diff: ${marker}`).toBe(false);
    fs.rmSync(sandbox, { recursive: true, force: true });
  });

  // filter.<drv>.clean is NAMESPACED, so no `-c` list can ever name it, and
  // `git add` (git.stage) runs it. There is no command-line off switch. The leg
  // is therefore closed on the WRITE side, and this test states both halves so
  // neither can be quietly dropped: the driver definition has to live under
  // `.git`, and every caller-supplied path that traverses `.git` is refused.
  (HAS_GIT ? it : it.skip)('the filter.clean chain is closed on the write side', async () => {
    const sandbox = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'wks-gitexec-flt-')));
    const marker = path.join(sandbox, 'PWN_CLEAN');
    const repo = plantRepo(sandbox, '', true);

    // Half one: the definition cannot be written through a guarded capability,
    // even though `repo` is a fully allowed root and an ordinary file in it is.
    expect(() => assertPathAllowed('fs.write', path.join(repo, 'a.txt'), [repo])).not.toThrow();
    for (const rel of ['.git/config', '.git/config.worktree', '.git/info/attributes']) {
      expect(
        () => assertPathAllowed('fs.write', path.join(repo, rel), [repo]),
        `${rel} is writable — filter.<drv>.clean has nothing standing in front of it`,
      ).toThrow(/outside the allowed workspace/);
    }

    // Half two: given the definition (planted here directly, i.e. a repository
    // this process did not write), `git add` DOES run it — which is why half one
    // has to hold. Asserting this keeps the comment honest: if a future git
    // grows a `--no-filters`, this flips and the test says so.
    fs.writeFileSync(path.join(repo, '.gitattributes'), '*.txt filter=evil\n');
    fs.appendFileSync(
      path.join(repo, '.git', 'config'),
      `[filter "evil"]\n\tclean = "sh -c 'touch ${marker}'; cat"\n`,
    );
    await gitStage(repo, 'a.txt');
    expect(fs.existsSync(marker)).toBe(true);

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
