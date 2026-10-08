import { describe, it, expect } from 'vitest';
import {
  parsePorcelain,
  parseNumstat,
  parseNumstatPath,
  parseLog,
  parseBranchHeader,
  formatGitActionError,
} from './gitService';

// These mirror the unit tests that lived in the old Rust git surface
// (services/claudemon/src/daemon/git.rs) before git moved to the host.

describe('parsePorcelain', () => {
  it('parses modified and untracked entries', () => {
    const out = ' M src/main.rs\0?? new.txt\0M  staged.rs\0';
    expect(parsePorcelain(out)).toEqual([
      { path: 'src/main.rs', orig_path: undefined, staged: ' ', unstaged: 'M' },
      { path: 'new.txt', orig_path: undefined, staged: '?', unstaged: '?' },
      { path: 'staged.rs', orig_path: undefined, staged: 'M', unstaged: ' ' },
    ]);
  });

  it('parses a rename (source follows as its own NUL token)', () => {
    const files = parsePorcelain('R  new/name.rs\0old/name.rs\0');
    expect(files).toHaveLength(1);
    expect(files[0].path).toBe('new/name.rs');
    expect(files[0].orig_path).toBe('old/name.rs');
    expect(files[0].staged).toBe('R');
  });

  it('parses a unicode path without quoting', () => {
    const files = parsePorcelain(' M src/файл.rs\0');
    expect(files).toHaveLength(1);
    expect(files[0].path).toBe('src/файл.rs');
    expect(files[0].path).not.toContain('"');
  });

  it('skips blank and too-short tokens', () => {
    expect(parsePorcelain('\0\0x\0')).toEqual([]);
  });
});

describe('parseNumstat', () => {
  it('parses counts and marks binary files as null', () => {
    const out = '12\t3\tsrc/main.rs\n-\t-\tlogo.png\n';
    expect(parseNumstat(out)).toEqual([
      { path: 'src/main.rs', added: 12, deleted: 3 },
      { path: 'logo.png', added: null, deleted: null },
    ]);
  });

  it('resolves rename paths to the new name', () => {
    expect(parseNumstatPath('old.rs => new.rs')).toBe('new.rs');
    expect(parseNumstatPath('src/{a.rs => b.rs}')).toBe('src/b.rs');
    expect(parseNumstatPath('src/{ => sub}/mod.rs')).toBe('src/sub/mod.rs');
    expect(parseNumstatPath('src/{old => }/mod.rs')).toBe('src/mod.rs');
    expect(parseNumstatPath('plain/path.rs')).toBe('plain/path.rs');
  });
});

describe('parseBranchHeader', () => {
  it('parses upstream with ahead/behind counts', () => {
    expect(parseBranchHeader('## master...origin/master [ahead 3, behind 1]')).toEqual({
      upstream: 'origin/master',
      ahead: 3,
      behind: 1,
    });
    expect(parseBranchHeader('## fix/x...origin/fix/x [ahead 2]')).toEqual({
      upstream: 'origin/fix/x',
      ahead: 2,
      behind: 0,
    });
    expect(parseBranchHeader('## main...upstream/main [behind 4]')).toEqual({
      upstream: 'upstream/main',
      ahead: 0,
      behind: 4,
    });
  });

  it('parses an in-sync upstream (no bracket)', () => {
    expect(parseBranchHeader('## master...origin/master')).toEqual({
      upstream: 'origin/master',
      ahead: 0,
      behind: 0,
    });
  });

  it('treats no upstream, gone upstream, detached, and unborn as none', () => {
    const none = { upstream: null, ahead: 0, behind: 0 };
    expect(parseBranchHeader('## master')).toEqual(none);
    expect(parseBranchHeader('## feature...origin/feature [gone]')).toEqual(none);
    expect(parseBranchHeader('## HEAD (no branch)')).toEqual(none);
    expect(parseBranchHeader('## No commits yet on master')).toEqual(none);
    expect(parseBranchHeader('')).toEqual(none);
  });
});

describe('parseLog', () => {
  it('parses NUL-separated hash/subject/author-time rows', () => {
    const out =
      'abc1234\x00fix(desktop): a thing\x001751980000\ndef5678\x00feat: another\x001751900000';
    expect(parseLog(out)).toEqual([
      { hash: 'abc1234', subject: 'fix(desktop): a thing', authoredAt: 1751980000 },
      { hash: 'def5678', subject: 'feat: another', authoredAt: 1751900000 },
    ]);
  });

  it('keeps subjects containing tabs and unicode intact', () => {
    const rows = parseLog('abc1234\x00fix: файл\twith tab\x001751980000');
    expect(rows).toHaveLength(1);
    expect(rows[0].subject).toBe('fix: файл\twith tab');
  });

  it('skips blank and malformed lines', () => {
    expect(parseLog('')).toEqual([]);
    expect(parseLog('\n\n')).toEqual([]);
    expect(parseLog('onlyhash\nabc\x00subject\x00notanumber')).toEqual([]);
  });
});

