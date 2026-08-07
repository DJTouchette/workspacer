/**
 * replay.read / replay.seek against a repository carrying a COMMITTED SYMLINK.
 *
 * capspec leaves replay.read/replay.diff unscoped on the recorded grounds that
 * "containment is structural (resolveInside)". It was not: resolveInside and
 * seek() both contained paths with path.relative / path.resolve / path.join and
 * nothing else — a textual clean, no lstat, no readlink. The worktree is cut with
 * `git worktree add` from a real repository, and git materializes a committed
 * symlink inside it verbatim, so `vendor -> <configDir>` made the checked path
 * and the opened path two different files:
 *
 *   read: the bus caller got remote-token back — the credential that promotes a
 *         connection to TRUSTED and unlocks /plugins/install.
 *   seek: the Write op overwrote config.yaml with caller bytes (updates.channel
 *         feeds the electron-updater URL) and reported changedFiles: 0, because
 *         `git status` inside the worktree cannot see a write that landed
 *         outside it.
 *
 * capspec_test's TestUnscopedByDecisionProviderClaimsAreTrue explicitly skips
 * these entries, and the corpus has no replay case, so nothing anywhere tested
 * the claim.
 */
import { execFileSync } from 'child_process';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

import { timelineReplay } from './timelineReplayService';

let repo: string;
let victimDir: string;

function git(args: string[], env: Record<string, string> = {}): void {
  execFileSync('git', args, { cwd: repo, env: { ...process.env, ...env }, stdio: 'pipe' });
}

beforeAll(() => {
  repo = fs.mkdtempSync(path.join(os.tmpdir(), 'wks-replay-link-repo-'));
  victimDir = fs.mkdtempSync(path.join(os.tmpdir(), 'wks-replay-victim-'));
  fs.writeFileSync(path.join(victimDir, 'remote-token'), 'SUPER-SECRET-BUS-TOKEN\n');
  fs.writeFileSync(path.join(victimDir, 'config.yaml'), 'updates:\n  channel: latest\n');

  git(['init', '-q']);
  fs.writeFileSync(path.join(repo, 'login.js'), 'function login() {\n  return false;\n}\n');
  // The ordinary real-world form: git stores a symlink verbatim, so a clone —
  // or a `git worktree add` — carries it.
  fs.symlinkSync(victimDir, path.join(repo, 'vendor'));
  git(['add', '-A']);
  git(['commit', '-m', 'base', '--no-gpg-sign'], {
    GIT_AUTHOR_DATE: '2026-01-01T00:00:00Z',
    GIT_COMMITTER_DATE: '2026-01-01T00:00:00Z',
    GIT_AUTHOR_NAME: 't',
    GIT_AUTHOR_EMAIL: 't@t',
    GIT_COMMITTER_NAME: 't',
    GIT_COMMITTER_EMAIL: 't@t',
  });
});

afterAll(async () => {
  await timelineReplay.close('link-read').catch(() => {});
  await timelineReplay.close('link-seek').catch(() => {});
  fs.rmSync(repo, { recursive: true, force: true });
  fs.rmSync(victimDir, { recursive: true, force: true });
});

describe('replay.* containment through a committed symlink', () => {
  it('refuses to read through a symlink that leaves the worktree', async () => {
    await timelineReplay.open(repo, 'link-read');
    await expect(timelineReplay.read('link-read', 'vendor/remote-token')).rejects.toThrow(
      /outside the replay worktree/,
    );
    // Absolute spelling of the same file, remapped through the repo root.
    await expect(
      timelineReplay.read('link-read', path.join(repo, 'vendor', 'remote-token')),
    ).rejects.toThrow(/outside the replay worktree/);
    // The floor: an ordinary file in the worktree still reads.
    expect((await timelineReplay.read('link-read', 'login.js')).content).toContain(
      'function login',
    );
  });

  it('refuses to write through a symlink that leaves the worktree', async () => {
    await timelineReplay.open(repo, 'link-seek');
    const before = fs.readFileSync(path.join(victimDir, 'config.yaml'), 'utf8');
    const res = await timelineReplay.seek('link-seek', [
      {
        name: 'Write',
        input: {
          file_path: path.join(repo, 'vendor', 'config.yaml'),
          content: 'updates:\n  channel: http://attacker.example/feed\n',
        },
      },
    ]);
    expect(res.applied).toBe(0);
    expect(res.skipped.length).toBe(1);
    expect(fs.readFileSync(path.join(victimDir, 'config.yaml'), 'utf8')).toBe(before);
    expect(fs.readFileSync(path.join(victimDir, 'remote-token'), 'utf8')).toContain('SECRET');

    // The floor: an ordinary write inside the worktree still applies.
    const ok = await timelineReplay.seek('link-seek', [
      { name: 'Write', input: { file_path: path.join(repo, 'login.js'), content: 'ok\n' } },
    ]);
    expect(ok.applied).toBe(1);
  });
});
