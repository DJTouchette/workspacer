import React, { useEffect, useRef } from 'react';
import { Bell, X } from 'lucide-react';
import { useNotifications } from '../../contexts/NotificationsContext';
import { timeAgo, type StoredNotification } from '../../lib/notificationStore';
import { LevelIcon } from './NotificationToasts';

/** One history row in the bell panel. */
const CenterRow: React.FC<{ item: StoredNotification }> = ({ item }) => {
  const { activate, remove } = useNotifications();
  return (
    <div
      onClick={() => activate(item)}
      style={{
        display: 'flex',
        gap: 8,
        alignItems: 'flex-start',
        padding: '8px 12px',
        borderBottom: '1px solid var(--wks-border-subtle)',
        cursor: 'pointer',
        background: 'transparent',
        transition: 'background 0.1s',
      }}
      onMouseEnter={(e) => {
        (e.currentTarget as HTMLElement).style.background = 'var(--wks-bg-hover)';
      }}
      onMouseLeave={(e) => {
        (e.currentTarget as HTMLElement).style.background = 'transparent';
      }}
    >
      <div style={{ marginTop: 2, flexShrink: 0, display: 'flex', alignItems: 'center' }}>
        <LevelIcon level={item.level} size={12} />
      </div>
      <div style={{ flex: 1, minWidth: 0 }}>
        <div style={{ display: 'flex', alignItems: 'baseline', gap: 6 }}>
          <span
            style={{
              flex: 1,
              minWidth: 0,
              fontSize: '0.72rem',
              fontWeight: item.read ? 500 : 600,
              color: item.read ? 'var(--wks-text-secondary)' : 'var(--wks-text-primary)',
              overflow: 'hidden',
              textOverflow: 'ellipsis',
              whiteSpace: 'nowrap',
            }}
          >
            {item.title}
          </span>
          <span
            style={{
              flexShrink: 0,
              fontSize: '0.6rem',
              fontFamily: 'var(--wks-font-mono)',
              color: 'var(--wks-text-faint)',
            }}
          >
            {timeAgo(item.createdAt)}
          </span>
        </div>
        {item.body && (
          <div
            style={{
              fontSize: '0.66rem',
              color: 'var(--wks-text-muted)',
              marginTop: 2,
              lineHeight: 1.45,
              wordBreak: 'break-word',
              display: '-webkit-box',
              WebkitLineClamp: 2,
              WebkitBoxOrient: 'vertical',
              overflow: 'hidden',
            }}
          >
            {item.body}
          </div>
        )}
        <div style={{ fontSize: '0.6rem', color: 'var(--wks-text-faint)', marginTop: 3 }}>
          {item.source}
        </div>
      </div>
      {!item.read && (
        <span
          aria-label="Unread"
          style={{
            flexShrink: 0,
            marginTop: 5,
            width: 6,
            height: 6,
            borderRadius: '50%',
            background: 'var(--wks-accent)',
          }}
        />
      )}
      <button
        onClick={(e) => {
          e.stopPropagation();
          remove(item.id);
        }}
        title="Remove"
        aria-label="Remove notification"
        style={{
          flexShrink: 0,
          cursor: 'pointer',
          background: 'transparent',
          border: 'none',
          color: 'var(--wks-text-faint)',
          padding: 2,
          marginTop: 1,
          display: 'flex',
          alignItems: 'center',
        }}
      >
        <X size={12} />
      </button>
    </div>
  );
};

const headerButtonStyle: React.CSSProperties = {
  cursor: 'pointer',
  fontSize: '0.66rem',
  fontWeight: 500,
  fontFamily: 'inherit',
  background: 'transparent',
  border: 'none',
  color: 'var(--wks-text-muted)',
  padding: '2px 4px',
};

/**
 * The NavBar bell: unread badge + dropdown panel over the notification-center
 * history. Everything renders from NotificationsContext; clicking a row
 * navigates to its target (agent / pane / URL) and marks it read.
 */
