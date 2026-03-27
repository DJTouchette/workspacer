import React, { useState, useEffect, useRef, useCallback } from 'react';
import type { ClaudeSessionSnapshot } from '../types/claudeSession';
import type { TrackerIssue, TrackerAccount } from '../types/tracker';
import type { TabConfig } from '../types/pane';
import { WriteTerminal } from '../lib/terminalApi';
import {
  claudeColors as colors,
  badgeColors,
  badgeLabels,
  ensureKeyframes,
  sendApproval,
} from '../components/claude-shared';

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

  const handleApproval = (approve: boolean) => {
    if (!session.ptyId) return;
    sendApproval(session.ptyId, approve, (data) => WriteTerminal(session.ptyId, data));
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

// ── Status pill color ──

function statusColor(cat: string): string {
  if (cat === 'done') return colors.success;
  if (cat === 'in_progress') return colors.accent;
  return colors.muted;
}

// ── My Issues Card ──

const MyIssuesCard: React.FC = () => {
  const [issues, setIssues] = useState<TrackerIssue[]>([]);
  const [accounts, setAccounts] = useState<TrackerAccount[]>([]);
  const [loading, setLoading] = useState(true);
  const [expanded, setExpanded] = useState(true);
  const [selectedIssue, setSelectedIssue] = useState<TrackerIssue | null>(null);

  const loadIssues = useCallback(async () => {
    setLoading(true);
    try {
      const accts = await window.electronAPI.trackerGetAccounts() as TrackerAccount[];
      setAccounts(accts);
      const allIssues: TrackerIssue[] = [];
      for (const acct of accts) {
        try {
          const issues = await window.electronAPI.trackerListIssues(acct.id, {
            assignedToMe: true,
            maxResults: 20,
          }) as TrackerIssue[];
          allIssues.push(...issues);
        } catch { /* skip failing accounts */ }
      }
      setIssues(allIssues);
    } catch {} finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => { loadIssues(); }, [loadIssues]);

  if (accounts.length === 0 && !loading) return null;

  const todoIssues = issues.filter(i => i.statusCategory !== 'done');
  const grouped = new Map<string, TrackerIssue[]>();
  for (const issue of todoIssues) {
    const key = issue.projectKey;
    const list = grouped.get(key) ?? [];
    list.push(issue);
    grouped.set(key, list);
  }

  return (
    <div style={{
      borderRadius: 10,
      border: `1px solid ${colors.borderSubtle}`,
      backgroundColor: 'rgba(255,255,255,0.02)',
      overflow: 'hidden',
    }}>
      <div
        onClick={() => setExpanded(!expanded)}
        style={{
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'space-between',
          padding: '10px 14px',
          borderBottom: expanded ? `1px solid ${colors.borderSubtle}` : 'none',
          cursor: 'pointer',
          userSelect: 'none',
        }}
      >
        <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
          <span style={{ fontSize: '0.82rem' }}>{'\u{1F4CB}'}</span>
          <span style={{ color: colors.textBright, fontWeight: 600, fontSize: '0.82rem' }}>
            My Issues
          </span>
          <span style={{ color: colors.muted, fontSize: '0.65rem' }}>
            {todoIssues.length}
          </span>
        </div>
        <span style={{
          fontSize: '0.6rem',
          color: colors.muted,
          transform: expanded ? 'rotate(90deg)' : 'rotate(0deg)',
          transition: 'transform 0.15s',
        }}>
          {'\u25B6'}
        </span>
      </div>

      {expanded && (
        <div style={{ maxHeight: 350, overflow: 'auto' }}>
          {loading && (
            <div style={{ padding: '12px 14px', color: colors.muted, fontSize: '0.72rem' }}>Loading...</div>
          )}
          {!loading && todoIssues.length === 0 && (
            <div style={{ padding: '12px 14px', color: colors.muted, fontSize: '0.72rem' }}>
              No issues assigned to you
            </div>
          )}
          {[...grouped.entries()].map(([projectKey, projectIssues]) => (
            <div key={projectKey}>
              <div style={{
                padding: '6px 14px 2px',
                fontSize: '0.6rem',
                fontWeight: 600,
                color: colors.muted,
                fontFamily: 'monospace',
                letterSpacing: '0.03em',
              }}>
                {projectKey}
              </div>
              {projectIssues.map(issue => (
                <div
                  key={issue.id}
                  onClick={() => setSelectedIssue(issue)}
                  style={{
                    padding: '5px 14px',
                    display: 'flex',
                    alignItems: 'center',
                    gap: 8,
                    fontSize: '0.72rem',
                    cursor: 'pointer',
                    minWidth: 0,
                  }}
                >
                  <span style={{
                    width: 6, height: 6, borderRadius: '50%',
                    backgroundColor: statusColor(issue.statusCategory),
                    flexShrink: 0,
                  }} />
                  <span style={{
                    color: colors.accent,
                    fontWeight: 600,
                    fontFamily: 'monospace',
                    fontSize: '0.65rem',
                    flexShrink: 0,
                  }}>
                    {issue.key}
                  </span>
                  <span style={{
                    color: colors.text,
                    overflow: 'hidden',
                    textOverflow: 'ellipsis',
                    whiteSpace: 'nowrap',
                    flex: 1,
                    minWidth: 0,
                  }}>
                    {issue.title}
                  </span>
                  <span style={{
                    fontSize: '0.58rem',
                    padding: '1px 6px',
                    borderRadius: 8,
                    backgroundColor: 'rgba(255,255,255,0.05)',
                    color: statusColor(issue.statusCategory),
                    fontWeight: 500,
                    flexShrink: 0,
                    whiteSpace: 'nowrap',
                  }}>
                    {issue.status}
                  </span>
                </div>
              ))}
            </div>
          ))}
          {!loading && (
            <div
              onClick={loadIssues}
              style={{
                padding: '6px 14px 8px',
                fontSize: '0.62rem',
                color: colors.accent,
                cursor: 'pointer',
                textAlign: 'center',
              }}
            >
              Refresh
            </div>
          )}
        </div>
      )}

      {/* Issue detail popup */}
      {selectedIssue && (
        <div
          onClick={() => setSelectedIssue(null)}
          style={{
            position: 'fixed', inset: 0, zIndex: 200,
            display: 'flex', alignItems: 'center', justifyContent: 'center',
            backgroundColor: 'rgba(0,0,0,0.5)',
          }}
        >
          <div
            onClick={e => e.stopPropagation()}
            style={{
              width: 480, maxHeight: '70vh', overflow: 'auto',
              borderRadius: 10, border: `1px solid ${colors.border}`,
              backgroundColor: 'var(--wks-bg-surface)', padding: '16px 20px',
            }}
          >
            <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 10 }}>
              <span style={{ color: colors.accent, fontWeight: 700, fontFamily: 'monospace', fontSize: '0.82rem' }}>
                {selectedIssue.key}
              </span>
              <span style={{
                fontSize: '0.6rem', fontWeight: 600, padding: '2px 8px', borderRadius: 10,
                backgroundColor: 'rgba(255,255,255,0.05)',
                color: statusColor(selectedIssue.statusCategory),
              }}>
                {selectedIssue.status}
              </span>
              <div style={{ flex: 1 }} />
              <span onClick={() => setSelectedIssue(null)} style={{ cursor: 'pointer', color: colors.muted, fontSize: '0.85rem' }}>{'\u00D7'}</span>
            </div>
            <div style={{ fontSize: '0.85rem', fontWeight: 600, color: colors.textBright, marginBottom: 8 }}>
              {selectedIssue.title}
            </div>
            <div style={{ fontSize: '0.65rem', color: colors.muted, marginBottom: 10, display: 'flex', gap: 12 }}>
              <span>Type: {selectedIssue.type}</span>
              {selectedIssue.priority && <span>Priority: {selectedIssue.priority}</span>}
              {selectedIssue.assignee && <span>Assignee: {selectedIssue.assignee}</span>}
            </div>
            {selectedIssue.labels.length > 0 && (
              <div style={{ display: 'flex', gap: 4, flexWrap: 'wrap', marginBottom: 10 }}>
                {selectedIssue.labels.map(l => (
                  <span key={l} style={{ fontSize: '0.58rem', padding: '1px 6px', borderRadius: 8, backgroundColor: 'rgba(255,255,255,0.06)', color: colors.text }}>{l}</span>
                ))}
              </div>
            )}
            {selectedIssue.description && (
              <div style={{
                fontSize: '0.72rem', lineHeight: 1.5, color: colors.text,
                whiteSpace: 'pre-wrap', padding: '10px 12px', borderRadius: 6,
                border: `1px solid ${colors.borderSubtle}`, backgroundColor: 'rgba(255,255,255,0.02)',
                maxHeight: 250, overflow: 'auto',
              }}>
                {selectedIssue.description}
              </div>
            )}
          </div>
        </div>
      )}
    </div>
  );
};

// ── Dashboard Pane ──

const DashboardPane: React.FC<DashboardPaneProps> = ({ title, tabs, onNavigateToTab }) => {
  const [sessions, setSessions] = useState<ClaudeSessionSnapshot[]>([]);

  useEffect(() => { ensureKeyframes(); }, []);

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
      {/* My Issues */}
      <div style={{ marginBottom: 20 }}>
        <MyIssuesCard />
      </div>

      {/* Claude Sessions */}
      <div style={{
        fontSize: '0.75rem',
        color: colors.muted,
        marginBottom: 12,
        fontWeight: 500,
      }}>
        {sorted.length} active session{sorted.length !== 1 ? 's' : ''}
      </div>

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
