import React, { useState, useEffect, useRef, useCallback } from 'react';
import type { ClaudeSessionSnapshot } from '../types/claudeSession';
import type { TrackerIssue, TrackerAccount } from '../types/tracker';
import type { TabConfig, PaneType } from '../types/pane';
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
  /** paneId → ptySessionId (for Claude panes, ptySessionId === Claude session id). */
  ptyMapping: Record<string, string>;
  onNavigateToTab: (tabId: string) => void;
  onAddTab?: (type: PaneType, shell?: string, label?: string, cwd?: string, profileId?: string, resumeSessionId?: string, attachSessionId?: string) => void;
}

// ── Session Card ──

const SessionCard: React.FC<{
  session: ClaudeSessionSnapshot;
  tabs: TabConfig[];
  ptyMapping: Record<string, string>;
  onNavigateToTab: (tabId: string) => void;
  onAddTab?: (type: PaneType, shell?: string, label?: string, cwd?: string, profileId?: string, resumeSessionId?: string, attachSessionId?: string) => void;
}> = ({ session, tabs, ptyMapping, onNavigateToTab, onAddTab }) => {
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

  // Find which tab contains this session. A pane is bound to a session if
  // either: (a) it was created with `resumeSessionId === sessionId` and that
  // resume is still active, or (b) the runtime ptyMapping for the pane's id
  // resolves to this session id (covers fresh sessions where resumeSessionId
  // was never set).
  const ownerTab = tabs.find((tab) =>
    tab.panes.some(
      (p) =>
        p.resumeSessionId === session.sessionId ||
        ptyMapping[p.id] === session.sessionId,
    ),
  );

  const handleGoToPane = () => {
    if (ownerTab) {
      onNavigateToTab(ownerTab.id);
      return;
    }
    if (onAddTab) {
      // No pane open for this session — create a viewer pane that ATTACHES
      // to the running daemon session (no new Claude process). Passing
      // attachSessionId (not resumeSessionId) ensures we don't try to spawn
      // a second claude with --resume on a session that's already attached.
      onAddTab('claude', undefined, undefined, session.cwd, undefined, undefined, session.sessionId);
    }
  };

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
        onClick={handleGoToPane}
        style={{
          padding: '7px 14px',
          fontSize: '0.68rem',
          color: colors.accent,
          cursor: (ownerTab || onAddTab) ? 'pointer' : 'default',
          opacity: (ownerTab || onAddTab) ? 1 : 0.4,
          textAlign: 'center',
          fontWeight: 500,
        }}
      >
        {ownerTab ? 'Go to pane' : 'Open in new pane'} {'\u2192'}
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

// ── Pipeline status colors ──

function pipelineColor(s: string): string {
  if (s === 'succeeded') return colors.success;
  if (s === 'failed') return colors.error;
  if (s === 'running') return colors.accent;
  if (s === 'queued') return colors.warning;
  return colors.muted;
}

// ── Recent Pipelines Card ──

const RecentPipelinesCard: React.FC = () => {
  const [pipelines, setPipelines] = useState<any[]>([]);
  const [loading, setLoading] = useState(true);
  const [expanded, setExpanded] = useState(true);
  const [selected, setSelected] = useState<any | null>(null);
  const [hasAccounts, setHasAccounts] = useState(false);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const accounts = await window.electronAPI.devopsGetAccounts();
      setHasAccounts(accounts.length > 0);
      if (accounts.length === 0) { setLoading(false); return; }

      // Try cache first
      let pl = await window.electronAPI.cacheRecentPipelines(15).catch(() => [] as any[]);

      // If cache empty, fetch directly from DevOps accounts
      if (!pl || pl.length === 0) {
        const all: any[] = [];
        for (const acct of accounts) {
          try {
            const runs = await window.electronAPI.devopsListPipelines(acct.id, { maxResults: 15 });
            all.push(...runs);
          } catch (e) {
            console.error('[Dashboard] pipeline fetch failed:', e);
          }
        }
        all.sort((a: any, b: any) => (b.startedAt ?? '').localeCompare(a.startedAt ?? ''));
        pl = all.slice(0, 15);
      }
      setPipelines(pl);
    } catch (e) {
      console.error('[Dashboard] pipeline load failed:', e);
    }
    setLoading(false);
  }, []);

  useEffect(() => { load(); }, [load]);

  // Hide entirely if no DevOps accounts connected
  if (!loading && !hasAccounts) return null;

  const running = pipelines.filter(p => (p.status ?? p.status) === 'running').length;
  const failed = pipelines.filter(p => (p.status ?? p.status) === 'failed').length;

  // Normalize field names (cache uses snake_case, live API uses camelCase)
  const norm = (p: any) => ({
    id: p.id,
    name: p.name,
    status: p.status,
    sourceBranch: p.source_branch ?? p.sourceBranch ?? '',
    commitSha: p.commit_sha ?? p.commitSha ?? '',
    author: p.author ?? '',
    url: p.url ?? '',
    startedAt: p.started_at ?? p.startedAt ?? '',
    finishedAt: p.finished_at ?? p.finishedAt ?? '',
  });

  return (
    <div style={{
      borderRadius: 10,
      border: `1px solid ${failed > 0 ? colors.error : colors.borderSubtle}`,
      backgroundColor: 'rgba(255,255,255,0.02)',
      overflow: 'hidden',
      transition: 'border-color 0.2s',
    }}>
      <div
        onClick={() => setExpanded(!expanded)}
        style={{
          display: 'flex', alignItems: 'center', justifyContent: 'space-between',
          padding: '10px 14px', cursor: 'pointer', userSelect: 'none',
          borderBottom: expanded ? `1px solid ${colors.borderSubtle}` : 'none',
        }}
      >
        <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
          <span style={{ fontSize: '0.82rem' }}>{'\u{1F527}'}</span>
          <span style={{ color: colors.textBright, fontWeight: 600, fontSize: '0.82rem' }}>Pipelines</span>
          {running > 0 && <span style={{ fontSize: '0.6rem', color: colors.accent, fontWeight: 600 }}>{running} running</span>}
          {failed > 0 && <span style={{ fontSize: '0.6rem', color: colors.error, fontWeight: 600 }}>{failed} failed</span>}
        </div>
        <span style={{ fontSize: '0.6rem', color: colors.muted, transform: expanded ? 'rotate(90deg)' : 'rotate(0deg)', transition: 'transform 0.15s' }}>
          {'\u25B6'}
        </span>
      </div>
      {expanded && (
        <div style={{ maxHeight: 300, overflow: 'auto' }}>
          {loading && <div style={{ padding: '12px 14px', color: colors.muted, fontSize: '0.72rem' }}>Loading...</div>}
          {pipelines.map(raw => {
            const p = norm(raw);
            const dur = p.startedAt && p.finishedAt
              ? Math.round((new Date(p.finishedAt).getTime() - new Date(p.startedAt).getTime()) / 1000)
              : null;
            const durStr = dur ? (dur < 60 ? `${dur}s` : `${Math.floor(dur / 60)}m ${dur % 60}s`) : '';
            return (
              <div key={p.id} onClick={() => setSelected(p)} style={{ padding: '5px 14px', display: 'flex', alignItems: 'center', gap: 6, fontSize: '0.72rem', cursor: 'pointer', minWidth: 0 }}>
                {p.status === 'running' ? (
                  <span style={{ width: 10, height: 10, border: `1.5px solid ${colors.accent}`, borderTopColor: 'transparent', borderRadius: '50%', animation: 'claudeSpinner 0.8s linear infinite', flexShrink: 0 }} />
                ) : (
                  <span style={{ width: 6, height: 6, borderRadius: '50%', backgroundColor: pipelineColor(p.status), flexShrink: 0 }} />
                )}
                <span style={{ color: colors.text, flex: 1, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', minWidth: 0 }}>
                  {p.name}
                </span>
                <span style={{ fontSize: '0.55rem', color: colors.muted, fontFamily: 'monospace', flexShrink: 1, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', maxWidth: 150 }}>
                  {p.sourceBranch}
                </span>
                {durStr && <span style={{ fontSize: '0.55rem', color: colors.muted, flexShrink: 0, whiteSpace: 'nowrap' }}>{durStr}</span>}
                <span style={{
                  fontSize: '0.55rem', fontWeight: 600, padding: '1px 5px', borderRadius: 6,
                  backgroundColor: 'rgba(255,255,255,0.05)', color: pipelineColor(p.status), flexShrink: 0, whiteSpace: 'nowrap',
                }}>
                  {p.status}
                </span>
              </div>
            );
          })}
          {!loading && (
            <div onClick={load} style={{ padding: '6px 14px 8px', fontSize: '0.62rem', color: colors.accent, cursor: 'pointer', textAlign: 'center' }}>
              Refresh
            </div>
          )}
        </div>
      )}

      {/* Pipeline detail popup */}
      {selected && (
        <div
          onClick={() => setSelected(null)}
          style={{
            position: 'fixed', inset: 0, zIndex: 200,
            display: 'flex', alignItems: 'center', justifyContent: 'center',
            backgroundColor: 'rgba(0,0,0,0.5)',
          }}
        >
          <div onClick={e => e.stopPropagation()} style={{
            width: 460, maxHeight: '70vh', overflow: 'auto',
            borderRadius: 10, border: `1px solid ${colors.border}`,
            backgroundColor: 'var(--wks-bg-surface)', padding: '16px 20px',
          }}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 12 }}>
              <span style={{ width: 8, height: 8, borderRadius: '50%', backgroundColor: pipelineColor(selected.status) }} />
              <span style={{ fontSize: '0.82rem', fontWeight: 600, color: colors.textBright, flex: 1 }}>{selected.name}</span>
              <span style={{
                fontSize: '0.6rem', fontWeight: 600, padding: '2px 8px', borderRadius: 10,
                backgroundColor: 'rgba(255,255,255,0.05)', color: pipelineColor(selected.status),
              }}>
                {selected.status}
              </span>
              <span onClick={() => setSelected(null)} style={{ cursor: 'pointer', color: colors.muted, fontSize: '0.85rem' }}>{'\u00D7'}</span>
            </div>

            <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: '8px 16px', fontSize: '0.68rem', marginBottom: 14 }}>
              <div><span style={{ color: colors.muted }}>Branch: </span><span style={{ color: colors.text, fontFamily: 'monospace' }}>{selected.sourceBranch}</span></div>
              {selected.commitSha && <div><span style={{ color: colors.muted }}>Commit: </span><span style={{ color: colors.text, fontFamily: 'monospace' }}>{selected.commitSha}</span></div>}
              {selected.author && <div><span style={{ color: colors.muted }}>Author: </span><span style={{ color: colors.text }}>{selected.author}</span></div>}
              {selected.startedAt && <div><span style={{ color: colors.muted }}>Started: </span><span style={{ color: colors.text }}>{new Date(selected.startedAt).toLocaleString()}</span></div>}
              {selected.finishedAt && <div><span style={{ color: colors.muted }}>Finished: </span><span style={{ color: colors.text }}>{new Date(selected.finishedAt).toLocaleString()}</span></div>}
              {selected.startedAt && selected.finishedAt && (() => {
                const dur = Math.round((new Date(selected.finishedAt).getTime() - new Date(selected.startedAt).getTime()) / 1000);
                return <div><span style={{ color: colors.muted }}>Duration: </span><span style={{ color: colors.text }}>{dur < 60 ? `${dur}s` : `${Math.floor(dur / 60)}m ${dur % 60}s`}</span></div>;
              })()}
            </div>

            {selected.url && (
              <div
                onClick={() => { /* Can't open external URLs from renderer — copy to clipboard instead */ navigator.clipboard.writeText(selected.url); }}
                style={{
                  padding: '8px 12px', borderRadius: 6,
                  border: `1px solid ${colors.borderSubtle}`, backgroundColor: 'rgba(255,255,255,0.02)',
                  fontSize: '0.65rem', color: colors.accent, fontFamily: 'monospace',
                  cursor: 'pointer', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
                }}
                title="Click to copy URL"
              >
                {selected.url}
              </div>
            )}
          </div>
        </div>
      )}
    </div>
  );
};

