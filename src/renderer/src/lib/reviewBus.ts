/**
 * Cross-component bus for opening a changed file in the Review (git diff) pane.
 *
 * Flow: a file list (e.g. the Claude pane's inspector rail) calls
 * `requestReviewFile`. App handles that, ensures a Review pane is open and
 * focused for the active agent, then calls `openReviewFile`, which the
 * now-mounted ReviewPane picks up to select the file and load its diff. The
 * two-step hop exists so a *freshly created* Review pane is mounted and
 * listening before the open event fires.
 */

export const REVIEW_REQUEST_FILE_EVENT = 'review:request-file';
export const REVIEW_OPEN_FILE_EVENT = 'review:open-file';

export interface ReviewFileTarget {
  /** Path as reported by the file change — typically absolute (Claude emits
   *  absolute `file_path`s). ReviewPane resolves it against git status. */
  path: string;
  /** Working directory of the repo the file belongs to, so the right Review
   *  pane (and only it) responds. */
  cwd?: string;
}

/** Called by a file list to ask the app to reveal a file in the Review pane. */
export function requestReviewFile(target: ReviewFileTarget): void {
  window.dispatchEvent(new CustomEvent(REVIEW_REQUEST_FILE_EVENT, { detail: target }));
}

/** Called by App once a Review pane is open + focused, to tell it which file
 *  to select. */
export function openReviewFile(target: ReviewFileTarget): void {
  window.dispatchEvent(new CustomEvent(REVIEW_OPEN_FILE_EVENT, { detail: target }));
}
