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

  it('read returns a file as it stands at the current scrub position', async () => {
    await timelineReplay.open(repo, 'sess-replay-1', '2026-03-01T00:00:00Z');
    await timelineReplay.seek('sess-replay-1', [
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

    const edited = await timelineReplay.read('sess-replay-1', 'login.js');
    expect(edited.content).toContain('checkCredentials');
    expect(edited.binary).toBe(false);
    expect(edited.truncated).toBe(false);

    // An absolute transcript path (pointing at the agent's real checkout) is
    // remapped into the worktree rather than read from the real repo.
    const viaAbsolute = await timelineReplay.read('sess-replay-1', path.join(repo, 'login.js'));
    expect(viaAbsolute.path).toBe('login.js');
    expect(viaAbsolute.content).toContain('checkCredentials');

    const written = await timelineReplay.read('sess-replay-1', 'health.js');
    expect(written.content).toBe('ok()\n');
  });

  it('read refuses paths that escape the worktree', async () => {
    await expect(timelineReplay.read('sess-replay-1', '../../etc/passwd')).rejects.toThrow(
      /outside the replay worktree/,
    );
    // Normalizes to an escape only after resolution — the string test alone
    // would let this through.
    await expect(timelineReplay.read('sess-replay-1', 'a/../../../etc/passwd')).rejects.toThrow(
      /outside the replay worktree/,
    );
    // Absolute, and not under the repo root at all.
    await expect(timelineReplay.read('sess-replay-1', '/etc/passwd')).rejects.toThrow(
      /outside the replay worktree/,
    );
    await expect(timelineReplay.read('sess-replay-1', '')).rejects.toThrow(/path is required/);
  });

  it('read reports a file that does not exist yet at this position', async () => {
    await expect(timelineReplay.read('sess-replay-1', 'later.txt')).rejects.toThrow(
      /does not exist at this point in the timeline/,
    );
  });

  it('read flags binary content instead of returning it', async () => {
    await timelineReplay.seek('sess-replay-1', [
      {
        name: 'Write',
        input: {
          file_path: path.join(repo, 'blob.bin'),
          content: 'a' + String.fromCharCode(0) + 'b',
        },
      },
    ]);
    const res = await timelineReplay.read('sess-replay-1', 'blob.bin');
    expect(res.binary).toBe(true);
    expect(res.content).toBe('');
    expect(res.bytes).toBeGreaterThan(0);
  });

  it('diff reports written + edited files against the base commit', async () => {
    await timelineReplay.seek('sess-replay-1', [
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

    const res = await timelineReplay.diff('sess-replay-1');
    expect(res.baseCommit).toHaveLength(40);
    expect(res.truncated).toBe(false);
    const byPath = new Map(res.files.map((f) => [f.path, f]));
    // A Write of a new file is untracked in the worktree — it only appears
    // here because diff stages it intent-to-add first.
    expect(byPath.get('health.js')?.status).toBe('added');
    expect(byPath.get('login.js')?.status).toBe('modified');
    expect(byPath.get('login.js')?.additions).toBe(1);
    expect(byPath.get('login.js')?.deletions).toBe(1);
    expect(res.patch).toContain('checkCredentials');
    expect(res.patch).toContain('health.js');
  });

  it('diff scopes to one file when asked', async () => {
    const res = await timelineReplay.diff('sess-replay-1', 'login.js');
    expect(res.files.map((f) => f.path)).toEqual(['login.js']);
    expect(res.patch).toContain('checkCredentials');
    expect(res.patch).not.toContain('health.js');
  });

  it('a seek after a diff still clears intent-to-add files', async () => {
    // Regression: `git clean` will not remove a file the index knows about, so
    // without seek's index reset the previous diff's `add -N` would leave
    // health.js behind and the timeline would show a future edit.
    const { dir } = await timelineReplay.open(repo, 'sess-replay-1', '2026-03-01T00:00:00Z');
    const res = await timelineReplay.seek('sess-replay-1', []);
    expect(res.changedFiles).toBe(0);
    expect(fs.existsSync(path.join(dir, 'health.js'))).toBe(false);
    expect(fs.existsSync(path.join(dir, 'blob.bin'))).toBe(false);
    expect((await timelineReplay.diff('sess-replay-1')).files).toEqual([]);
  });

  it('read and diff require an open replay', async () => {
    await expect(timelineReplay.read('sess-never-opened', 'x.js')).rejects.toThrow(
      /replay not open/,
    );
    await expect(timelineReplay.diff('sess-never-opened')).rejects.toThrow(/replay not open/);
  });

  it('close removes the worktree; seek then requires reopening', async () => {
    const { dir } = await timelineReplay.open(repo, 'sess-replay-1', '2026-03-01T00:00:00Z');
    await timelineReplay.close('sess-replay-1');
    expect(fs.existsSync(dir)).toBe(false);
    await expect(timelineReplay.seek('sess-replay-1', [])).rejects.toThrow(/replay not open/);
  });
});
