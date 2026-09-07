import type { KeyboardEvent } from 'react';

/** Keep keyboard traversal in the active modal, including when controls disappear. */
export function containDialogTab(event: KeyboardEvent<HTMLElement>): void {
  if (event.key !== 'Tab') return;
  const controls = Array.from(
    event.currentTarget.querySelectorAll<HTMLElement>(
      'button:not(:disabled), a[href], input:not(:disabled), textarea:not(:disabled), select:not(:disabled), [tabindex="0"]',
    ),
  ).filter((element) => element.getClientRects().length > 0 && element.tabIndex >= 0);
  const first = controls[0];
  const last = controls.at(-1);
  if (!first || !last) return;
  if (event.shiftKey && document.activeElement === first) {
    event.preventDefault();
    last.focus();
  } else if (!event.shiftKey && document.activeElement === last) {
    event.preventDefault();
    first.focus();
  }
}
