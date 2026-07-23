/**
 * In-app notification center state. One provider (mounted in App) ingests
 * every notification producer into a single history + unread count + transient
 * toast queue, and owns click-through navigation:
 *
 *  - main-process notifiers (agent needs-you/done, budget, capability calls)
 *    via `onInAppNotification` (NOTIFY_IN_APP IPC push)
 *  - hub-bus `notify.post` events — the plugin/remote path (the renderer
 *    already receives every bus event via onHubEvent)
 *  - renderer-internal posts via lib/notificationBus
 *  - system notices (mirrored silently — SystemNotices already banners them)
 *
 * History is capped and persisted to localStorage so the center survives a
 * reload. Clicking a notification resolves its target in priority order:
 * sessionId (select that agent) → paneType (open that pane) → url (external).
 *
 * Renderer-originated notifications (bus events, in-renderer posts) that
 * arrive while the window is unfocused escalate to an OS notification
 * (notifyEscalate → main), whose click routes back through `activate` — see
 * `shouldEscalate` in lib/notificationStore for the exact rule.
 */

import React, {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
} from 'react';
import { useConfig } from '../hooks/useConfig';
import { NOTIFY_POST_EVENT } from '../lib/notificationBus';
import {
  ingest,
  loadPersisted,
  markAllRead as storeMarkAllRead,
  markRead as storeMarkRead,
  normalizeNotification,
  persist,
  removeItem,
  shouldEscalate,
  unreadCount,
  type InAppNotification,
  type NotificationInput,
  type NotificationOrigin,
  type StoredNotification,
} from '../lib/notificationStore';

/** Most toasts shown at once — older ones are dropped, not queued. */
const MAX_TOASTS = 4;

export interface NotificationsApi {
  items: StoredNotification[];
  unread: number;
  /** Transient toast queue (already filtered by the inAppToasts setting). */
  toasts: StoredNotification[];
  panelOpen: boolean;
  setPanelOpen: (open: boolean) => void;
  /** Post from renderer code (equivalent to lib/notificationBus). */
  post: (input: NotificationInput) => void;
  /** Click-through: mark read, close toast/panel, navigate to the target. */
  activate: (n: StoredNotification) => void;
  dismissToast: (id: string) => void;
  markRead: (id: string) => void;
  markAllRead: () => void;
  remove: (id: string) => void;
  clearAll: () => void;
}

const NotificationsContext = createContext<NotificationsApi | null>(null);

export function useNotifications(): NotificationsApi {
  const ctx = useContext(NotificationsContext);
  if (!ctx) throw new Error('useNotifications must be used within NotificationsProvider');
  return ctx;
}

interface NotificationsProviderProps {
  /** Select the agent owning this session (sidebar click equivalent). */
  onFocusSession?: (sessionId: string) => void;
  /** Open a pane by type (plugin panes use their plugin pane type). */
  onOpenPane?: (paneType: string) => void;
  children: React.ReactNode;
}

