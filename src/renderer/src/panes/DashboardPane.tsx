import React, { useState, useEffect, useRef } from 'react';
import type { ClaudeSessionSnapshot } from '../types/claudeSession';
import type { TabConfig } from '../types/pane';
import { WriteTerminal } from '../lib/terminalApi';

// ── Colors (shared with ClaudePane) ──

const colors = {
  bg: 'var(--wks-claude-bg)',
  bgSecondary: 'var(--wks-bg-hover)',
  text: 'var(--wks-text-secondary)',
  textBright: 'var(--wks-text-primary)',
  muted: 'var(--wks-text-faint)',
  accent: 'var(--wks-accent-text)',
  success: 'var(--wks-success)',
  error: 'var(--wks-error)',
  warning: 'var(--wks-warning)',
  border: 'var(--wks-claude-border)',
  borderSubtle: 'var(--wks-claude-border-subtle)',
};

const badgeColors: Record<string, string> = {
  idle: colors.success,
  thinking: colors.warning,
  streaming: colors.accent,
  waiting_input: '#c084fc',
  waiting_approval: colors.error,
};

const badgeLabels: Record<string, string> = {
  idle: 'Idle',
  thinking: 'Thinking...',
  streaming: 'Streaming',
  waiting_input: 'Waiting for input',
  waiting_approval: 'Needs approval',
};

// ── Props ──

interface DashboardPaneProps {
  title: string;
  tabs: TabConfig[];
  onNavigateToTab: (tabId: string) => void;
}

// ── Session Card ──

