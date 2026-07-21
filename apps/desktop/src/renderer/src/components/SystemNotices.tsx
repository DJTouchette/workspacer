import React, { useEffect, useState, useCallback } from 'react';
import { X, AlertTriangle, AlertCircle, Info } from 'lucide-react';

interface Notice {
  level: 'error' | 'warn' | 'info';
  title: string;
  detail?: string;
  key?: string;
}

const ACCENT: Record<Notice['level'], string> = {
  error: 'var(--wks-error)',
  warn: 'var(--wks-warning)',
  info: 'var(--wks-accent)',
};

function Icon({ level }: { level: Notice['level'] }) {
  const c = ACCENT[level];
  if (level === 'error') return <AlertCircle size={15} color={c} />;
  if (level === 'warn') return <AlertTriangle size={15} color={c} />;
  return <Info size={15} color={c} />;
}

/**
 * Stacked, dismissible banners for main-process system notices (a daemon that
 * failed to start, a crash-loop that gave up, etc.) pushed over
 * `onSystemNotice`. Without these, those failures only hit the console and the
 * app looks silently broken. Same-`key` notices replace rather than stack.
 */
export const SystemNotices: React.FC = () => {
  const [notices, setNotices] = useState<Notice[]>([]);

  useEffect(() => {
    const unsub = window.electronAPI.onSystemNotice?.((n) => {
      setNotices((prev) => {
        const rest = n.key ? prev.filter((p) => p.key !== n.key) : prev;
        return [...rest, n];
      });
    });
    return () => {
      unsub?.();
    };
  }, []);

  const dismiss = useCallback((idx: number) => {
    setNotices((prev) => prev.filter((_, i) => i !== idx));
  }, []);

  if (notices.length === 0) return null;

  return (
    <div
      style={{
        position: 'fixed',
        top: 38,
        left: '50%',
        transform: 'translateX(-50%)',
        zIndex: 30000,
        display: 'flex',
        flexDirection: 'column',
        gap: 8,
        width: 'min(560px, calc(100vw - 40px))',
        pointerEvents: 'none',
      }}
    >
      {notices.map((n, i) => (
        <div
          key={(n.key ?? 'n') + ':' + i}
          style={{
            pointerEvents: 'auto',
            display: 'flex',
            gap: 10,
            alignItems: 'flex-start',
            padding: '10px 12px',
            borderRadius: 'var(--wks-radius-md)',
            background: 'var(--wks-glass-strong)',
            backdropFilter: 'blur(var(--wks-glass-blur)) saturate(160%)',
            WebkitBackdropFilter: 'blur(var(--wks-glass-blur)) saturate(160%)',
            border: `1px solid ${ACCENT[n.level]}`,
            boxShadow: '0 8px 28px var(--wks-glass-shadow)',
            color: 'var(--wks-text-primary)',
          }}
        >
          <div style={{ marginTop: 1, flexShrink: 0 }}>
            <Icon level={n.level} />
          </div>
          <div style={{ flex: 1, minWidth: 0 }}>
            <div style={{ fontSize: '0.78rem', fontWeight: 600 }}>{n.title}</div>
            {n.detail && (
              <div
                style={{
                  fontSize: '0.7rem',
                  color: 'var(--wks-text-muted)',
                  marginTop: 3,
                  lineHeight: 1.45,
                  wordBreak: 'break-word',
                }}
              >
                {n.detail}
              </div>
            )}
            {n.level !== 'info' && (
              <button
                onClick={() => window.electronAPI.openLogsFolder?.()}
                style={{
                  marginTop: 6,
                  cursor: 'pointer',
                  fontSize: '0.69rem',
                  fontWeight: 600,
                  fontFamily: 'inherit',
                  background: 'transparent',
                  color: ACCENT[n.level],
                  border: `1px solid ${ACCENT[n.level]}`,
                  borderRadius: 4,
                  padding: '2px 8px',
                }}
              >
                Open logs
              </button>
            )}
          </div>
          <button
            onClick={() => dismiss(i)}
            title="Dismiss"
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
            <X size={14} />
          </button>
        </div>
      ))}
    </div>
  );
};

export default SystemNotices;
