/**
 * Cross-component bus for opening a markdown file in a Preview pane.
 *
 * A file affordance (e.g. FileLink in the chat's tool-call cards) calls
 * `requestMarkdownPreview`; App handles the event and opens (or focuses) an
 * 'mdpreview' pane bound to that file. Mirrors editorBus, kept separate so the
 * two concerns stay independent.
 */

export const MARKDOWN_PREVIEW_EVENT = 'preview:open-markdown';

export interface MarkdownPreviewTarget {
  /** Path of the file to preview — absolute, or relative to `cwd`. */
  path: string;
  /** Repo/working directory the file belongs to (used as the pane's cwd). */
  cwd?: string;
}

export function requestMarkdownPreview(target: MarkdownPreviewTarget): void {
  window.dispatchEvent(new CustomEvent(MARKDOWN_PREVIEW_EVENT, { detail: target }));
}
