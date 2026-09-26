import { beforeEach, describe, expect, it, vi } from 'vitest';
import {
  CHAT_READING_STORAGE,
  firstUnreadTurn,
  loadChatReadingPosition,
  readThrough,
  saveChatReadingPosition,
} from './chatReadingPosition';
import type { ConversationTurn } from '../types/claudeSession';

const turns: ConversationTurn[] = [
  { role: 'user', content: 'Hello', timestamp: 1 },
  { role: 'assistant', content: 'Working', timestamp: 2 },
];
beforeEach(() => {
  localStorage.clear();
  vi.restoreAllMocks();
});
describe('conversation reading bookmarks', () => {
  it('detects appended turns and streaming growth without confusing offsets', () => {
    const seen = readThrough(turns, 100);
    expect(firstUnreadTurn(seen, turns, 100)).toBeNull();
    expect(firstUnreadTurn(seen, [...turns, { ...turns[1], timestamp: 3 }], 100)).toBe(102);
    expect(firstUnreadTurn(seen, [turns[0], { ...turns[1], content: 'Finished' }], 100)).toBe(101);
    expect(firstUnreadTurn(seen, [turns[1]], 101)).toBeNull();
    expect(firstUnreadTurn(seen, [{ ...turns[1], timestamp: 3 }], 110)).toBe(110);
  });
  it('does not misidentify a replaced transcript or a first visit as unread', () => {
    expect(firstUnreadTurn(undefined, turns, 0)).toBeNull();
    expect(firstUnreadTurn(readThrough(turns, 0), [turns[0]], 0)).toBeNull();
    expect(
      firstUnreadTurn(readThrough(turns, 0), [turns[0], { ...turns[1], timestamp: 9 }], 0),
    ).toBeNull();
  });
  it('recognizes tool completion within an existing reply', () => {
    const turn = {
      ...turns[1],
      toolCalls: [{ id: 'a', name: 'Read', input: {}, status: 'running' as const, startedAt: 2 }],
    };
    expect(
      firstUnreadTurn(
        readThrough([turn], 0),
        [{ ...turn, toolCalls: [{ ...turn.toolCalls[0], status: 'complete' }] }],
        0,
      ),
    ).toBe(0);
  });
  it('bounds stored sessions, isolates keys, and never saves conversation text', () => {
    for (let i = 0; i < 150; i++)
      saveChatReadingPosition(`host:${i}`, {
        anchor: 'msg-1',
        offset: -40,
        atBottom: false,
        readThrough: readThrough(turns, 0),
        updatedAt: Date.now() + i,
      });
    expect(Object.keys(JSON.parse(localStorage.getItem(CHAT_READING_STORAGE)!))).toHaveLength(128);
    expect(loadChatReadingPosition('host:0')).toBeUndefined();
    expect(loadChatReadingPosition('host:149')?.offset).toBe(-40);
    expect(loadChatReadingPosition('other:149')).toBeUndefined();
    expect(localStorage.getItem(CHAT_READING_STORAGE)).not.toContain('Working');
  });
  it('tolerates malformed and unavailable storage', () => {
    localStorage.setItem(CHAT_READING_STORAGE, '{bad');
    expect(loadChatReadingPosition('s')).toBeUndefined();
    localStorage.setItem(
      CHAT_READING_STORAGE,
      JSON.stringify({ s: { anchor: 'msg-1', offset: 'oops' } }),
    );
    expect(loadChatReadingPosition('s')).toBeUndefined();
    vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new Error('full');
    });
    expect(() =>
      saveChatReadingPosition('s', {
        anchor: 'msg-1',
        offset: 0,
        atBottom: true,
        updatedAt: Date.now(),
      }),
    ).not.toThrow();
  });
});
