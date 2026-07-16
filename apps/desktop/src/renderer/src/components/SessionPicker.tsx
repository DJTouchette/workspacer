import React, { useEffect, useState } from 'react';

interface SessionEntry {
  name: string;
  filename: string;
  timestamp: string;
  paneCount: number;
  agentCount?: number;
}

interface SessionPickerProps {
  sessions: SessionEntry[];
  /** Start a fresh session. `name` is the user-typed name; undefined lets the
   *  lifecycle hook pick a dated default ("Session Jul 16"). */
  onNewSession: (name?: string) => void;
  onResumeSession: (filename: string) => void;
  onDeleteSession: (filename: string) => void;
  /** Rename a saved session file (re-save under the new name + delete old). */
  onRenameSession?: (filename: string, newName: string) => void;
  /** Name of the session currently loaded (mid-session switch) — its row gets
   *  a "current" chip so you know which one you'd be leaving. */
  currentName?: string;
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
  onRenameSession,
  currentName,
  onCancel,
}) => {
  const [newName, setNewName] = useState('');
  /** Filename of the row being renamed inline, and its draft text. */
  const [renaming, setRenaming] = useState<{ filename: string; draft: string } | null>(null);

  // Escape → dismiss (mid-session switch) if cancellable, else start a new
  // session. Capture-phase, so skip it while an input has focus — the name
  // field and inline rename handle their own Escape.
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        if (e.target instanceof HTMLInputElement) return;
        e.preventDefault();
        if (onCancel) onCancel();
        else onNewSession();
      }
    };
    window.addEventListener('keydown', handler, true);
    return () => window.removeEventListener('keydown', handler, true);
  }, [onNewSession, onCancel]);

  const lastSession = sessions[0];
  const startNew = () => onNewSession(newName.trim() || undefined);
  const commitRename = () => {
    if (renaming && renaming.draft.trim()) onRenameSession?.(renaming.filename, renaming.draft);
    setRenaming(null);
  };

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

        {/* New session: optional name + start. Sessions are one-file-per-name,
            so naming here is what keeps the previous session's file intact. */}
        <div style={{ display: 'flex', gap: '8px', marginBottom: '8px' }}>
          <input
            value={newName}
            onChange={(e) => setNewName(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') startNew();
              if (e.key === 'Escape') setNewName('');
            }}
            placeholder="New session name (optional)…"
            style={{
              flex: 1,
              minWidth: 0,
              padding: '7px 10px',
              fontSize: '0.75rem',
              fontFamily: 'inherit',
              backgroundColor: 'var(--wks-bg-elevated)',
              color: 'var(--wks-text-primary)',
              border: '1px solid var(--wks-border-input)',
              borderRadius: 'var(--wks-radius-sm)',
              outline: 'none',
              boxSizing: 'border-box',
            }}
          />
          <ActionButton label="New Session" onClick={startNew} primary={!lastSession} />
        </div>
        <div style={{ display: 'flex', gap: '8px', marginBottom: '16px' }}>
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
                    {renaming?.filename === session.filename ? (
                      <input
                        autoFocus
                        value={renaming.draft}
                        onClick={(e) => e.stopPropagation()}
                        onChange={(e) =>
                          setRenaming((r) => (r ? { ...r, draft: e.target.value } : r))
                        }
                        onKeyDown={(e) => {
                          if (e.key === 'Enter') commitRename();
                          if (e.key === 'Escape') setRenaming(null);
                        }}
                        onBlur={commitRename}
                        style={{
                          width: '100%',
                          padding: '2px 6px',
                          fontSize: '0.75rem',
                          fontFamily: 'inherit',
                          backgroundColor: 'var(--wks-bg-elevated)',
                          color: 'var(--wks-text-primary)',
                          border: '1px solid var(--wks-accent)',
                          borderRadius: '3px',
                          outline: 'none',
                          boxSizing: 'border-box',
                        }}
                      />
                    ) : (
                      <div
                        style={{
                          display: 'flex',
                          alignItems: 'center',
                          gap: 6,
                          fontSize: '0.75rem',
                          color: 'var(--wks-text-secondary)',
                          fontWeight: 500,
                        }}
                      >
                        <span
                          style={{
                            overflow: 'hidden',
                            textOverflow: 'ellipsis',
                            whiteSpace: 'nowrap',
                          }}
                        >
                          {session.name}
                        </span>
                        {currentName === session.name && (
                          <span
                            style={{
                              fontSize: '0.55rem',
                              fontWeight: 700,
                              padding: '0px 5px',
                              borderRadius: 'var(--wks-radius-pill, 999px)',
                              letterSpacing: '0.04em',
                              flexShrink: 0,
                              color: 'var(--wks-accent-text, var(--wks-accent))',
                              border:
                                '1px solid color-mix(in srgb, var(--wks-accent) 45%, transparent)',
                              background: 'color-mix(in srgb, var(--wks-accent) 12%, transparent)',
                            }}
                          >
                            current
                          </span>
                        )}
                      </div>
                    )}
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
                  {onRenameSession && renaming?.filename !== session.filename && (
                    <button
                      onClick={(e) => {
                        e.stopPropagation();
                        setRenaming({ filename: session.filename, draft: session.name });
                      }}
                      style={{
                        background: 'none',
                        border: 'none',
                        color: 'var(--wks-text-faint)',
                        cursor: 'pointer',
                        fontSize: '0.7rem',
                        padding: '2px 4px',
                        margin: 0,
                        width: 'auto',
                        height: 'auto',
                        borderRadius: '3px',
                        lineHeight: 1,
                        flexShrink: 0,
                      }}
                      onMouseEnter={(e) => {
                        (e.currentTarget as HTMLElement).style.color = 'var(--wks-text-primary)';
                      }}
                      onMouseLeave={(e) => {
                        (e.currentTarget as HTMLElement).style.color = 'var(--wks-text-faint)';
                      }}
                      title="Rename session"
                    >
                      ✎
                    </button>
                  )}
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
