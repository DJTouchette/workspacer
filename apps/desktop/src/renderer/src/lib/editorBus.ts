/**
 * Cross-component bus for opening a file in an Editor pane.
 *
 * A file list (e.g. the Review pane's tree) calls `requestOpenInEditor` with an
 * absolute path; App handles the event and opens an Editor pane bound to it.
 * Mirrors reviewBus, kept separate so the two concerns stay independent.
 */

export const EDITOR_OPEN_FILE_EVENT = 'editor:open-file';

export interface EditorOpenTarget {
  /** Absolute path of the file to edit. */
  path: string;
  /** Repo/working directory the file belongs to (used as the pane's cwd). */
  cwd?: string;
}

export function requestOpenInEditor(target: EditorOpenTarget): void {
  window.dispatchEvent(new CustomEvent(EDITOR_OPEN_FILE_EVENT, { detail: target }));
}
