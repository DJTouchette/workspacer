/**
 * The hub's shared sidebar (`sessionOpen`, docs/session-roster.md), as this
 * renderer sees it: which sessions every client lists, which of those ended on
 * purpose, and the hub-derived "too many sessions" cleanup prompt.
 *
 * The hub owns and maintains the document; this hook only follows it. It is
 * read on mount and after every (re)connect (changes made while away have no
 * replay) and follows `sessionOpen.changed`. Documents are versioned like the
 * archive's (see useSessionArchive): an older one arriving late never undoes a
 * newer one, and the first read after each (re)connect is taken as-is, so a
 * hub whose document was reset is still believed.
 *
 * An older hub without `sessionOpen` leaves `doc` null: no Paused/Ended
 * labels and no cleanup prompt.
 */
import { useCallback, useEffect, useRef, useState } from 'react';
import type { SessionOpenDoc } from '../types/electron';

function isMap(value: unknown): value is Record<string, number> {
  return !!value && typeof value === 'object' && !Array.isArray(value);
}

export function parseSessionOpenDoc(value: unknown): SessionOpenDoc | null {
  const v = value as Partial<SessionOpenDoc> | null;
  if (!v || typeof v.version !== 'number' || !isMap(v.open)) return null;
  const c = v.cleanup as Partial<NonNullable<SessionOpenDoc['cleanup']>> | undefined;
  const cleanup =
    c &&
    typeof c.ended === 'number' &&
    typeof c.threshold === 'number' &&
    typeof c.suggested === 'boolean'
      ? {
          ended: c.ended,
          threshold: c.threshold,
          suggested: c.suggested,
          snoozedAt: typeof c.snoozedAt === 'number' ? c.snoozedAt : null,
        }
      : undefined;
  return {
    version: v.version,
    open: v.open,
    ended: isMap(v.ended) ? v.ended : {},
    ...(cleanup ? { cleanup } : {}),
  };
}

/** How a stopped session reads on the sidebar, per the shared document. */
export type StoppedKind = 'paused' | 'ended' | null;

/** Paused = stopped ∧ open ∧ ¬ended; Ended = stopped ∧ open ∧ ended. Null when
 *  the session is not on the shared sidebar (or there is no document). */
export function stoppedKind(
  doc: SessionOpenDoc | null,
  sessionId: string | undefined,
): StoppedKind {
  if (!doc || !sessionId || !(sessionId in doc.open)) return null;
  return sessionId in doc.ended ? 'ended' : 'paused';
}

export interface SessionOpenState {
  /** The latest document, or null before the first read / on an old hub. */
  doc: SessionOpenDoc | null;
  /** Adopt a document a call returned (e.g. snoozeCleanup), version-fenced. */
  adopt: (next: unknown) => void;
}

export function useSessionOpen(enabled = true): SessionOpenState {
  const [doc, setDoc] = useState<SessionOpenDoc | null>(null);
  const versionRef = useRef(-1);

  const apply = useCallback((next: unknown, force = false) => {
    const parsed = parseSessionOpenDoc(next);
    if (!parsed) return;
    if (!force && parsed.version <= versionRef.current) return;
    versionRef.current = parsed.version;
    setDoc(parsed);
  }, []);

  useEffect(() => {
    const api = window.electronAPI;
    if (!enabled || !api.sessionOpenGet) return;
    let alive = true;
    let loaded = false;
    let readSequence = 0;
    let eventSequence = 0;
    const read = () => {
      const seq = ++readSequence;
      const eventsAtRead = eventSequence;
      api.sessionOpenGet!()
        .then((next) => {
          if (!alive || seq !== readSequence) return;
          // Forced only when no event raced this read.
          apply(next, eventSequence === eventsAtRead);
          loaded = true;
        })
        // Old hub or not connected yet: no labels/prompt; reconnect retries.
        .catch(() => {});
    };
    const offChanged = api.onSessionOpenChanged?.((next) => {
      ++eventSequence;
      apply(next, !loaded);
      loaded = true;
    });
    read();
    let connected = true;
    const offStatus = api.onHubStatus?.((status) => {
      if (!status.connected) {
        ++readSequence;
        loaded = false;
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

  const adopt = useCallback((next: unknown) => apply(next), [apply]);
  return { doc, adopt };
}
