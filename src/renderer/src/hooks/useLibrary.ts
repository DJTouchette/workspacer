import { useState, useEffect, useCallback } from 'react';
import type { LibraryItem, LibrarySaveInput, LibraryScope, LibraryKind } from '../types/library';

/**
 * Loads the merged library (global + project for `cwd`) and keeps it live by
 * subscribing to the main process's `library:changed` push (fires on any file
 * edit, in-app or on disk).
 */
export function useLibrary(cwd?: string) {
  const [items, setItems] = useState<LibraryItem[]>([]);
  const [loaded, setLoaded] = useState(false);

  const reload = useCallback(() => {
    window.electronAPI.libraryList(cwd)
      .then((list) => { setItems(Array.isArray(list) ? list : []); setLoaded(true); })
      .catch(() => setLoaded(true));
  }, [cwd]);

  useEffect(() => { reload(); }, [reload]);

  useEffect(() => {
    const unsub = window.electronAPI.onLibraryChanged?.(reload);
    return unsub;
  }, [reload]);

  const save = useCallback(async (input: LibrarySaveInput) => {
    await window.electronAPI.librarySave({ ...input, cwd: input.cwd ?? cwd });
    reload();
  }, [cwd, reload]);

  const remove = useCallback(async (scope: LibraryScope, id: string, kind?: LibraryKind) => {
    await window.electronAPI.libraryRemove(scope, id, cwd, kind);
    reload();
  }, [cwd, reload]);

  return { items, loaded, reload, save, remove };
}
