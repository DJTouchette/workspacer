/**
 * Cross-component bus for posting to the in-app notification center from
 * anywhere in the renderer (panes, hooks, plugin host code) without threading
 * the notifications context through props.
 *
 * Flow: any component calls `postNotification`; NotificationsProvider owns the
 * matching window listener and ingests it into the center (and, unless the
 * input is `silent`, shows a transient toast).
 *
 * External producers do NOT use this bus: plugins publish `notify.post` on the
 * hub bus, and main-process services push over the NOTIFY_IN_APP IPC channel —
 * both are ingested by the same provider.
 */

import type { NotificationInput } from './notificationStore';

export const NOTIFY_POST_EVENT = 'wks:notify-post';

/** Post a notification to the in-app center. Only `title` is required. */
export function postNotification(input: NotificationInput): void {
  window.dispatchEvent(new CustomEvent(NOTIFY_POST_EVENT, { detail: input }));
}
