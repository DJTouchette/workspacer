import { describe, it, expect } from 'vitest';
import { filterSlashItems, type SlashItem } from './slashItems';

const items: SlashItem[] = [
  { id: 'a', label: 'review', hint: 'code review', kind: 'skill' },
  { id: 'b', label: 'refactor', hint: 'clean up code', kind: 'skill' },
  { id: 'c', label: 'commit', hint: 'write a message', kind: 'prompt' },
  { id: 'd', label: 'preview', hint: 'open a review of the build', kind: 'prompt' },
];

describe('filterSlashItems', () => {
  it('returns the head of the list (capped) for an empty query', () => {
    expect(filterSlashItems(items, '').map((i) => i.id)).toEqual(['a', 'b', 'c', 'd']);
    expect(filterSlashItems(items, '  ', 2).map((i) => i.id)).toEqual(['a', 'b']);
  });

  it('is case-insensitive', () => {
    expect(filterSlashItems(items, 'REV').map((i) => i.id)).toContain('a');
  });

  it('ranks label prefix matches ahead of other substring matches', () => {
    // "re": prefix → review, refactor; substring → preview (label) has "re" too,
    // but not as a prefix, so it ranks after the prefix matches.
    expect(filterSlashItems(items, 're').map((i) => i.id)).toEqual(['a', 'b', 'd']);
  });

  it('matches the hint when the label does not', () => {
    // "message" only appears in commit's hint.
    expect(filterSlashItems(items, 'message').map((i) => i.id)).toEqual(['c']);
  });

  it('excludes non-matches', () => {
    expect(filterSlashItems(items, 'zzz')).toEqual([]);
  });

  it('caps the result length', () => {
    const many: SlashItem[] = Array.from({ length: 20 }, (_, i) => ({
      id: `x${i}`,
      label: `skill${i}`,
    }));
    expect(filterSlashItems(many, 'skill', 8)).toHaveLength(8);
  });
});
