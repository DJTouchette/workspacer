import { describe, it, expect, vi, beforeEach } from 'vitest';
import { act, renderHook, waitFor } from '@testing-library/react';
import { useSessionArchive } from '../src/hooks/useSessionArchive';
import type { SessionArchiveDoc } from '../src/types/electron';

/**
 * The sidebar's view of the hub's shared archive: a fake bus that behaves
 * like `sessionArchive.get/set` + `sessionArchive.changed`. No real hub, no
 * provider, no lifecycle method — the fake exposes none, so any attempt to
 * stop or close a session here would throw.
 */
type Listener<T> = (value: T) => void;

function fakeHub(initial: SessionArchiveDoc) {
  let doc = initial;
  const changed = new Set<Listener<SessionArchiveDoc>>();
  const status = new Set<Listener<{ connected: boolean }>>();
  const api = {
    sessionArchiveGet: vi.fn(async () => doc),
    sessionArchiveSet: vi.fn(async (id: string, archived: boolean) => {
      const next = { ...doc.archived };
      if (archived) next[id] = 1;
      else delete next[id];
      doc = { version: doc.version + 1, archived: next };
      return doc;
    }),
    onSessionArchiveChanged: vi.fn((cb: Listener<SessionArchiveDoc>) => {
      changed.add(cb);
      return () => changed.delete(cb);
    }),
    onHubStatus: vi.fn((cb: Listener<{ connected: boolean }>) => {
      status.add(cb);
      return () => status.delete(cb);
    }),
  };
  return {
    api,
    /** Another client (e.g. native) changed the archive. */
    emit(next: SessionArchiveDoc) {
      doc = next;
      for (const cb of changed) cb(next);
    },
    /** Replace the hub's document without an event (changes while offline). */
    replace(next: SessionArchiveDoc) {
      doc = next;
    },
    setConnected(connected: boolean) {
      for (const cb of status) cb({ connected });
    },
  };
}

let hub: ReturnType<typeof fakeHub>;
beforeEach(() => {
  hub = fakeHub({ version: 1, archived: { a: 1 } });
  (window as any).electronAPI = hub.api;
});

describe('useSessionArchive', () => {
  it('reads the hub archive and follows native-shaped change events', async () => {
    const { result } = renderHook(() => useSessionArchive());
    await waitFor(() => expect(result.current.available).toBe(true));
    expect([...result.current.archived]).toEqual(['a']);

    act(() => hub.emit({ version: 2, archived: { a: 1, b: 2 } }));
    expect(result.current.archived.has('b')).toBe(true);

    // A late, older document never undoes a newer one.
    act(() => hub.emit({ version: 1, archived: {} }));
    expect([...result.current.archived].sort()).toEqual(['a', 'b']);
  });

  it('archives and restores through the hub, showing the change at once', async () => {
    const { result } = renderHook(() => useSessionArchive());
    await waitFor(() => expect(result.current.available).toBe(true));

    let done!: Promise<void>;
    act(() => {
      done = result.current.setArchived('c', true);
    });
    expect(result.current.archived.has('c')).toBe(true); // optimistic
    await act(() => done);
    expect(hub.api.sessionArchiveSet).toHaveBeenCalledWith('c', true);
    expect(result.current.archived.has('c')).toBe(true); // confirmed

    await act(() => result.current.setArchived('a', false));
    expect(result.current.archived.has('a')).toBe(false);
  });

  it('rolls back and rejects when the hub refuses', async () => {
    const { result } = renderHook(() => useSessionArchive());
    await waitFor(() => expect(result.current.available).toBe(true));
    hub.api.sessionArchiveSet.mockRejectedValueOnce(new Error('denied'));
    let failure: unknown;
    await act(async () => {
      await result.current.setArchived('z', true).catch((e) => (failure = e));
    });
    expect(String(failure)).toContain('denied');
    expect(result.current.archived.has('z')).toBe(false);
  });

  it('re-reads after a reconnect, even when the hub’s archive was reset', async () => {
    const { result } = renderHook(() => useSessionArchive());
    await waitFor(() => expect(result.current.available).toBe(true));
    act(() => hub.setConnected(false));
    // Changed elsewhere while this client was away — no event will replay it.
    hub.replace({ version: 0, archived: { fresh: 5 } });
    act(() => hub.setConnected(true));
    await waitFor(() => expect([...result.current.archived]).toEqual(['fresh']));
    expect(hub.api.sessionArchiveGet).toHaveBeenCalledTimes(2);
  });

  it('retries on the first connect when the boot-time read could not get out', async () => {
    hub.api.sessionArchiveGet.mockRejectedValueOnce(new Error('Hub disconnected'));
    const { result } = renderHook(() => useSessionArchive());
    await waitFor(() => expect(hub.api.sessionArchiveGet).toHaveBeenCalledTimes(1));
    expect(result.current.available).toBe(false);
    act(() => hub.setConnected(true));
    await waitFor(() => expect(result.current.available).toBe(true));
    expect([...result.current.archived]).toEqual(['a']);
  });

  it('a hub without shared archives leaves everything visible and unavailable', async () => {
    hub.api.sessionArchiveGet.mockRejectedValue(new Error('no provider for sessionArchive.get'));
    const { result } = renderHook(() => useSessionArchive());
    await waitFor(() => expect(hub.api.sessionArchiveGet).toHaveBeenCalled());
    expect(result.current.available).toBe(false);
    expect(result.current.archived.size).toBe(0);
  });

  it('does nothing while disabled (before the session is active)', () => {
    renderHook(() => useSessionArchive(false));
    expect(hub.api.sessionArchiveGet).not.toHaveBeenCalled();
  });
});

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

