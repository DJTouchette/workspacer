import { describe, it, expect } from 'vitest';
import { isUnmergedStatus } from '../src/lib/gitQueries';

describe('isUnmergedStatus', () => {
  it.each(['DD', 'AU', 'UD', 'UA', 'DU', 'AA', 'UU'])('treats %s as an unmerged conflict', (xy) => {
    expect(isUnmergedStatus({ staged: xy[0], unstaged: xy[1] })).toBe(true);
  });

  it('does not treat ordinary staged or working-tree changes as conflicts', () => {
    expect(isUnmergedStatus({ staged: 'M', unstaged: ' ' })).toBe(false);
    expect(isUnmergedStatus({ staged: ' ', unstaged: 'M' })).toBe(false);
    expect(isUnmergedStatus({ staged: '?', unstaged: '?' })).toBe(false);
  });
});
