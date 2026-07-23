/**
 * Pure-state contract of the in-app notification center (lib/notificationStore).
 *
 * Pins the behaviours the UI leans on: loose external input (plugins over the
 * bus) normalizes or is rejected rather than crashing the provider; same-`key`
 * notifications replace instead of stacking (the "agent needs-you" slot);
 * history is capped; corrupt localStorage restores to empty, never throws.
 */

import { describe, it, expect } from 'vitest';
import {
  ingest,
  loadPersisted,
  markAllRead,
  markRead,
  normalizeNotification,
  persist,
  removeItem,
  shouldEscalate,
  timeAgo,
  unreadCount,
  MAX_ITEMS,
  PERSIST_LIMIT,
  STORAGE_KEY,
  type StoredNotification,
} from './notificationStore';

function make(over: Partial<StoredNotification> = {}): StoredNotification {
  return {
    id: over.id ?? Math.random().toString(36).slice(2),
    level: 'info',
    title: 't',
    source: 'test',
    createdAt: 1,
    read: false,
    ...over,
  };
}

describe('normalizeNotification', () => {
  it('rejects input without a displayable title', () => {
    expect(normalizeNotification(undefined, 'x')).toBeNull();
    expect(normalizeNotification({}, 'x')).toBeNull();
    expect(normalizeNotification({ title: '   ' }, 'x')).toBeNull();
    expect(normalizeNotification({ title: 42 as unknown as string }, 'x')).toBeNull();
  });

  it('clamps unknown levels to info and keeps known ones', () => {
    expect(normalizeNotification({ title: 'a', level: 'catastrophic' }, 'x')!.level).toBe('info');
    expect(normalizeNotification({ title: 'a', level: 'error' }, 'x')!.level).toBe('error');
  });

  it('applies the fallback source only when the input has none', () => {
    expect(normalizeNotification({ title: 'a' }, 'plugin:ci')!.source).toBe('plugin:ci');
    expect(normalizeNotification({ title: 'a', source: 'me' }, 'plugin:ci')!.source).toBe('me');
  });

  it('truncates oversized titles and bodies', () => {
    const n = normalizeNotification({ title: 'T'.repeat(500), body: 'B'.repeat(2000) }, 'x')!;
    expect(n.title.length).toBeLessThanOrEqual(200);
    expect(n.body!.length).toBeLessThanOrEqual(600);
  });

  it('generates unique ids when none is provided', () => {
    const a = normalizeNotification({ title: 'a' }, 'x')!;
    const b = normalizeNotification({ title: 'b' }, 'x')!;
    expect(a.id).not.toBe(b.id);
  });
});

describe('ingest', () => {
  it('prepends newest-first and marks unread', () => {
    const items = ingest([make({ id: 'old' })], make({ id: 'new' }));
    expect(items.map((i) => i.id)).toEqual(['new', 'old']);
    expect(items[0].read).toBe(false);
  });

  it('drops same-id redelivery (bus reconnect replays)', () => {
    const start = [make({ id: 'a' })];
    expect(ingest(start, make({ id: 'a' }))).toBe(start);
  });

  it('replaces same-key entries instead of stacking', () => {
    const first = ingest([], make({ id: '1', key: 'agent:s:needs-you', title: 'old prompt' }));
    const second = ingest(first, make({ id: '2', key: 'agent:s:needs-you', title: 'new prompt' }));
    expect(second).toHaveLength(1);
    expect(second[0].title).toBe('new prompt');
  });

  it('caps history at MAX_ITEMS', () => {
    let items: StoredNotification[] = [];
    for (let i = 0; i < MAX_ITEMS + 10; i++) items = ingest(items, make({ id: `n${i}` }));
    expect(items).toHaveLength(MAX_ITEMS);
    expect(items[0].id).toBe(`n${MAX_ITEMS + 9}`);
  });
});

describe('read state', () => {
  it('counts, marks one, marks all', () => {
    let items = [make({ id: 'a' }), make({ id: 'b' }), make({ id: 'c', read: true })];
    expect(unreadCount(items)).toBe(2);
    items = markRead(items, 'a');
    expect(unreadCount(items)).toBe(1);
    items = markAllRead(items);
    expect(unreadCount(items)).toBe(0);
  });

  it('markAllRead is identity when nothing is unread (no spurious rerenders)', () => {
    const items = [make({ read: true })];
    expect(markAllRead(items)).toBe(items);
  });

  it('removeItem drops exactly the id', () => {
    const items = [make({ id: 'a' }), make({ id: 'b' })];
    expect(removeItem(items, 'a').map((i) => i.id)).toEqual(['b']);
  });
});

describe('persistence', () => {
  function memStorage(initial: Record<string, string> = {}) {
    const map = new Map(Object.entries(initial));
    return {
      getItem: (k: string) => map.get(k) ?? null,
      setItem: (k: string, v: string) => void map.set(k, v),
      dump: () => map,
    };
  }

  it('round-trips items through storage', () => {
    const storage = memStorage();
    const items = [make({ id: 'a', read: true }), make({ id: 'b' })];
    persist(storage, items);
    const restored = loadPersisted(storage);
    expect(restored.map((i) => i.id)).toEqual(['a', 'b']);
    expect(restored[0].read).toBe(true);
    expect(restored[1].read).toBe(false);
  });

  it('persists at most PERSIST_LIMIT entries', () => {
    const storage = memStorage();
    persist(
      storage,
      Array.from({ length: PERSIST_LIMIT + 50 }, (_, i) => make({ id: `n${i}` })),
    );
    expect(loadPersisted(storage)).toHaveLength(PERSIST_LIMIT);
  });

  it('restores to empty on corrupt or wrong-shape payloads', () => {
    expect(loadPersisted(memStorage({ [STORAGE_KEY]: 'not json{' }))).toEqual([]);
    expect(loadPersisted(memStorage({ [STORAGE_KEY]: '{"a":1}' }))).toEqual([]);
    expect(
      loadPersisted(memStorage({ [STORAGE_KEY]: JSON.stringify([{ bogus: true }, null]) })),
    ).toEqual([]);
  });

  it('drops invalid entries but keeps valid ones', () => {
    const storage = memStorage({
      [STORAGE_KEY]: JSON.stringify([make({ id: 'ok' }), { title: 'no id' }]),
    });
    expect(loadPersisted(storage).map((i) => i.id)).toEqual(['ok']);
  });
});

describe('shouldEscalate — renderer-only, unfocused, non-silent', () => {
  it('escalates a renderer-originated notification when the window is unfocused', () => {
    expect(shouldEscalate(make(), 'renderer', false)).toBe(true);
  });

  it('never escalates main-originated notifications (they made their own OS decision)', () => {
    expect(shouldEscalate(make(), 'main', false)).toBe(false);
  });

  it('never escalates while the window is focused (the toast is visible)', () => {
    expect(shouldEscalate(make(), 'renderer', true)).toBe(false);
  });

  it('silent means silent on every surface', () => {
    expect(shouldEscalate(make({ silent: true }), 'renderer', false)).toBe(false);
  });
});

describe('timeAgo', () => {
  it('buckets seconds/minutes/hours/days', () => {
    const now = 1_000_000_000_000;
    expect(timeAgo(now - 5_000, now)).toBe('now');
    expect(timeAgo(now - 5 * 60_000, now)).toBe('5m');
    expect(timeAgo(now - 3 * 3_600_000, now)).toBe('3h');
    expect(timeAgo(now - 2 * 86_400_000, now)).toBe('2d');
  });
});
