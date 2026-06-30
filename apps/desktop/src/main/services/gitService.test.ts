import { describe, it, expect } from 'vitest';
import { parsePorcelain, parseNumstat, parseNumstatPath } from './gitService';

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
