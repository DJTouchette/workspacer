import React, { useEffect } from 'react';
import { X, AlertTriangle, AlertCircle, Info, Check } from 'lucide-react';
import { useNotifications } from '../../contexts/NotificationsContext';
import type { StoredNotification } from '../../lib/notificationStore';

const ACCENT: Record<StoredNotification['level'], string> = {
  error: 'var(--wks-error)',
  warn: 'var(--wks-warning)',
  success: 'var(--wks-success)',
  info: 'var(--wks-accent)',
};

export function LevelIcon({ level, size = 13 }: { level: StoredNotification['level']; size?: number }) {
  const c = ACCENT[level];
  if (level === 'error') return <AlertCircle size={size} color={c} />;
  if (level === 'warn') return <AlertTriangle size={size} color={c} />;
  if (level === 'success') return <Check size={size} color={c} />;
  return <Info size={size} color={c} />;
}

/** How long a toast lingers before auto-dismissing. */
const TOAST_MS = 6000;

/** One transient toast: click-through to the target, auto-dismisses. */
const Toast: React.FC<{ toast: StoredNotification }> = ({ toast }) => {
  const { activate, dismissToast } = useNotifications();

  useEffect(() => {
    const timer = setTimeout(() => dismissToast(toast.id), TOAST_MS);
    return () => clearTimeout(timer);
  }, [toast.id, dismissToast]);

  const clickable = !!(toast.sessionId || toast.paneType || toast.url);
  return (
    <div
      onClick={() => (clickable ? activate(toast) : dismissToast(toast.id))}
      role="status"
      style={{
        pointerEvents: 'auto',
        display: 'flex',
        gap: 8,
        alignItems: 'flex-start',
        padding: '10px 12px',
        borderRadius: 'var(--wks-radius-md)',
        background: 'var(--wks-glass-strong)',
        backdropFilter: 'blur(var(--wks-glass-blur)) saturate(160%)',
        WebkitBackdropFilter: 'blur(var(--wks-glass-blur)) saturate(160%)',
        border: '1px solid var(--wks-border)',
        borderLeft: `3px solid ${ACCENT[toast.level]}`,
        boxShadow: '0 8px 28px var(--wks-glass-shadow)',
        color: 'var(--wks-text-primary)',
        cursor: 'pointer',
      }}
    >
      <div style={{ marginTop: 1, flexShrink: 0, display: 'flex', alignItems: 'center' }}>
        <LevelIcon level={toast.level} />
      </div>
      <div style={{ flex: 1, minWidth: 0 }}>
        <div style={{ fontSize: '0.72rem', fontWeight: 600 }}>{toast.title}</div>
        {toast.body && (
          <div
            style={{
              fontSize: '0.66rem',
              color: 'var(--wks-text-muted)',
              marginTop: 3,
              lineHeight: 1.45,
              wordBreak: 'break-word',
              display: '-webkit-box',
              WebkitLineClamp: 3,
              WebkitBoxOrient: 'vertical',
              overflow: 'hidden',
            }}
          >
            {toast.body}
          </div>
        )}
        <div style={{ fontSize: '0.6rem', color: 'var(--wks-text-faint)', marginTop: 4 }}>
          {toast.source}
        </div>
      </div>
      <button
        onClick={(e) => {
          e.stopPropagation();
          dismissToast(toast.id);
        }}
        title="Dismiss"
        aria-label="Dismiss notification"
        style={{
          flexShrink: 0,
          cursor: 'pointer',
          background: 'transparent',
          border: 'none',
          color: 'var(--wks-text-muted)',
          padding: 2,
          display: 'flex',
          alignItems: 'center',
        }}
      >
        <X size={13} />
      </button>
    </div>
  );
};

/**
 * Bottom-right transient toast stack for the notification center. New
 * notifications (unless `silent`, or toasts are disabled in Settings) surface
 * here for a few seconds; the full history stays in the bell panel.
 * Bottom-right deliberately avoids the SystemNotices banner strip (top-center).
 */
export const NotificationToasts: React.FC = () => {
  const { toasts } = useNotifications();
  if (toasts.length === 0) return null;

  return (
    <div
      style={{
        position: 'fixed',
        right: 16,
        bottom: 16,
        zIndex: 30000,
        display: 'flex',
        flexDirection: 'column',
        gap: 8,
        width: 'min(360px, calc(100vw - 32px))',
        pointerEvents: 'none',
      }}
    >
      {toasts.map((t) => (
        <Toast key={t.id} toast={t} />
      ))}
    </div>
  );
};

export default NotificationToasts;
