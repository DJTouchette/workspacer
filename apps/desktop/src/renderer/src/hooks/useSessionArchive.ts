/**
 * The hub's shared session archive, as the sidebar sees it.
 *
 * Archiving is view state the hub owns (`sessionArchive.get/set`, broadcast as
 * `sessionArchive.changed`), so a session archived in the native client — or
 * another browser — disappears from this sidebar too, and a reload or
 * reconnect re-reads it rather than trusting what this tab remembered.
 *
 * It is ONLY visibility: nothing here stops, signals, closes or forgets a
 * session, and a running archived session keeps running.
 *
 * A change made here shows immediately and is confirmed by the hub's reply; a
 * refused change rolls back and rejects so the caller can say so. Documents
 * are versioned: an older one arriving late (a reply racing the event it
 * caused) never undoes a newer one. The first read after each (re)connect is
 * taken as-is, so a hub whose archive was reset is still believed.
 */
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { SessionArchiveDoc } from '../types/electron';

export interface SessionArchiveState {
  /** Archived session ids, including changes still on their way to the hub. */
  archived: ReadonlySet<string>;
  /** The hub answered: archive/restore is possible. False for a hub that
   *  predates shared archives (the sidebar then shows everything, no action). */
  available: boolean;
  /** Archive (true) or restore (false). Rejects — after rolling back — when
   *  the hub refuses or is unreachable. */
  setArchived: (sessionId: string, archived: boolean) => Promise<void>;
}

function isDoc(value: unknown): value is SessionArchiveDoc {
  const v = value as SessionArchiveDoc | null;
  return (
    !!v &&
    typeof v.version === 'number' &&
    !!v.archived &&
    typeof v.archived === 'object' &&
    !Array.isArray(v.archived)
  );
}

export function useSessionArchive(enabled = true): SessionArchiveState {
  const [doc, setDoc] = useState<SessionArchiveDoc | null>(null);
  const [pending, setPending] = useState<ReadonlyMap<string, boolean>>(new Map());
  const versionRef = useRef(-1);
  /** Accept the next read unconditionally (set on every (re)connect). */
  const resyncRef = useRef(true);

  const apply = useCallback((next: unknown, force = false) => {
    if (!isDoc(next)) return;
    if (!force && next.version <= versionRef.current) return;
    versionRef.current = next.version;
    setDoc(next);
  }, []);

  useEffect(() => {
    if (!enabled) return;
    const api = window.electronAPI;
    if (!api.sessionArchiveGet) return;
    let alive = true;
    let loaded = false;
    const read = () => {
      resyncRef.current = true;
      api
        .sessionArchiveGet?.()
        .then((next) => {
          if (!alive) return;
          apply(next, resyncRef.current);
          resyncRef.current = false;
          loaded = true;
        })
        // An older hub has no provider for it, or the socket is down: keep
        // the last known archive (or none) and try again on reconnect.
        .catch(() => {});
    };
    read();
    const offChanged = api.onSessionArchiveChanged?.((next) => {
      resyncRef.current = false;
      apply(next);
    });
    let connected = true;
    const offStatus = api.onHubStatus?.((status) => {
      // Every reconnect re-reads (changes made meanwhile have no replay), and
      // so does the first connect when the boot-time read could not get out.
      if (status.connected && (!connected || !loaded)) read();
      connected = status.connected;
    });
    return () => {
      alive = false;
      offChanged?.();
      offStatus?.();
    };
  }, [enabled, apply]);

  const setArchived = useCallback(
    async (sessionId: string, archived: boolean) => {
      const api = window.electronAPI;
      if (!api.sessionArchiveSet) throw new Error('This hub cannot archive sessions');
      setPending((prev) => new Map(prev).set(sessionId, archived));
      const settle = () =>
        setPending((prev) => {
          if (prev.get(sessionId) !== archived) return prev; // a newer click owns it
          const next = new Map(prev);
          next.delete(sessionId);
          return next;
        });
      try {
        apply(await api.sessionArchiveSet(sessionId, archived));
      } finally {
        settle();
      }
    },
    [apply],
  );

  const archived = useMemo(() => {
    const ids = new Set(doc ? Object.keys(doc.archived) : []);
    for (const [id, value] of pending) {
      if (value) ids.add(id);
      else ids.delete(id);
    }
    return ids;
  }, [doc, pending]);

  return { archived, available: doc !== null, setArchived };
}