// ── Dashboard Pane ──

// ── Agent Run Card ──

interface AgentRun {
  id: string;
  prompt_id: string;
  prompt_snapshot: string;
  rendered_prompt: string;
  cwd: string;
  model: string;
  claude_session_id: string;
  status: string;
  error: string;
  started_at: string;
}

const RunCard: React.FC<{
  run: AgentRun;
  session?: ClaudeSessionSnapshot;
  tabs: TabConfig[];
  ptyMapping: Record<string, string>;
  onNavigateToTab: (tabId: string) => void;
  onAddTab?: (type: PaneType, shell?: string, label?: string, cwd?: string, profileId?: string, resumeSessionId?: string, attachSessionId?: string) => void;
}> = ({ run, session, tabs, ptyMapping, onNavigateToTab, onAddTab }) => {
  const [inputValue, setInputValue] = useState('');
  const projectName = run.cwd.split(/[/\\]/).pop() || run.cwd;

  // Use live session state if available, else fall back to run status
  const ambientState = session?.ambientState;
  const isLive = !!session && session.status !== 'ended';

  const statusColors: Record<string, string> = {
    running: colors.accent,
    pending: '#e6a700',
    stopped: colors.muted,
    error: colors.error,
  };

  const liveColor = ambientState === 'waiting_approval' ? colors.error
    : (ambientState === 'streaming' || ambientState === 'thinking') ? colors.accent
    : colors.muted;
  const borderColor = isLive
    ? (ambientState === 'waiting_approval' ? colors.error : liveColor + '60')
    : (run.status === 'error' ? colors.error + '60' : colors.borderSubtle);
  const statusColor = isLive ? liveColor : (statusColors[run.status] ?? colors.muted);
  const statusLabel = isLive ? (badgeLabels[ambientState!] ?? ambientState ?? run.status) : run.status;

  // Find existing Claude pane for this session — match by stored resume id
  // OR by the runtime paneId→sessionId mapping (covers fresh sessions whose
  // session id wasn't known when the pane was created).
  const ownerTab = run.claude_session_id
    ? tabs.find((tab) =>
        tab.panes.some(
          (p) =>
            p.resumeSessionId === run.claude_session_id ||
            ptyMapping[p.id] === run.claude_session_id,
        ),
      )
    : undefined;

  const handleGoToPane = () => {
    if (ownerTab) {
      onNavigateToTab(ownerTab.id);
    } else if (run.claude_session_id && onAddTab) {
      // Attach as a viewer to the live daemon session — don't respawn claude.
      onAddTab('claude', undefined, `Run: ${projectName}`, run.cwd, undefined, undefined, run.claude_session_id);
    }
  };

  // Last message preview from session
  const lastMsg = session?.conversation ? [...session.conversation].reverse().find(t => t.content) : null;
  const preview = lastMsg?.content?.slice(0, 150) ?? run.rendered_prompt.slice(0, 120);

  const handleSend = () => {
    if (!inputValue.trim() || !session?.ptyId) return;
    WriteTerminal(session.ptyId, inputValue);
    setTimeout(() => WriteTerminal(session.ptyId, '\r'), 50);
    setInputValue('');
  };

  const handleApproval = (approve: boolean) => {
    if (!session?.ptyId) return;
    sendApproval(session.ptyId, approve, (data) => WriteTerminal(session.ptyId, data));
  };

  return (
    <div style={{
      borderRadius: 10,
      border: `1px solid ${borderColor}`,
      backgroundColor: 'rgba(255,255,255,0.02)',
      overflow: 'hidden',
      transition: 'border-color 0.2s',
    }}>
      {/* Header */}
      <div style={{ padding: '10px 14px', borderBottom: `1px solid ${colors.borderSubtle}` }}>
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
          <span style={{ fontSize: '0.82rem', fontWeight: 600, color: colors.text }}>
            {projectName}
          </span>
          <span style={{
            fontSize: '0.65rem',
            padding: '2px 8px',
            borderRadius: 9,
            backgroundColor: statusColor + '22',
            color: statusColor,
            fontWeight: 600,
          }}>
            {statusLabel}
          </span>
        </div>
        {run.model && (
          <div style={{ fontSize: '0.68rem', color: colors.muted, marginTop: 2, fontFamily: 'monospace' }}>
            {run.model}
          </div>
        )}
      </div>

      {/* Live message preview */}
      <div style={{ padding: '8px 14px', fontSize: '0.72rem', color: colors.muted, lineHeight: 1.4 }}>
        {preview}{preview.length >= 120 ? '...' : ''}
      </div>

      {/* Approval buttons (when waiting) */}
      {isLive && ambientState === 'waiting_approval' && session?.pendingApproval && (
        <div style={{
          display: 'flex', gap: 8, padding: '6px 14px 10px',
        }}>
          <button
            onClick={() => handleApproval(true)}
            style={{
              flex: 1, padding: '5px 0', borderRadius: 6,
              border: `1px solid ${colors.success}`,
              backgroundColor: colors.success + '18',
              color: colors.success, fontSize: '0.7rem', fontWeight: 600, cursor: 'pointer',
            }}
          >
            Approve
          </button>
          <button
            onClick={() => handleApproval(false)}
            style={{
              flex: 1, padding: '5px 0', borderRadius: 6,
              border: `1px solid ${colors.error}`,
              backgroundColor: colors.error + '18',
              color: colors.error, fontSize: '0.7rem', fontWeight: 600, cursor: 'pointer',
            }}
          >
            Deny
          </button>
        </div>
      )}

      {/* Inline input (when idle or waiting_input) */}
      {isLive && (ambientState === 'idle' || ambientState === 'waiting_input') && session?.ptyId && (
        <div style={{ display: 'flex', gap: 6, padding: '4px 14px 10px' }}>
          <input
            value={inputValue}
            onChange={e => setInputValue(e.target.value)}
            onKeyDown={e => { if (e.key === 'Enter') handleSend(); }}
            placeholder="Send message..."
            style={{
              flex: 1, padding: '5px 10px', borderRadius: 6,
              border: `1px solid ${colors.borderSubtle}`,
              backgroundColor: 'transparent', color: colors.text,
              fontSize: '0.72rem', outline: 'none',
            }}
          />
          <button
            onClick={handleSend}
            style={{
              padding: '5px 12px', borderRadius: 6, border: 'none',
              backgroundColor: inputValue.trim() ? colors.accent : 'transparent',
              color: inputValue.trim() ? '#fff' : colors.muted,
              fontSize: '0.72rem', fontWeight: 600, cursor: inputValue.trim() ? 'pointer' : 'default',
            }}
          >
            Send
          </button>
        </div>
      )}

      {/* Spinner for thinking/streaming */}
      {isLive && (ambientState === 'thinking' || ambientState === 'streaming') && (
        <div style={{ padding: '4px 14px 8px', fontSize: '0.68rem', color: colors.accent }}>
          {ambientState === 'thinking' ? 'Thinking...' : 'Streaming...'}
        </div>
      )}

      {/* Error */}
      {run.error && !isLive && (
        <div style={{ padding: '4px 14px 8px', fontSize: '0.68rem', color: colors.error }}>
          {run.error.slice(0, 100)}
        </div>
      )}

      {/* Footer — go to pane */}
      {run.claude_session_id && (
        <div
          onClick={handleGoToPane}
          style={{
            padding: '7px 14px',
            fontSize: '0.68rem',
            color: colors.accent,
            cursor: 'pointer',
            textAlign: 'center',
            fontWeight: 500,
            borderTop: `1px solid ${colors.borderSubtle}`,
          }}
        >
          {ownerTab ? 'Go to pane' : 'Open in pane'} {'\u2192'}
        </div>
      )}
    </div>
  );
};

