/**
 * Exercises worktree replay against a real temporary git repository: base
 * commit resolution by timestamp, seek applying/reverting ops, containment of
 * traversal-shaped paths, and teardown. Skips nothing — git is a hard
 * dependency of the feature.
 */
import { describe, it, expect, beforeAll, afterAll } from 'vitest';
import { execFileSync } from 'child_process';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import { timelineReplay } from './timelineReplayService';

let repo: string;

function git(args: string[], env: Record<string, string> = {}) {
  execFileSync('git', args, {
    cwd: repo,
    env: { ...process.env, ...env },
    stdio: 'pipe',
  });
}

function commitAll(message: string, isoDate: string) {
  git(['add', '-A']);
  git(['commit', '-m', message, '--no-gpg-sign'], {
    GIT_AUTHOR_DATE: isoDate,
    GIT_COMMITTER_DATE: isoDate,
    GIT_AUTHOR_NAME: 't',
    GIT_AUTHOR_EMAIL: 't@t',
    GIT_COMMITTER_NAME: 't',
    GIT_COMMITTER_EMAIL: 't@t',
  });
}

beforeAll(() => {
  repo = fs.mkdtempSync(path.join(os.tmpdir(), 'wks-replay-repo-'));
  git(['init', '-q']);
  fs.writeFileSync(path.join(repo, 'login.js'), 'function login() {\n  return false;\n}\n');
  commitAll('base', '2026-01-01T00:00:00Z');
  fs.writeFileSync(path.join(repo, 'later.txt'), 'added after the session started\n');
  commitAll('later', '2026-06-01T00:00:00Z');
});

afterAll(async () => {
  await timelineReplay.close('sess-replay-1');
  fs.rmSync(repo, { recursive: true, force: true });
});

describe('timelineReplay', () => {
  it('rejects a non-git cwd and traversal-shaped session ids', async () => {
    const plain = fs.mkdtempSync(path.join(os.tmpdir(), 'wks-plain-'));
    await expect(timelineReplay.open(plain, 'sess-x')).rejects.toThrow(/not a git repository/);
    fs.rmSync(plain, { recursive: true, force: true });
    await expect(timelineReplay.open(repo, '../evil')).rejects.toThrow(/invalid session id/);
  });

  it('open picks the last commit before the session started', async () => {
    // Session began 2026-03-01: the 2026-06 commit must not be in the base.
    const { dir, baseCommit } = await timelineReplay.open(
      repo,
      'sess-replay-1',
      '2026-03-01T00:00:00Z',
    );
    expect(baseCommit).toHaveLength(40);
    expect(fs.existsSync(path.join(dir, 'login.js'))).toBe(true);
    expect(fs.existsSync(path.join(dir, 'later.txt'))).toBe(false);
  });

  it('seek applies Write + Edit ops into the worktree only', async () => {
    const { dir } = await timelineReplay.open(repo, 'sess-replay-1', '2026-03-01T00:00:00Z');
    const res = await timelineReplay.seek('sess-replay-1', [
      { name: 'Write', input: { file_path: path.join(repo, 'health.js'), content: 'ok()\n' } },
      {
        name: 'Edit',
        input: {
          file_path: path.join(repo, 'login.js'),
          old_string: 'return false;',
          new_string: 'return checkCredentials();',
        },
      },
    ]);
    expect(res.applied).toBe(2);
    expect(res.skipped).toEqual([]);
    expect(res.changedFiles).toBe(2);
    expect(fs.readFileSync(path.join(dir, 'login.js'), 'utf8')).toContain('checkCredentials');
    expect(fs.readFileSync(path.join(dir, 'health.js'), 'utf8')).toBe('ok()\n');
    // The real repo is untouched.
    expect(fs.readFileSync(path.join(repo, 'login.js'), 'utf8')).toContain('return false;');
    expect(fs.existsSync(path.join(repo, 'health.js'))).toBe(false);
  });

  it('scrubbing back (fewer ops) reverts to the base state', async () => {
    const { dir } = await timelineReplay.open(repo, 'sess-replay-1', '2026-03-01T00:00:00Z');
    const res = await timelineReplay.seek('sess-replay-1', []);
    expect(res.applied).toBe(0);
    expect(res.changedFiles).toBe(0);
    expect(fs.readFileSync(path.join(dir, 'login.js'), 'utf8')).toContain('return false;');
    expect(fs.existsSync(path.join(dir, 'health.js'))).toBe(false);
  });

  it('skips ops that escape the repository or no longer match', async () => {
    const res = await timelineReplay.seek('sess-replay-1', [
      { name: 'Write', input: { file_path: '/etc/passwd', content: 'nope' } },
      {
        name: 'Edit',
        input: { file_path: path.join(repo, 'login.js'), old_string: 'NOT THERE', new_string: 'x' },
      },
      { name: 'Bash', input: {} },
    ]);
    expect(res.applied).toBe(0);
    expect(res.skipped.map((s) => s.reason)).toEqual([
      'outside the repository',
      'old_string not found (file diverged from the transcript)',
      'unsupported op',
    ]);
  });

  it('close removes the worktree; seek then requires reopening', async () => {
    const { dir } = await timelineReplay.open(repo, 'sess-replay-1', '2026-03-01T00:00:00Z');
    await timelineReplay.close('sess-replay-1');
    expect(fs.existsSync(dir)).toBe(false);
    await expect(timelineReplay.seek('sess-replay-1', [])).rejects.toThrow(/replay not open/);
  });
});
