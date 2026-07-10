import { describe, it, expect } from 'vitest';
import {
  parsePorcelain,
  parseNumstat,
  parseNumstatPath,
  parseLog,
  parseBranchHeader,
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
