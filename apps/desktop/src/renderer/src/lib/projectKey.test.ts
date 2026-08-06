import { describe, it, expect } from 'vitest';
import { projectKey, resolveProjectKey } from './projectKey';

describe('projectKey', () => {
  it('normalizes separators and trailing slashes so one dir keys one way', () => {
    expect(projectKey('C:\\Users\\me\\work\\repo')).toBe('C:/Users/me/work/repo');
    expect(projectKey('/home/me/repo/')).toBe('/home/me/repo');
    expect(projectKey('/home/me/repo///')).toBe('/home/me/repo');
  });

  // Case is deliberately NOT folded here: lowercasing would be wrong on a
  // case-sensitive filesystem, and would orphan `scripts` entries already
  // written with mixed case. resolveProjectKey handles it at lookup instead.
  it('leaves case alone', () => {
    expect(projectKey('C:/Users/Me/Repo')).toBe('C:/Users/Me/Repo');
  });
});

describe('resolveProjectKey', () => {
  it('returns the normalized key when the map has no entry yet', () => {
    expect(resolveProjectKey({}, 'C:\\Users\\me\\repo')).toBe('C:/Users/me/repo');
    expect(resolveProjectKey(undefined, 'C:\\Users\\me\\repo')).toBe('C:/Users/me/repo');
  });

  it('prefers an exact hit', () => {
    const map = { 'C:/Users/me/repo': [], 'c:/users/me/repo': [] };
    expect(resolveProjectKey(map, 'C:/Users/me/repo')).toBe('C:/Users/me/repo');
  });

  // The real bug this exists for: the app writes agent cwds lowercase
  // ('c:/users/…') while a seeded or picker-sourced path can be mixed case.
  // On Windows those are one directory, so the board must not split in two.
  it('adopts an existing case-only variant on a case-insensitive filesystem', () => {
    const map = { 'c:/users/damientouchette/work/leroy': [{ widget: 'git', size: 'small' }] };
    const got = resolveProjectKey(map, 'C:\\Users\\DamienTouchette\\work\\leroy');
    // Only meaningful on win32/darwin; on Linux the two paths are different dirs.
    if (/win|mac/i.test(globalThis.navigator?.platform ?? '')) {
      expect(got).toBe('c:/users/damientouchette/work/leroy');
    } else {
      expect(got).toBe('C:/Users/DamienTouchette/work/leroy');
    }
  });

  it('never merges two genuinely different directories', () => {
    const map = { '/home/me/alpha': [] };
    expect(resolveProjectKey(map, '/home/me/beta')).toBe('/home/me/beta');
  });
});