export const NotificationsProvider: React.FC<NotificationsProviderProps> = ({
  onFocusSession,
  onOpenPane,
  children,
}) => {
  const { config } = useConfig();
  const [items, setItems] = useState<StoredNotification[]>(() =>
    loadPersisted(window.localStorage),
  );
  const [toasts, setToasts] = useState<StoredNotification[]>([]);
  const [panelOpen, setPanelOpen] = useState(false);

  // Refs so the (mount-once) ingestion listeners always see current values.
  const toastsEnabledRef = useRef(true);
  toastsEnabledRef.current =
    (config.notifications as { inAppToasts?: boolean } | undefined)?.inAppToasts !== false;
  const focusSessionRef = useRef(onFocusSession);
  focusSessionRef.current = onFocusSession;
  const openPaneRef = useRef(onOpenPane);
  openPaneRef.current = onOpenPane;

  // Ids already ingested this session, so bus redelivery (reconnect replays)
  // can't re-toast or re-escalate. History dedup lives in ingest(); this set
  // guards the side effects that fire outside the state updater.
  const seenIdsRef = useRef<Set<string>>(new Set());

  const ingestOne = useCallback((n: InAppNotification | null, origin: NotificationOrigin) => {
    if (!n) return;
    if (seenIdsRef.current.has(n.id)) return;
    if (seenIdsRef.current.size > 2000) seenIdsRef.current.clear();
    seenIdsRef.current.add(n.id);

    setItems((prev) => ingest(prev, n));
    if (!n.silent && toastsEnabledRef.current) {
      setToasts((prev) =>
        [
          ...prev.filter((t) => t.id !== n.id && (!n.key || t.key !== n.key)),
          { ...n, read: false },
        ].slice(-MAX_TOASTS),
      );
    }
    // A toast in an unfocused window is invisible — hand renderer-only
    // notifications to the OS surface (main re-checks `notifications.enabled`).
    if (shouldEscalate(n, origin, document.hasFocus())) {
      window.electronAPI.notifyEscalate?.(n);
    }
  }, []);

  // ── Producers ──────────────────────────────────────────────────────────
  useEffect(() => {
    // Renderer-internal posts.
    const onPost = (e: Event) => {
      ingestOne(
        normalizeNotification((e as CustomEvent).detail as NotificationInput, 'app'),
        'renderer',
      );
    };
    window.addEventListener(NOTIFY_POST_EVENT, onPost);

    // Main-process notifiers (agent state, budget, hub capability calls).
    const offMain = window.electronAPI.onInAppNotification?.((n) =>
      ingestOne(normalizeNotification(n, 'app'), 'main'),
    );

    // Hub bus: plugins/remote publish `notify.post` events.
    const offHub = window.electronAPI.onHubEvent?.((ev) => {
      if (ev.type !== 'notify.post') return;
      const input = { ...((ev.data ?? {}) as NotificationInput) };
      // The envelope id makes bus redelivery (reconnect replays) idempotent.
      if (!input.id && typeof (ev as { id?: string }).id === 'string') {
        input.id = `bus-${(ev as { id?: string }).id}`;
      }
      const source =
        typeof (ev as { source?: string }).source === 'string' && (ev as { source?: string }).source
          ? ((ev as { source?: string }).source as string)
          : 'bus';
      ingestOne(normalizeNotification(input, source), 'renderer');
    });

    // System notices already show as banners — record silently for history.
    const offNotice = window.electronAPI.onSystemNotice?.((notice) =>
      ingestOne(
        normalizeNotification(
          {
            level: notice.level,
            title: notice.title,
            body: notice.detail,
            key: notice.key ? `system:${notice.key}` : undefined,
            silent: true,
          },
          'system',
        ),
        'main',
      ),
    );

    return () => {
      window.removeEventListener(NOTIFY_POST_EVENT, onPost);
      offMain?.();
      offHub?.();
      offNotice?.();
    };
  }, [ingestOne]);

  useEffect(() => {
    persist(window.localStorage, items);
  }, [items]);

  // ── Actions ────────────────────────────────────────────────────────────
  const dismissToast = useCallback((id: string) => {
    setToasts((prev) => prev.filter((t) => t.id !== id));
  }, []);

  const activate = useCallback(
    (n: StoredNotification) => {
      setItems((prev) => storeMarkRead(prev, n.id));
      dismissToast(n.id);
      setPanelOpen(false);
      if (n.sessionId && focusSessionRef.current) focusSessionRef.current(n.sessionId);
      else if (n.paneType && openPaneRef.current) openPaneRef.current(n.paneType);
      else if (n.url) {
        if (window.electronAPI.openExternalUrl) void window.electronAPI.openExternalUrl(n.url);
        else window.open(n.url, '_blank', 'noopener');
      }
    },
    [dismissToast],
  );

  // Escalated OS notification clicked: main focused the window and handed the
  // notification back — run the same activate path (mark read + navigate).
  useEffect(() => {
    const off = window.electronAPI.onNotificationActivate?.((n) => activate({ ...n, read: false }));
    return () => off?.();
  }, [activate]);

  const api = useMemo<NotificationsApi>(
    () => ({
      items,
      unread: unreadCount(items),
      toasts,
      panelOpen,
      setPanelOpen,
      post: (input) => ingestOne(normalizeNotification(input, 'app'), 'renderer'),
      activate,
      dismissToast,
      markRead: (id) => setItems((prev) => storeMarkRead(prev, id)),
      markAllRead: () => setItems((prev) => storeMarkAllRead(prev)),
      remove: (id) => {
        setItems((prev) => removeItem(prev, id));
        dismissToast(id);
      },
      clearAll: () => {
        setItems([]);
        setToasts([]);
      },
    }),
    [items, toasts, panelOpen, ingestOne, activate, dismissToast],
  );

  return <NotificationsContext.Provider value={api}>{children}</NotificationsContext.Provider>;
};