describe('archive operation fencing', () => {
  it('ignores a pre-disconnect read arriving before the reset-version reconnect read', async () => {
    const old = deferred<SessionArchiveDoc>();
    const fresh = deferred<SessionArchiveDoc>();
    hub.api.sessionArchiveGet.mockReturnValueOnce(old.promise).mockReturnValueOnce(fresh.promise);
    const { result } = renderHook(() => useSessionArchive());
    act(() => {
      hub.setConnected(false);
      hub.setConnected(true);
    });
    await act(async () => old.resolve({ version: 90, archived: { stale: 1 } }));
    await act(async () => fresh.resolve({ version: 0, archived: { fresh: 1 } }));
    expect([...result.current.archived]).toEqual(['fresh']);
  });
  it('only the last same-value operation can settle archive/restore/archive', async () => {
    const { result } = renderHook(() => useSessionArchive());
    await waitFor(() => expect(result.current.available).toBe(true));
    const first = deferred<SessionArchiveDoc>();
    const second = deferred<SessionArchiveDoc>();
    const third = deferred<SessionArchiveDoc>();
    hub.api.sessionArchiveSet
      .mockReturnValueOnce(first.promise)
      .mockReturnValueOnce(second.promise)
      .mockReturnValueOnce(third.promise);
    let a!: Promise<void>, b!: Promise<void>, c!: Promise<void>;
    act(() => {
      a = result.current.setArchived('z', true);
      b = result.current.setArchived('z', false);
      c = result.current.setArchived('z', true);
    });
    await act(async () => {
      first.resolve({ version: 2, archived: { z: 1 } });
      await a;
    });
    await act(async () => {
      second.resolve({ version: 3, archived: {} });
      await b;
    });
    expect(result.current.archived.has('z')).toBe(true);
    await act(async () => {
      third.resolve({ version: 4, archived: { z: 1 } });
      await c;
    });
    expect(result.current.archived.has('z')).toBe(true);
  });
});

describe('archive hydration', () => {
  it('keeps first paint pending until shared visibility is known', async () => {
    const first = deferred<SessionArchiveDoc>();
    hub.api.sessionArchiveGet.mockReturnValueOnce(first.promise);
    const { result } = renderHook(() => useSessionArchive());
    expect(result.current.ready).toBe(false);
    await act(async () => first.resolve({ version: 1, archived: { hidden: 1 } }));
    expect(result.current.ready).toBe(true);
    expect(result.current.archived.has('hidden')).toBe(true);
  });
  it('lets an authoritative reconnect event reset versions before its read returns', async () => {
    const { result } = renderHook(() => useSessionArchive());
    await waitFor(() => expect(result.current.available).toBe(true));
    const fresh = deferred<SessionArchiveDoc>();
    hub.api.sessionArchiveGet.mockReturnValueOnce(fresh.promise);
    act(() => {
      hub.setConnected(false);
      hub.setConnected(true);
      hub.emit({ version: 0, archived: { reset: 1 } });
    });
    await act(async () => fresh.resolve({ version: 0, archived: {} }));
    expect([...result.current.archived]).toEqual(['reset']);
  });
});