describe('formatGitActionError', () => {
  it('adds a staging hint when commit has nothing staged', () => {
    const msg = formatGitActionError('no changes added to commit');
    expect(msg).toContain('Nothing is staged to commit');
    expect(msg).toContain('no changes added to commit');
  });

  it('adds an upstream hint for first push', () => {
    const msg = formatGitActionError('fatal: The current branch feature/x has no upstream branch.');
    expect(msg).toContain('No upstream branch is configured');
    expect(msg).toContain('git push --set-upstream');
  });

  it('adds a conflict hint for unmerged paths', () => {
    const msg = formatGitActionError(
      'error: Committing is not possible because you have unmerged paths.',
    );
    expect(msg).toContain('Merge conflicts need resolution');
    expect(msg).toContain('unmerged paths');
  });

  it('adds a remote-update hint for rejected pushes', () => {
    const msg = formatGitActionError('! [rejected] main -> main (fetch first)');
    expect(msg).toContain('Push was rejected');
    expect(msg).toContain('Pull or rebase');
  });

  it('preserves unknown git errors without inventing advice', () => {
    expect(formatGitActionError('fatal: strange repository state')).toBe(
      'fatal: strange repository state',
    );
  });
});

describe('push', () => {
  it('publishes and tracks a branch that has no upstream yet', async () => {
    const { execFileSync } = await import('node:child_process');
    const fs = await import('node:fs');
    const os = await import('node:os');
    const path = await import('node:path');
    const { push } = await import('./gitService');
    const base = fs.mkdtempSync(path.join(os.tmpdir(), 'wks-git-push-'));
    try {
      const repo = path.join(base, 'repo');
      const remote = path.join(base, 'remote.git');
      const git = (cwd: string, ...args: string[]) =>
        execFileSync('git', args, { cwd, encoding: 'utf8' }).trim();
      fs.mkdirSync(repo);
      git(base, 'init', '--bare', '--quiet', remote);
      git(repo, 'init', '--quiet', '-b', 'agent-branch');
      git(repo, 'config', 'user.email', 'test@example.com');
      git(repo, 'config', 'user.name', 'Test');
      fs.writeFileSync(path.join(repo, 'a.txt'), 'a\n');
      git(repo, 'add', 'a.txt');
      git(repo, 'commit', '--quiet', '-m', 'first');
      git(repo, 'remote', 'add', 'origin', remote);
      await push(repo);
      expect(git(remote, 'rev-parse', 'refs/heads/agent-branch')).toBe(
        git(repo, 'rev-parse', 'HEAD'),
      );
      expect(git(repo, 'config', 'branch.agent-branch.remote')).toBe('origin');
    } finally {
      fs.rmSync(base, { recursive: true, force: true });
    }
  });
});

describe('pull and discard', () => {
  it('fast-forwards, and discards one file without touching staged work', async () => {
    const { execFileSync } = await import('node:child_process');
    const fs = await import('node:fs');
    const os = await import('node:os');
    const path = await import('node:path');
    const { pull, discard, push } = await import('./gitService');
    const base = fs.mkdtempSync(path.join(os.tmpdir(), 'wks-git-pull-'));
    try {
      const git = (cwd: string, ...args: string[]) =>
        execFileSync('git', args, { cwd, encoding: 'utf8' }).trim();
      const ident = (cwd: string) => {
        git(cwd, 'config', 'user.email', 'test@example.com');
        git(cwd, 'config', 'user.name', 'Test');
      };
      const repo = path.join(base, 'repo');
      const other = path.join(base, 'other');
      const remote = path.join(base, 'remote.git');
      fs.mkdirSync(repo);
      git(base, 'init', '--bare', '--quiet', '-b', 'main', remote);
      git(repo, 'init', '--quiet', '-b', 'main');
      ident(repo);
      fs.writeFileSync(path.join(repo, 'tracked.txt'), 'one\n');
      fs.writeFileSync(path.join(repo, 'staged.txt'), 'base\n');
      git(repo, 'add', '-A');
      git(repo, 'commit', '--quiet', '-m', 'base');
      git(repo, 'remote', 'add', 'origin', remote);
      await push(repo);
      git(base, 'clone', '--quiet', remote, other);
      ident(other);
      fs.writeFileSync(path.join(other, 'upstream.txt'), 'up\n');
      git(other, 'add', '-A');
      git(other, 'commit', '--quiet', '-m', 'upstream');
      git(other, 'push', '--quiet', 'origin', 'main');

      await pull(repo);
      expect(git(repo, 'rev-parse', 'HEAD')).toBe(git(other, 'rev-parse', 'HEAD'));

      fs.writeFileSync(path.join(repo, 'tracked.txt'), 'edited\n');
      fs.writeFileSync(path.join(repo, 'staged.txt'), 'staged edit\n');
      git(repo, 'add', 'staged.txt');
      fs.writeFileSync(path.join(repo, 'scratch.txt'), 'scratch\n');
      await discard(repo, 'tracked.txt');
      expect(fs.readFileSync(path.join(repo, 'tracked.txt'), 'utf8')).toBe('one\n');
      expect(fs.readFileSync(path.join(repo, 'staged.txt'), 'utf8')).toBe('staged edit\n');
      await discard(repo, 'scratch.txt');
      expect(fs.existsSync(path.join(repo, 'scratch.txt'))).toBe(false);

      fs.mkdirSync(path.join(repo, 'dir'));
      fs.writeFileSync(path.join(repo, 'dir', 'keep.txt'), 'keep\n');
      await expect(discard(repo, 'dir')).rejects.toThrow('not a folder');
      await expect(discard(repo, 'tracked.txt')).rejects.toThrow('nothing to discard');
      await expect(discard(repo, '')).rejects.toThrow('requires a file path');
      expect(fs.existsSync(path.join(repo, 'dir', 'keep.txt'))).toBe(true);
    } finally {
      fs.rmSync(base, { recursive: true, force: true });
    }
  });

  it('explains a diverged fast-forward pull', () => {
    const msg = formatGitActionError('fatal: Not possible to fast-forward, aborting.');
    expect(msg).toContain('cannot be fast-forwarded');
  });
});