// ── Agent Runs Section ──

const AgentRunsCard: React.FC<{
  tabs: TabConfig[];
  sessions: ClaudeSessionSnapshot[];
  ptyMapping: Record<string, string>;
  onNavigateToTab: (tabId: string) => void;
  onAddTab?: (type: PaneType, shell?: string, label?: string, cwd?: string, profileId?: string, resumeSessionId?: string, attachSessionId?: string) => void;
}> = ({ tabs, sessions, ptyMapping, onNavigateToTab, onAddTab }) => {
  const [runs, setRuns] = useState<AgentRun[]>([]);

  useEffect(() => {
    const fetchRuns = () => {
      fetch('http://127.0.0.1:9800/api/runs')
        .then(r => r.ok ? r.json() : null)
        .then(data => { if (data?.runs) setRuns(data.runs); })
        .catch(() => {});
    };
    fetchRuns();
    const interval = setInterval(fetchRuns, 5000);
    return () => clearInterval(interval);
  }, []);

  const active = runs.filter(r => r.status === 'running' || r.status === 'pending');
  const recent = runs.filter(r => r.status !== 'running' && r.status !== 'pending').slice(0, 4);

  if (runs.length === 0) return null;

  return (
    <div>
      <div style={{ fontSize: '0.75rem', color: colors.muted, marginBottom: 12, fontWeight: 500 }}>
        {active.length} active run{active.length !== 1 ? 's' : ''}
        {recent.length > 0 && ` · ${recent.length} recent`}
      </div>
      <div style={{
        display: 'grid',
        gridTemplateColumns: 'repeat(auto-fill, minmax(300px, 1fr))',
        gap: 14,
      }}>
        {[...active, ...recent].map(run => {
          // Match run to live Claude session by sessionId
          const session = run.claude_session_id
            ? sessions.find(s => s.sessionId === run.claude_session_id)
            : undefined;
          return (
            <RunCard key={run.id} run={run} session={session} tabs={tabs} ptyMapping={ptyMapping} onNavigateToTab={onNavigateToTab} onAddTab={onAddTab} />
          );
        })}
      </div>
    </div>
  );
};

const DashboardPane: React.FC<DashboardPaneProps> = ({ title: _t, tabs, ptyMapping, onNavigateToTab, onAddTab }) => {
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
      boxSizing: 'border-box',
    }}>
      {/* My Issues */}
      <div style={{ marginBottom: 14 }}>
        <MyIssuesCard />
      </div>

      {/* Recent Pipelines */}
      <div style={{ marginBottom: 20 }}>
        <RecentPipelinesCard />
      </div>

      {/* Agent Runs */}
      <div style={{ marginBottom: 20 }}>
        <AgentRunsCard tabs={tabs} sessions={sessions} ptyMapping={ptyMapping} onNavigateToTab={onNavigateToTab} onAddTab={onAddTab} />
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
            ptyMapping={ptyMapping}
            onNavigateToTab={onNavigateToTab}
            onAddTab={onAddTab}
          />
        ))}
      </div>
    </div>
  );
};

export default DashboardPane;