export const NotificationCenter: React.FC = () => {
  const { items, unread, panelOpen, setPanelOpen, markAllRead, clearAll } = useNotifications();
  const rootRef = useRef<HTMLDivElement>(null);

  // Close on outside click / Escape, matching the NavBar menu pattern.
  useEffect(() => {
    if (!panelOpen) return;
    const onMouseDown = (e: MouseEvent) => {
      if (rootRef.current && !rootRef.current.contains(e.target as Node)) setPanelOpen(false);
    };
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setPanelOpen(false);
    };
    document.addEventListener('mousedown', onMouseDown);
    document.addEventListener('keydown', onKeyDown);
    return () => {
      document.removeEventListener('mousedown', onMouseDown);
      document.removeEventListener('keydown', onKeyDown);
    };
  }, [panelOpen, setPanelOpen]);

  return (
    <div
      ref={rootRef}
      style={{
        position: 'relative',
        flexShrink: 0,
        display: 'flex',
        alignItems: 'center',
        // @ts-ignore
        WebkitAppRegion: 'no-drag',
        appRegion: 'no-drag',
      }}
    >
      <button
        onClick={() => setPanelOpen(!panelOpen)}
        title={unread > 0 ? `Notifications — ${unread} unread` : 'Notifications'}
        aria-label="Notifications"
        style={{
          position: 'relative',
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
          width: 24,
          height: 24,
          padding: 0,
          border: 'none',
          borderRadius: 'var(--wks-radius-sm)',
          cursor: 'pointer',
          background: panelOpen ? 'var(--wks-bg-selected)' : 'transparent',
          color: unread > 0 ? 'var(--wks-text-secondary)' : 'var(--wks-text-muted)',
        }}
        onMouseEnter={(e) => {
          (e.currentTarget as HTMLElement).style.background = 'var(--wks-bg-hover)';
        }}
        onMouseLeave={(e) => {
          (e.currentTarget as HTMLElement).style.background = panelOpen
            ? 'var(--wks-bg-selected)'
            : 'transparent';
        }}
      >
        <Bell size={14} strokeWidth={1.75} />
        {unread > 0 && (
          <span
            style={{
              position: 'absolute',
              top: -2,
              right: -3,
              minWidth: 13,
              height: 13,
              padding: '0 3px',
              borderRadius: 'var(--wks-radius-pill)',
              background: 'var(--wks-accent)',
              color: 'var(--wks-accent-text)',
              fontSize: '0.6rem',
              fontWeight: 600,
              lineHeight: '13px',
              textAlign: 'center',
            }}
          >
            {unread > 99 ? '99+' : unread}
          </span>
        )}
      </button>

      {panelOpen && (
        <div
          style={{
            position: 'absolute',
            top: '100%',
            right: 0,
            marginTop: 8,
            width: 'min(360px, calc(100vw - 24px))',
            maxHeight: 440,
            display: 'flex',
            flexDirection: 'column',
            borderRadius: 'var(--wks-radius-lg)',
            background: 'var(--wks-glass-strong)',
            backdropFilter: 'blur(var(--wks-glass-blur)) saturate(160%)',
            WebkitBackdropFilter: 'blur(var(--wks-glass-blur)) saturate(160%)',
            border: '1px solid var(--wks-border)',
            boxShadow: '0 8px 28px var(--wks-glass-shadow)',
            overflow: 'hidden',
            zIndex: 10000,
          }}
        >
          <div
            style={{
              display: 'flex',
              alignItems: 'center',
              gap: 6,
              padding: '8px 12px',
              borderBottom: '1px solid var(--wks-border-subtle)',
            }}
          >
            <span style={{ flex: 1, fontSize: '0.72rem', fontWeight: 600 }}>Notifications</span>
            {unread > 0 && (
              <button onClick={markAllRead} style={headerButtonStyle}>
                Mark all read
              </button>
            )}
            {items.length > 0 && (
              <button onClick={clearAll} style={headerButtonStyle}>
                Clear
              </button>
            )}
          </div>
          <div style={{ flex: 1, minHeight: 0, overflowY: 'auto' }}>
            {items.length === 0 ? (
              <div
                style={{
                  padding: '20px 16px',
                  textAlign: 'center',
                  fontSize: '0.72rem',
                  color: 'var(--wks-text-disabled)',
                }}
              >
                Nothing yet. Agent alerts and plugin updates land here.
              </div>
            ) : (
              items.map((item) => <CenterRow key={item.id} item={item} />)
            )}
          </div>
        </div>
      )}
    </div>
  );
};

export default NotificationCenter;
