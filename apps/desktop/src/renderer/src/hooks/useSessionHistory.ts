/**
 * Pages through permanent session history (`sessions.history`,
 * docs/session-roster.md §5): every session claudemon ever saw, newest first,
 * with server-side search.
 *
 * - `available` is false when this backend has no `listSessionHistory` (an
 *   older preload) or its hub has no provider. The History pane then falls back
 *   to the bounded `sessions.recent` list. A rejection is reported as
 *   `unavailable`, never as an empty history.
 * - Changing `query` restarts from the first page after a short debounce. A
 *   reply for an older query or generation is dropped.
 * - `remove`/`patch` update the held rows in place after a forget/unhide, so
 *   the list does not jump back to page one.
 */
import { useCallback, useEffect, useRef, useState } from 'react';
import type { HistoryAgentSession } from '../../../main/shared/ipcTypes';

export const HISTORY_PAGE_SIZE = 50;
const QUERY_DEBOUNCE_MS = 250;

export interface SessionHistoryState {
  /** False when there is no history method: callers use their fallback. */
  available: boolean;
  rows: HistoryAgentSession[];
  /** True while a page is in flight. */
  loading: boolean;
  /** More (older) rows exist. */
  hasMore: boolean;
  /** Why history could not be read, or null. */
  unavailable: string | null;
  /** The query the held rows answer ('' = unfiltered). */
  answeredQuery: string;
  loadMore: () => void;
  /** Re-read from the first page (keeps the query). */
  reload: () => void;
  remove: (sessionId: string) => void;
  patch: (sessionId: string, change: Partial<HistoryAgentSession>) => void;
}

export function useSessionHistory(query: string, enabled = true): SessionHistoryState {
  const api = typeof window !== 'undefined' ? window.electronAPI : undefined;
  const available = enabled && typeof api?.listSessionHistory === 'function';
  const [rows, setRows] = useState<HistoryAgentSession[]>([]);
  const [next, setNext] = useState<string | undefined>(undefined);
  const [loading, setLoading] = useState(false);
  const [unavailable, setUnavailable] = useState<string | null>(null);
  const [answeredQuery, setAnsweredQuery] = useState('');
  const [debounced, setDebounced] = useState(query.trim());
  const [generation, setGeneration] = useState(0);
  /** Bumped per first-page read; stale replies compare against it. */
  const epoch = useRef(0);
  const inflight = useRef(false);

  useEffect(() => {
    const q = query.trim();
    if (q === debounced) return;
    const t = setTimeout(() => setDebounced(q), QUERY_DEBOUNCE_MS);
    return () => clearTimeout(t);
  }, [query, debounced]);

  const fetchPage = useCallback(
    (cursor: string | undefined, q: string, mine: number) => {
      if (!available || !api?.listSessionHistory) return;
      inflight.current = true;
      setLoading(true);
      api
        .listSessionHistory({
          limit: HISTORY_PAGE_SIZE,
          ...(cursor ? { cursor } : {}),
          ...(q ? { query: q } : {}),
        })
        .then((page) => {
          if (mine !== epoch.current) return;
          const got = Array.isArray(page?.sessions) ? page.sessions : [];
          setRows((held) => {
            if (!cursor) return got;
            const seen = new Set(held.map((r) => r.sessionId));
            return [...held, ...got.filter((r) => !seen.has(r.sessionId))];
          });
          setNext(typeof page?.next === 'string' && page.next ? page.next : undefined);
          setAnsweredQuery(q);
          setUnavailable(null);
        })
        .catch((err: unknown) => {
          if (mine !== epoch.current) return;
          setUnavailable(err instanceof Error ? err.message : String(err));
        })
        .finally(() => {
          if (mine !== epoch.current) return;
          inflight.current = false;
          setLoading(false);
        });
    },
    [api, available],
  );

  useEffect(() => {
    if (!available) return;
    const mine = ++epoch.current;
    inflight.current = false;
    fetchPage(undefined, debounced, mine);
  }, [available, debounced, generation, fetchPage]);

  const loadMore = useCallback(() => {
    if (!next || inflight.current) return;
    fetchPage(next, answeredQuery, epoch.current);
  }, [next, answeredQuery, fetchPage]);

  const reload = useCallback(() => setGeneration((g) => g + 1), []);
  const remove = useCallback(
    (sessionId: string) => setRows((held) => held.filter((r) => r.sessionId !== sessionId)),
    [],
  );
  const patch = useCallback(
    (sessionId: string, change: Partial<HistoryAgentSession>) =>
      setRows((held) => held.map((r) => (r.sessionId === sessionId ? { ...r, ...change } : r))),
    [],
  );

  return {
    available,
    rows,
    loading,
    hasMore: !!next,
    unavailable,
    answeredQuery,
    loadMore,
    reload,
    remove,
    patch,
  };
}