const SessionCard: React.FC<{
  session: ClaudeSessionSnapshot;
  tabs: TabConfig[];
  onNavigateToTab: (tabId: string) => void;
}> = ({ session, tabs, onNavigateToTab }) => {
  const [inputValue, setInputValue] = useState('');
  const [approvalDismissedAt, setApprovalDismissedAt] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);

  const projectName = session.cwd.split(/[/\\]/).pop() || session.cwd;
  const cwdShort = session.cwd.length > 50
    ? '...' + session.cwd.slice(-47)
    : session.cwd;
  const state = session.ambientState;
  const badgeColor = badgeColors[state] ?? '#555';
  const badgeLabel = badgeLabels[state] ?? state;

  // Find last meaningful message
  const lastMsg = [...session.conversation].reverse().find(t => t.content);
  const preview = lastMsg?.content?.slice(0, 150) ?? '';

  // Find which tab contains this session's pty
  const ownerTab = tabs.find(tab =>
    tab.panes.some(p => p.id === session.ptyId || p.type === 'claude')
  );

  const handleSend = () => {
    if (!inputValue.trim() || !session.ptyId) return;
    WriteTerminal(session.ptyId, inputValue);
    setTimeout(() => WriteTerminal(session.ptyId, '\r'), 50);
    setInputValue('');
  };

  // Claude Code uses an interactive select menu — Enter selects "Yes",
  // arrow-down twice then Enter selects "No"
  const handleApproval = (approve: boolean) => {
    if (!session.ptyId) return;
    if (approve) {
      WriteTerminal(session.ptyId, '\r');
    } else {
      WriteTerminal(session.ptyId, '\x1b[B\x1b[B\r');
    }
    setApprovalDismissedAt(Date.now());
  };

  return (
    <div style={{
      borderRadius: 10,
      border: `1px solid ${state === 'waiting_approval' ? colors.error : colors.borderSubtle}`,
      backgroundColor: 'rgba(255,255,255,0.02)',
      overflow: 'hidden',
      transition: 'border-color 0.2s',
    }}>
      {/* Header */}
      <div style={{
        padding: '10px 14px',
        borderBottom: `1px solid ${colors.borderSubtle}`,
      }}>
        <div style={{
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'space-between',
          marginBottom: 4,
        }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 8, minWidth: 0 }}>
            <span style={{ color: colors.accent, fontSize: '0.85rem', flexShrink: 0 }}>{'\u2666'}</span>
            <span style={{
              color: colors.textBright,
              fontWeight: 600,
              fontSize: '0.82rem',
              overflow: 'hidden',
              textOverflow: 'ellipsis',
              whiteSpace: 'nowrap',
            }}>
              {projectName}
            </span>
          </div>
          <div style={{
            display: 'inline-flex',
            alignItems: 'center',
            gap: 6,
            padding: '2px 8px',
            borderRadius: 10,
            backgroundColor: 'rgba(255,255,255,0.05)',
            flexShrink: 0,
            marginLeft: 10,
          }}>
            <span style={{
              display: 'inline-block',
              width: 6,
              height: 6,
              borderRadius: '50%',
              backgroundColor: badgeColor,
            }} />
            <span style={{ color: badgeColor, fontSize: '0.65rem', fontWeight: 600, whiteSpace: 'nowrap' }}>
              {badgeLabel}
            </span>
          </div>
        </div>
        <div style={{
          fontSize: '0.6rem',
          color: colors.muted,
          fontFamily: 'monospace',
          overflow: 'hidden',
          textOverflow: 'ellipsis',
          whiteSpace: 'nowrap',
        }} title={session.cwd}>
          {cwdShort}
        </div>
      </div>

      {/* Last message preview */}
      {preview && (
        <div style={{
          padding: '10px 14px',
          fontSize: '0.75rem',
          color: colors.text,
          lineHeight: 1.5,
          borderBottom: `1px solid ${colors.borderSubtle}`,
          maxHeight: 80,
          overflow: 'hidden',
        }}>
          <span style={{ color: colors.muted, fontSize: '0.65rem', marginRight: 6 }}>
            {lastMsg?.role === 'user' ? 'You:' : 'Claude:'}
          </span>
          {preview}{preview.length >= 150 ? '...' : ''}
        </div>
      )}

      {/* Approval prompt */}
      {state === 'waiting_approval' && session.pendingApproval && session.pendingApproval.timestamp > approvalDismissedAt && (
        <div style={{
          padding: '10px 14px',
          borderBottom: `1px solid ${colors.borderSubtle}`,
        }}>
          <div style={{ fontSize: '0.72rem', color: colors.text, marginBottom: 8 }}>
            <span style={{ color: colors.warning, fontWeight: 600 }}>
              {session.pendingApproval.toolName}
            </span>
            {' wants to run'}
          </div>
          <div style={{ display: 'flex', gap: 8 }}>
            <button
              onClick={() => handleApproval(true)}
              style={{
                flex: 1,
                padding: '5px 0',
                borderRadius: 5,
                border: `1px solid ${colors.success}`,
                backgroundColor: 'transparent',
                color: colors.success,
                fontSize: '0.72rem',
                fontWeight: 600,
                cursor: 'pointer',
              }}
            >
              Allow
            </button>
            <button
              onClick={() => handleApproval(false)}
              style={{
                flex: 1,
                padding: '5px 0',
                borderRadius: 5,
                border: `1px solid ${colors.error}`,
                backgroundColor: 'transparent',
                color: colors.error,
                fontSize: '0.72rem',
                fontWeight: 600,
                cursor: 'pointer',
              }}
            >
              Deny
            </button>
          </div>
        </div>
      )}

      {/* Streaming/thinking indicator */}
      {(state === 'thinking' || state === 'streaming') && (
        <div style={{
          padding: '8px 14px',
          display: 'flex',
          alignItems: 'center',
          gap: 8,
          borderBottom: `1px solid ${colors.borderSubtle}`,
        }}>
          <span style={{
            display: 'inline-block',
            width: 12,
            height: 12,
            border: `1.5px solid ${state === 'streaming' ? colors.accent : colors.warning}`,
            borderTopColor: 'transparent',
            borderRadius: '50%',
            animation: 'claudeSpinner 0.8s linear infinite',
            flexShrink: 0,
          }} />
          <span style={{ color: colors.muted, fontSize: '0.72rem' }}>
            {state === 'streaming' ? 'Working...' : 'Thinking...'}
          </span>
        </div>
      )}

      {/* Message input for idle sessions */}
      {(state === 'idle' || state === 'waiting_input') && (
        <div style={{ padding: '8px 14px', borderBottom: `1px solid ${colors.borderSubtle}` }}>
          <div style={{ display: 'flex', gap: 6 }}>
            <input
              ref={inputRef}
              value={inputValue}
              onChange={(e) => setInputValue(e.target.value)}
              onKeyDown={(e) => { if (e.key === 'Enter') handleSend(); }}
              placeholder="Send a message..."
              style={{
                flex: 1,
                padding: '5px 10px',
                borderRadius: 5,
                border: `1px solid ${colors.borderSubtle}`,
                backgroundColor: 'transparent',
                color: colors.text,
                fontSize: '0.72rem',
                outline: 'none',
                fontFamily: 'inherit',
              }}
            />
            <button
              onClick={handleSend}
              disabled={!inputValue.trim()}
              style={{
                padding: '5px 12px',
                borderRadius: 5,
                border: `1px solid ${colors.accent}`,
                backgroundColor: inputValue.trim() ? colors.accent : 'transparent',
                color: inputValue.trim() ? '#fff' : colors.muted,
                fontSize: '0.72rem',
                fontWeight: 600,
                cursor: inputValue.trim() ? 'pointer' : 'default',
                opacity: inputValue.trim() ? 1 : 0.5,
              }}
            >
              Send
            </button>
          </div>
        </div>
      )}

      {/* Footer — navigate to pane */}
      <div
        onClick={() => ownerTab && onNavigateToTab(ownerTab.id)}
        style={{
          padding: '7px 14px',
          fontSize: '0.68rem',
          color: colors.accent,
          cursor: ownerTab ? 'pointer' : 'default',
          opacity: ownerTab ? 1 : 0.4,
          textAlign: 'center',
          fontWeight: 500,
        }}
      >
        Go to pane {'\u2192'}
      </div>
    </div>
  );
};

