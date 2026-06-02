/**
 * Renderer-local event bus for library actions. Decouples the surfaces that
 * *pick* an item (command palette, Library pane, hotkey picker) from the single
 * host that *runs* it (LibraryHost, mounted in App with spawn + agent context).
 */
import type { LibraryItem, LibraryAction } from '../types/library';

export const LIBRARY_RUN_EVENT = 'library:run';
export const LIBRARY_INSERT_EVENT = 'library:insert';

/** Ask the LibraryHost to run an item (templating + the chosen/default action). */
export function runLibraryItem(item: LibraryItem, action?: LibraryAction): void {
  window.dispatchEvent(new CustomEvent(LIBRARY_RUN_EVENT, { detail: { item, action } }));
}

/** Low-level: deliver text into a Claude pane's input. Targets a specific
 *  sessionId/paneId, or the active pane when neither is given. */
export function dispatchInsert(text: string, target?: { sessionId?: string; paneId?: string }): void {
  window.dispatchEvent(new CustomEvent(LIBRARY_INSERT_EVENT, { detail: { ...(target ?? {}), text } }));
}

export interface LibraryRunDetail {
  item: LibraryItem;
  action?: LibraryAction;
}
export interface LibraryInsertDetail {
  text: string;
  sessionId?: string;
  paneId?: string;
}
