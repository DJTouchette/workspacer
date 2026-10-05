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
  /** First archive read has settled; avoid briefly showing archived rows. */
  ready: boolean;
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
  const [ready, setReady] = useState(false);
  const [pending, setPending] = useState<ReadonlyMap<string, { archived: boolean; op: number }>>(
    new Map(),
  );
  const operation = useRef(0);
  const connectionEpoch = useRef(0);
  const versionRef = useRef(-1);

  const apply = useCallback((next: unknown, force = false) => {
    if (!isDoc(next)) return;
    if (!force && next.version <= versionRef.current) return;
    versionRef.current = next.version;
    setDoc(next);
  }, []);

  useEffect(() => {
    const api = window.electronAPI;
    if (!enabled || !api.sessionArchiveGet) {
      setReady(true);
      return;
    }
    let alive = true;
    let loaded = false;
    let readSequence = 0;
    let eventSequence = 0;
    const read = () => {
      const seq = ++readSequence;
      const eventsAtRead = eventSequence;
      api.sessionArchiveGet!()
        .then((next) => {
          if (!alive || seq !== readSequence) return;
          apply(next, eventSequence === eventsAtRead);
          loaded = true;
        })
        // Old hubs show all sessions after refusal; reconnect tries again.
        .catch(() => {})
        .finally(() => {
          if (alive && seq === readSequence) setReady(true);
        });
    };
    const offChanged = api.onSessionArchiveChanged?.((next) => {
      ++eventSequence;
      apply(next, !loaded);
      loaded = true;
      setReady(true);
    });
    read();
    let connected = true;
    const offStatus = api.onHubStatus?.((status) => {
      if (!status.connected) {
        ++readSequence;
        ++connectionEpoch.current;
        loaded = false;
        setPending(new Map());
      }
      if (status.connected && (!connected || !loaded)) read();
      connected = status.connected;
    });
    return () => {
      alive = false;
      ++readSequence;
      offChanged?.();
      offStatus?.();
    };
  }, [enabled, apply]);

  const setArchived = useCallback(
    async (sessionId: string, archived: boolean) => {
      const api = window.electronAPI;
      if (!api.sessionArchiveSet) throw new Error('This hub cannot archive sessions');
      const op = ++operation.current;
      const epoch = connectionEpoch.current;
      setPending((prev) => new Map(prev).set(sessionId, { archived, op }));
      const settle = () =>
        setPending((prev) => {
          if (prev.get(sessionId)?.op !== op) return prev; // a newer click owns it
          const next = new Map(prev);
          next.delete(sessionId);
          return next;
        });
      try {
        const next = await api.sessionArchiveSet(sessionId, archived);
        if (epoch === connectionEpoch.current) apply(next);
      } finally {
        settle();
      }
    },
    [apply],
  );

  const archived = useMemo(() => {
    const ids = new Set(doc ? Object.keys(doc.archived) : []);
    for (const [id, value] of pending) {
      if (value.archived) ids.add(id);
      else ids.delete(id);
    }
    return ids;
  }, [doc, pending]);

  return { archived, available: doc !== null, ready, setArchived };
}