// ── Dashboard Pane ──

const DashboardPane: React.FC<DashboardPaneProps> = ({ title, tabs, onNavigateToTab }) => {
  const [sessions, setSessions] = useState<ClaudeSessionSnapshot[]>([]);

  // Initial load + live updates
  useEffect(() => {
    window.electronAPI.getAllClaudeSessions().then((all) => {
      setSessions(all as ClaudeSessionSnapshot[]);
    });

    const unsub = window.electronAPI.onClaudeSessionUpdate((_ptyId, snapshot) => {
      setSessions(prev => {
        const idx = prev.findIndex(s => s.sessionId === (snapshot as ClaudeSessionSnapshot).sessionId);
        const next = [...prev];
        if (idx >= 0) {
          next[idx] = snapshot as ClaudeSessionSnapshot;
        } else {
          next.push(snapshot as ClaudeSessionSnapshot);
        }
        return next;
      });
    });

    return unsub;
  }, []);

  // Sort: approval first, then streaming/thinking, then idle
  const sorted = [...sessions]
    .filter(s => s.status !== 'ended')
    .sort((a, b) => {
      const priority = (s: ClaudeSessionSnapshot) => {
        if (s.ambientState === 'waiting_approval') return 0;
        if (s.ambientState === 'streaming' || s.ambientState === 'thinking') return 1;
        return 2;
      };
      return priority(a) - priority(b);
    });

  return (
    <div style={{
      width: '100%',
      height: '100%',
      overflow: 'auto',
      padding: '20px 24px',
      backgroundColor: colors.bg,
    }}>
      <div style={{
        fontSize: '0.75rem',
        color: colors.muted,
        marginBottom: 16,
        fontWeight: 500,
      }}>
        {sorted.length} active session{sorted.length !== 1 ? 's' : ''}
      </div>

      {sorted.length === 0 && (
        <div style={{
          textAlign: 'center',
          color: colors.muted,
          fontSize: '0.8rem',
          marginTop: 60,
        }}>
          No active Claude sessions. Open a Claude pane to get started.
        </div>
      )}

      <div style={{
        display: 'grid',
        gridTemplateColumns: 'repeat(auto-fill, minmax(320px, 1fr))',
        gap: 14,
      }}>
        {sorted.map(session => (
          <SessionCard
            key={session.sessionId}
            session={session}
            tabs={tabs}
            onNavigateToTab={onNavigateToTab}
          />
        ))}
      </div>
    </div>
  );
};

export default DashboardPane;
