import React, { useEffect } from 'react';

interface SessionEntry {
  name: string;
  filename: string;
  timestamp: string;
  paneCount: number;
  agentCount?: number;
}

interface SessionPickerProps {
  sessions: SessionEntry[];
  onNewSession: () => void;
  onResumeSession: (filename: string) => void;
  onDeleteSession: (filename: string) => void;
  /** When provided, the picker is dismissable (mid-session switch) — Escape and
   *  a Cancel button return to the running app instead of starting fresh. */
  onCancel?: () => void;
}

function formatTimestamp(ts: string): string {
  if (!ts) return '';
  try {
    const date = new Date(ts);
    const now = new Date();
    const diffMs = now.getTime() - date.getTime();
    const diffMin = Math.floor(diffMs / 60000);
    if (diffMin < 1) return 'Just now';
    if (diffMin < 60) return `${diffMin}m ago`;
    const diffHrs = Math.floor(diffMin / 60);
    if (diffHrs < 24) return `${diffHrs}h ago`;
    const diffDays = Math.floor(diffHrs / 24);
    if (diffDays < 7) return `${diffDays}d ago`;
    return date.toLocaleDateString();
  } catch {
    return ts;
  }
}

const SessionPicker: React.FC<SessionPickerProps> = ({
  sessions,
  onNewSession,
  onResumeSession,
  onDeleteSession,
  onCancel,
}) => {
  // Escape → dismiss (mid-session switch) if cancellable, else start a new session.
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault();
        if (onCancel) onCancel();
        else onNewSession();
      }
    };
    window.addEventListener('keydown', handler, true);
    return () => window.removeEventListener('keydown', handler, true);
  }, [onNewSession, onCancel]);

  const lastSession = sessions[0];

  return (
    <div
      style={{
        position: 'fixed',
        top: 0,
        left: 0,
        right: 0,
        bottom: 0,
        backgroundColor: 'var(--wks-bg-base)',
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        zIndex: 2000,
      }}
    >
      <div
        style={{
          backgroundColor: 'var(--wks-glass-strong)',
          backdropFilter: 'blur(var(--wks-glass-blur)) saturate(170%)',
          WebkitBackdropFilter: 'blur(var(--wks-glass-blur)) saturate(170%)',
          border: '1px solid var(--wks-glass-border)',
          borderRadius: 'var(--wks-radius-lg)',
          padding: '20px 24px',
          // Phone-safe: never wider than the viewport (was a hard 340px min that
          // overflowed on narrow screens).
          width: 'min(450px, 92vw)',
          boxSizing: 'border-box',
          maxHeight: '70vh',
          boxShadow:
            '0 16px 48px var(--wks-glass-shadow), inset 0 0 0 1.5px var(--wks-glass-highlight)',
          display: 'flex',
          flexDirection: 'column',
        }}
      >
        <div
          style={{
            fontSize: '1rem',
            fontWeight: 600,
            color: 'var(--wks-text-primary)',
            marginBottom: '16px',
          }}
        >
          Workspacer
        </div>

        {/* Action buttons */}
        <div style={{ display: 'flex', gap: '8px', marginBottom: '16px' }}>
          <ActionButton label="New Session" onClick={onNewSession} primary={!lastSession} />
          {lastSession && (
            <ActionButton
              label="Resume Last"
              onClick={() => onResumeSession(lastSession.filename)}
              primary
            />
          )}
          {onCancel && <ActionButton label="Cancel" onClick={onCancel} />}
        </div>

        {/* Session list */}
        {sessions.length > 0 && (
          <>
            <div
              style={{
                fontSize: '0.6rem',
                color: 'var(--wks-text-faint)',
                textTransform: 'uppercase',
                letterSpacing: '0.05em',
                marginBottom: '6px',
              }}
            >
              Saved Sessions
            </div>
            <div style={{ overflow: 'auto', flex: 1 }}>
              {sessions.map((session) => (
                <div
                  key={session.filename}
                  style={{
                    display: 'flex',
                    alignItems: 'center',
                    justifyContent: 'space-between',
                    padding: '6px 8px',
                    borderRadius: '4px',
                    cursor: 'pointer',
                    marginBottom: '2px',
                  }}
                  onClick={() => onResumeSession(session.filename)}
                  onMouseEnter={(e) => {
                    (e.currentTarget as HTMLElement).style.backgroundColor = 'var(--wks-bg-hover)';
                  }}
                  onMouseLeave={(e) => {
                    (e.currentTarget as HTMLElement).style.backgroundColor = 'transparent';
                  }}
                >
                  <div style={{ flex: 1, minWidth: 0 }}>
                    <div
                      style={{
                        fontSize: '0.75rem',
                        color: 'var(--wks-text-secondary)',
                        fontWeight: 500,
                      }}
                    >
                      {session.name}
                    </div>
                    <div
                      style={{
                        fontSize: '0.6rem',
                        color: 'var(--wks-text-faint)',
                        marginTop: '1px',
                      }}
                    >
                      {session.agentCount ? `${session.agentCount} agents · ` : ''}
                      {session.paneCount} panes &middot; {formatTimestamp(session.timestamp)}
                    </div>
                  </div>
                  <button
                    onClick={(e) => {
                      e.stopPropagation();
                      onDeleteSession(session.filename);
                    }}
                    style={{
                      background: 'none',
                      border: 'none',
                      color: 'var(--wks-text-faint)',
                      cursor: 'pointer',
                      fontSize: '0.75rem',
                      padding: '2px 4px',
                      margin: 0,
                      width: 'auto',
                      height: 'auto',
                      borderRadius: '3px',
                      lineHeight: 1,
                    }}
                    onMouseEnter={(e) => {
                      (e.currentTarget as HTMLElement).style.color = 'var(--wks-error)';
                    }}
                    onMouseLeave={(e) => {
                      (e.currentTarget as HTMLElement).style.color = 'var(--wks-text-faint)';
                    }}
                    title="Delete session"
                  >
                    &#x2715;
                  </button>
                </div>
              ))}
            </div>
          </>
        )}

        <div
          style={{
            fontSize: '0.55rem',
            color: 'var(--wks-text-disabled)',
            marginTop: '12px',
            textAlign: 'center',
          }}
        >
          {onCancel ? 'Press Escape to cancel' : 'Press Escape for new session'}
        </div>
      </div>
    </div>
  );
};

function ActionButton({
  label,
  onClick,
  primary,
}: {
  label: string;
  onClick: () => void;
  primary?: boolean;
}) {
  return (
    <button
      onClick={onClick}
      style={{
        flex: 1,
        padding: '8px 16px',
        fontSize: '0.75rem',
        fontFamily: 'inherit',
        fontWeight: 600,
        backgroundColor: primary ? 'var(--wks-accent)' : 'var(--wks-bg-elevated)',
        color: primary ? '#fff' : 'var(--wks-text-tertiary)',
        border: primary ? '1px solid var(--wks-accent)' : '1px solid var(--wks-border-input)',
        borderRadius: 'var(--wks-radius-sm)',
        cursor: 'pointer',
        height: 'auto',
        lineHeight: 1.4,
        margin: 0,
        width: 'auto',
      }}
      onMouseEnter={(e) => {
        (e.currentTarget as HTMLElement).style.opacity = '0.85';
      }}
      onMouseLeave={(e) => {
        (e.currentTarget as HTMLElement).style.opacity = '1';
      }}
    >
      {label}
    </button>
  );
}

export default SessionPicker;
