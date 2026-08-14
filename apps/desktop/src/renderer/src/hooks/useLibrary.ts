import { useState, useEffect, useCallback } from 'react';
import type {
  ClaudeOrigin,
  LibraryItem,
  LibrarySaveInput,
  LibraryScope,
  LibraryKind,
} from '../types/library';

/**
 * Loads the merged library (global + project for `cwd`) and keeps it live by
 * subscribing to the main process's `library:changed` push (fires on any file
 * edit, in-app or on disk).
 */
export function useLibrary(cwd?: string) {
  const [items, setItems] = useState<LibraryItem[]>([]);
  const [loaded, setLoaded] = useState(false);

  const reload = useCallback(() => {
    // Optional-chain: a backend that doesn't implement library listing (or a
    // test harness with a partial electronAPI mock) simply yields an empty
    // library instead of throwing.
    const p = window.electronAPI.libraryList?.(cwd);
    if (!p) {
      setLoaded(true);
      return;
    }
    p.then((list) => {
      setItems(Array.isArray(list) ? list : []);
      setLoaded(true);
    }).catch(() => setLoaded(true));
  }, [cwd]);

  useEffect(() => {
    reload();
  }, [reload]);

  useEffect(() => {
    const unsub = window.electronAPI.onLibraryChanged?.(reload);
    return unsub;
  }, [reload]);

  const save = useCallback(
    async (input: LibrarySaveInput) => {
      await window.electronAPI.librarySave({ ...input, cwd: input.cwd ?? cwd });
      reload();
    },
    [cwd, reload],
  );

  const remove = useCallback(
    // `origin` decides WHICH root a claude item is unlinked from: without it a
    // delete of a `~/.claude` skill silently unlinked nothing, because the
    // service defaulted to the project root the item never lived in.
    async (scope: LibraryScope, id: string, kind?: LibraryKind, origin?: ClaudeOrigin) => {
      await window.electronAPI.libraryRemove(scope, id, cwd, kind, origin);
      reload();
    },
    [cwd, reload],
  );

  return { items, loaded, reload, save, remove };
}
