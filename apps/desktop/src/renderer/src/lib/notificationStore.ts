/**
 * Pure state logic for the in-app notification center. The React context
 * (contexts/NotificationsContext.tsx) owns the state; everything that can be
 * expressed as data-in/data-out lives here so it is unit-testable.
 *
 * A notification can arrive from four producers: the main-process notifiers
 * (onInAppNotification IPC), hub-bus `notify.post` events (plugins/remote),
 * renderer-internal posts (lib/notificationBus), and mirrored system notices.
 * All of them normalize through `normalizeNotification` into the shared
 * InAppNotification wire shape before ingestion.
 */

import type { InAppNotification } from '../../../main/shared/ipcTypes';

export type { InAppNotification };

export interface StoredNotification extends InAppNotification {
  read: boolean;
}

/** Loose input accepted from buses and plugins; only `title` is required. */
export interface NotificationInput {
  id?: string;
  level?: string;
  title?: string;
  body?: string;
  source?: string;
  sessionId?: string;
  paneType?: string;
  url?: string;
  key?: string;
  silent?: boolean;
}

/** Center history cap (in memory). */
export const MAX_ITEMS = 200;
/** How many entries survive a reload (localStorage). */
export const PERSIST_LIMIT = 100;
export const STORAGE_KEY = 'wks.notifications.v1';

const LEVELS = ['info', 'success', 'warn', 'error'] as const;

function clampLevel(level: unknown): InAppNotification['level'] {
  return (LEVELS as readonly string[]).includes(level as string)
    ? (level as InAppNotification['level'])
    : 'info';
}

function truncate(s: string, max: number): string {
  return s.length <= max ? s : s.slice(0, max - 1) + '…';
}

let counter = 0;
function nextId(): string {
  counter += 1;
  return `n-${Date.now().toString(36)}-${counter}`;
}

/**
 * Validate + clamp a loose input into a full InAppNotification.
 * Returns null when there is nothing displayable (no title).
 */
export function normalizeNotification(
  input: NotificationInput | null | undefined,
  fallbackSource: string,
): InAppNotification | null {
  if (!input || typeof input.title !== 'string' || !input.title.trim()) return null;
  return {
    id: typeof input.id === 'string' && input.id ? input.id : nextId(),
    level: clampLevel(input.level),
    title: truncate(input.title.trim(), 200),
    body:
      typeof input.body === 'string' && input.body.trim()
        ? truncate(input.body.trim(), 600)
        : undefined,
    source:
      typeof input.source === 'string' && input.source.trim()
        ? truncate(input.source.trim(), 60)
        : fallbackSource,
    sessionId: typeof input.sessionId === 'string' ? input.sessionId : undefined,
    paneType: typeof input.paneType === 'string' ? input.paneType : undefined,
    url: typeof input.url === 'string' ? input.url : undefined,
    key: typeof input.key === 'string' && input.key ? input.key : undefined,
    createdAt: Date.now(),
    silent: input.silent === true,
  };
}

/** Prepend a notification: same-id re-delivery is dropped, same-`key` entries
 *  are replaced (latest wins, moves to the top), history is capped. */
export function ingest(
  items: StoredNotification[],
  n: InAppNotification,
): StoredNotification[] {
  if (items.some((it) => it.id === n.id)) return items;
  const rest = n.key ? items.filter((it) => it.key !== n.key) : items;
  return [{ ...n, read: false }, ...rest].slice(0, MAX_ITEMS);
}

export function unreadCount(items: StoredNotification[]): number {
  return items.reduce((acc, it) => acc + (it.read ? 0 : 1), 0);
}

export function markRead(items: StoredNotification[], id: string): StoredNotification[] {
  return items.map((it) => (it.id === id && !it.read ? { ...it, read: true } : it));
}

export function markAllRead(items: StoredNotification[]): StoredNotification[] {
  return items.some((it) => !it.read) ? items.map((it) => ({ ...it, read: true })) : items;
}

export function removeItem(items: StoredNotification[], id: string): StoredNotification[] {
  return items.filter((it) => it.id !== id);
}

type StorageLike = Pick<Storage, 'getItem' | 'setItem'>;

/** Restore persisted history (invalid/corrupt payloads restore to empty). */
export function loadPersisted(storage: StorageLike): StoredNotification[] {
  try {
    const raw = storage.getItem(STORAGE_KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    return parsed
      .filter(
        (it): it is StoredNotification =>
          !!it &&
          typeof it === 'object' &&
          typeof (it as StoredNotification).id === 'string' &&
          typeof (it as StoredNotification).title === 'string' &&
          typeof (it as StoredNotification).createdAt === 'number',
      )
      .map((it) => ({ ...it, level: clampLevel(it.level), read: it.read !== false }))
      .slice(0, PERSIST_LIMIT);
  } catch {
    return [];
  }
}

export function persist(storage: StorageLike, items: StoredNotification[]): void {
  try {
    storage.setItem(STORAGE_KEY, JSON.stringify(items.slice(0, PERSIST_LIMIT)));
  } catch {
    /* quota/serialization failures are non-fatal — history just won't survive reload */
  }
}

/** Where a notification entered the renderer: pushed by the main process
 *  (which already made its own OS-notification decision at the source), or
 *  ingested renderer-side (hub `notify.post` events, in-renderer posts). */
export type NotificationOrigin = 'main' | 'renderer';

/**
 * Should a just-ingested notification escalate to an OS notification?
 * Only renderer-originated ones (main-originated already fired or deliberately
 * didn't — re-raising would double-notify), only when the window isn't focused
 * (a toast in an unfocused window is invisible), and never for silent entries
 * (silent means silent on every surface). Config gating (`notifications.enabled`,
 * sound) is re-checked by the main process on receipt.
 */
export function shouldEscalate(
  n: InAppNotification,
  origin: NotificationOrigin,
  windowFocused: boolean,
): boolean {
  return origin === 'renderer' && !windowFocused && n.silent !== true;
}

/** Compact relative timestamp for center rows ("now", "5m", "2h", "3d"). */
export function timeAgo(createdAt: number, now: number = Date.now()): string {
  const s = Math.max(0, Math.floor((now - createdAt) / 1000));
  if (s < 60) return 'now';
  if (s < 3600) return `${Math.floor(s / 60)}m`;
  if (s < 86400) return `${Math.floor(s / 3600)}h`;
  return `${Math.floor(s / 86400)}d`;
}
