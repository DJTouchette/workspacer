import React, { useState, useEffect, useCallback } from 'react';
import type { TrackerAccount, TrackerProject, TrackerIssue } from '../types/tracker';
import { claudeColors as colors, ensureKeyframes } from '../components/claude-shared';

/**
 * TrackerPane — Jira (and Azure DevOps PRs/pipelines) browser.
 *
 * Credentials and API access are owned by the devdaemon (Go). This pane never
 * stores tokens — it talks to the daemon via IPC and renders normalized
 * issues. If a source is unauthenticated, the daemon returns no account for
 * that source and we surface a "configure with CLI" banner.
 */

type View =
  | { kind: 'sources' }
  | { kind: 'projects'; accountId: string; sourceLabel: string }
  | { kind: 'issues'; accountId: string; sourceLabel: string; projectKey: string; projectName: string }
  | { kind: 'issue-detail'; accountId: string; sourceLabel: string; issue: TrackerIssue };

function statusColor(cat: string): string {
  if (cat === 'done') return colors.success;
  if (cat === 'in_progress') return colors.accent;
  return colors.muted;
}

// ── Source list (replaces the old AccountList) ──

const SourceList: React.FC<{
  accounts: TrackerAccount[];
  loading: boolean;
  onSelect: (account: TrackerAccount) => void;
  onRefresh: () => void;
}> = ({ accounts, loading, onSelect, onRefresh }) => {
  const hasJira = accounts.some((a) => a.provider === 'jira');
  const hasAdo = accounts.some((a) => a.provider === 'ado');

  return (
    <div style={{ padding: '20px 24px' }}>
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 16 }}>
        <div style={{ fontSize: '0.9rem', fontWeight: 700, color: colors.textBright }}>Issue Tracker</div>
        <button onClick={onRefresh} style={btnStyle(colors.muted, true)} title="Re-check daemon auth">
          {loading ? '···' : 'Refresh'}
        </button>
      </div>

      <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
        {accounts.map((account) => (
          <div
            key={account.id}
            onClick={() => onSelect(account)}
            style={{
              padding: '12px 14px',
              borderRadius: 8,
              border: `1px solid ${colors.borderSubtle}`,
              backgroundColor: 'rgba(255,255,255,0.02)',
              cursor: 'pointer',
              display: 'flex',
              alignItems: 'center',
              justifyContent: 'space-between',
            }}
          >
            <div>
              <div style={{ fontSize: '0.8rem', fontWeight: 600, color: colors.textBright }}>
                {account.provider === 'jira' ? 'Jira' : 'Azure DevOps'}
              </div>
              <div style={{ fontSize: '0.65rem', color: colors.muted, marginTop: 2 }}>
                Credentials managed by devdaemon
              </div>
            </div>
            <span style={{ fontSize: '0.65rem', color: colors.accent }}>{'→'}</span>
          </div>
        ))}
      </div>

      {!hasJira && <MissingAuthBanner source="jira" />}
      {!hasAdo && <MissingAuthBanner source="ado" />}

      {accounts.length === 0 && !loading && (
        <div style={{ textAlign: 'center', marginTop: 24, color: colors.muted, fontSize: '0.75rem' }}>
          Neither Jira nor Azure DevOps is authenticated with the devdaemon.
        </div>
      )}
    </div>
  );
};

const MissingAuthBanner: React.FC<{ source: 'jira' | 'ado' }> = ({ source }) => {
  const label = source === 'jira' ? 'Jira' : 'Azure DevOps';
  const cmd = source === 'jira' ? 'devdaemon auth jira --token <api-token>' : 'devdaemon auth ado --pat <pat>';
  return (
    <div
      style={{
        marginTop: 16,
        padding: '12px 14px',
        borderRadius: 8,
        border: `1px solid ${colors.borderSubtle}`,
        backgroundColor: 'rgba(255,255,255,0.02)',
      }}
    >
      <div style={{ fontSize: '0.72rem', color: colors.textBright, marginBottom: 6, fontWeight: 600 }}>
        {label} not authenticated
      </div>
      <div style={{ fontSize: '0.68rem', color: colors.muted, marginBottom: 8 }}>
        Run the CLI command below to give the devdaemon a token, then click Refresh.
      </div>
      <code
        style={{
          display: 'block',
          fontSize: '0.7rem',
          fontFamily: 'monospace',
          color: colors.text,
          backgroundColor: 'rgba(0,0,0,0.3)',
          padding: '6px 8px',
          borderRadius: 4,
          whiteSpace: 'pre-wrap',
          wordBreak: 'break-all',
        }}
      >
        {cmd}
      </code>
    </div>
  );
};

// ── Project List ──

const ProjectList: React.FC<{
  accountId: string;
  sourceLabel: string;
  onSelect: (projectKey: string, projectName: string) => void;
  onBack: () => void;
}> = ({ accountId, sourceLabel, onSelect, onBack }) => {
  const [projects, setProjects] = useState<TrackerProject[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');

  useEffect(() => {
    setLoading(true);
    window.electronAPI
      .trackerListProjects(accountId)
      .then((p) => setProjects(p))
      .catch((e) => setError(e?.message ?? 'Failed to load projects'))
      .finally(() => setLoading(false));
  }, [accountId]);

  return (
    <div style={{ padding: '20px 24px' }}>
      <div style={{ display: 'flex', alignItems: 'center', gap: 10, marginBottom: 16 }}>
        <button onClick={onBack} style={backBtnStyle}>{'←'}</button>
        <div style={{ fontSize: '0.9rem', fontWeight: 700, color: colors.textBright }}>
          {sourceLabel} Projects
        </div>
      </div>

      {loading && <div style={{ color: colors.muted, fontSize: '0.75rem' }}>Loading projects...</div>}
      {error && <div style={{ color: colors.error, fontSize: '0.75rem' }}>{error}</div>}

      <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
        {projects.map((p) => (
          <div
            key={p.id}
            onClick={() => onSelect(p.key, p.name)}
            style={{
              padding: '10px 14px',
              borderRadius: 8,
              border: `1px solid ${colors.borderSubtle}`,
              backgroundColor: 'rgba(255,255,255,0.02)',
              cursor: 'pointer',
              display: 'flex',
              alignItems: 'center',
              gap: 10,
            }}
          >
            <span style={{ color: colors.accent, fontWeight: 700, fontFamily: 'monospace', fontSize: '0.75rem', minWidth: 50 }}>
              {p.key}
            </span>
            <span style={{ color: colors.text, fontSize: '0.78rem' }}>{p.name}</span>
          </div>
        ))}
      </div>
    </div>
  );
};

// ── Issue List ──

const IssueList: React.FC<{
  accountId: string;
  projectKey: string;
  projectName: string;
  onSelect: (issue: TrackerIssue) => void;
  onBack: () => void;
}> = ({ accountId, projectKey, projectName, onSelect, onBack }) => {
  const [issues, setIssues] = useState<TrackerIssue[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const [search, setSearch] = useState('');

  const load = useCallback(
    (query?: string) => {
      setLoading(true);
      setError('');
      const opts = query ? { projectKey, query, maxResults: 30 } : { projectKey, maxResults: 50 };
      window.electronAPI
        .trackerListIssues(accountId, opts)
        .then((i) => setIssues(i))
        .catch((e) => setError(e?.message ?? 'Failed to load issues'))
        .finally(() => setLoading(false));
    },
    [accountId, projectKey],
  );

  useEffect(() => { load(); }, [load]);

  const handleSearch = () => { if (search.trim()) load(search.trim()); else load(); };

  return (
    <div style={{ padding: '20px 24px' }}>
      <div style={{ display: 'flex', alignItems: 'center', gap: 10, marginBottom: 12 }}>
        <button onClick={onBack} style={backBtnStyle}>{'←'}</button>
        <div>
          <div style={{ fontSize: '0.9rem', fontWeight: 700, color: colors.textBright }}>{projectName}</div>
          <div style={{ fontSize: '0.6rem', color: colors.muted, fontFamily: 'monospace' }}>{projectKey}</div>
        </div>
      </div>

      <div style={{ display: 'flex', gap: 6, marginBottom: 14 }}>
        <input
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          onKeyDown={(e) => { if (e.key === 'Enter') handleSearch(); }}
          placeholder="Search issues..."
          style={{ ...inputStyle, flex: 1 }}
        />
        <button onClick={handleSearch} style={btnStyle(colors.accent, true)}>Search</button>
      </div>

      {loading && <div style={{ color: colors.muted, fontSize: '0.75rem' }}>Loading...</div>}
      {error && <div style={{ color: colors.error, fontSize: '0.75rem', marginBottom: 8 }}>{error}</div>}
      {!loading && !error && issues.length === 0 && (
        <div style={{ color: colors.muted, fontSize: '0.75rem', marginTop: 12 }}>No issues found</div>
      )}

      <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
        {issues.map((issue) => (
          <div
            key={issue.id}
            onClick={() => onSelect(issue)}
            style={{
              padding: '8px 12px',
              borderRadius: 6,
              border: `1px solid ${colors.borderSubtle}`,
              backgroundColor: 'rgba(255,255,255,0.02)',
              cursor: 'pointer',
              display: 'flex',
              alignItems: 'center',
              gap: 10,
            }}
          >
            <span style={{
              color: colors.accent,
              fontWeight: 600,
              fontFamily: 'monospace',
              fontSize: '0.7rem',
              minWidth: 70,
              flexShrink: 0,
            }}>
              {issue.key}
            </span>
            <span style={{
              color: colors.text,
              fontSize: '0.75rem',
              flex: 1,
              overflow: 'hidden',
              textOverflow: 'ellipsis',
              whiteSpace: 'nowrap',
            }}>
              {issue.title}
            </span>
            <span style={{
              fontSize: '0.6rem',
              fontWeight: 600,
              padding: '2px 6px',
              borderRadius: 8,
              backgroundColor: 'rgba(255,255,255,0.05)',
              color: statusColor(issue.statusCategory),
              whiteSpace: 'nowrap',
              flexShrink: 0,
            }}>
              {issue.status}
            </span>
          </div>
        ))}
      </div>
    </div>
  );
};

// ── Issue Detail ──

interface Transition { id: string; name: string; to: { id: string; name: string; category: string } }

const IssueDetail: React.FC<{
  issue: TrackerIssue;
  onBack: () => void;
}> = ({ issue, onBack }) => {
  const [transitions, setTransitions] = useState<Transition[]>([]);
  const [currentStatus, setCurrentStatus] = useState(issue.status);
  const [currentCategory, setCurrentCategory] = useState(issue.statusCategory);
  const [transitioning, setTransitioning] = useState(false);
  const [showTransitions, setShowTransitions] = useState(false);
  const [childIssues, setChildIssues] = useState<TrackerIssue[]>([]);
  const [issueLinks, setIssueLinks] = useState<Array<{ issue_key: string; link_type: string; link_id: string; link_label: string }>>([]);

  useEffect(() => {
    window.electronAPI.cacheGetIssueLinks(issue.key)
      .then((links) => setIssueLinks(links.filter((l: any) => l.link_type !== 'parent' && l.link_type !== 'child')))
      .catch(() => {});
    window.electronAPI.cacheGetChildIssues(issue.key)
      .then((children) => setChildIssues(children as TrackerIssue[]))
      .catch(() => {});
  }, [issue.key]);

  useEffect(() => {
    window.electronAPI.trackerGetTransitions(issue.accountId, issue.key)
      .then((t) => setTransitions(t))
      .catch(() => {});
  }, [issue.accountId, issue.key]);

  const handleTransition = async (t: Transition) => {
    setTransitioning(true);
    try {
      await window.electronAPI.trackerTransitionIssue(issue.accountId, issue.key, t.id);
      setCurrentStatus(t.to.name);
      setCurrentCategory(t.to.category as any);
      setShowTransitions(false);
    } catch (e: any) {
      console.error('[TrackerPane] transition failed:', e);
    } finally {
      setTransitioning(false);
    }
  };

  return (
    <div style={{ padding: '20px 24px', maxWidth: 700 }}>
      <div style={{ display: 'flex', alignItems: 'center', gap: 10, marginBottom: 16 }}>
        <button onClick={onBack} style={backBtnStyle}>{'←'}</button>
        <span style={{ color: colors.accent, fontWeight: 700, fontFamily: 'monospace', fontSize: '0.8rem' }}>
          {issue.key}
        </span>
        <span
          onClick={() => setShowTransitions(!showTransitions)}
          style={{
            fontSize: '0.6rem',
            fontWeight: 600,
            padding: '2px 8px',
            borderRadius: 10,
            backgroundColor: 'rgba(255,255,255,0.05)',
            color: statusColor(currentCategory),
            cursor: transitions.length > 0 ? 'pointer' : 'default',
          }}
          title={transitions.length > 0 ? 'Click to change status' : undefined}
        >
          {currentStatus} {transitions.length > 0 ? '▾' : ''}
        </span>
      </div>

      {showTransitions && transitions.length > 0 && (
        <div style={{
          marginBottom: 14,
          padding: '8px 0',
          borderRadius: 8,
          border: `1px solid ${colors.borderSubtle}`,
          backgroundColor: 'rgba(255,255,255,0.03)',
        }}>
          <div style={{ fontSize: '0.62rem', color: colors.muted, padding: '0 12px 6px', fontWeight: 500 }}>
            Move to:
          </div>
          {transitions.map((t) => (
            <div
              key={t.id}
              onClick={() => !transitioning && handleTransition(t)}
              style={{
                padding: '6px 12px',
                fontSize: '0.72rem',
                color: colors.text,
                cursor: transitioning ? 'wait' : 'pointer',
                display: 'flex',
                alignItems: 'center',
                gap: 8,
                opacity: transitioning ? 0.5 : 1,
              }}
            >
              <span style={{
                width: 6, height: 6, borderRadius: '50%',
                backgroundColor: statusColor(t.to.category),
                flexShrink: 0,
              }} />
              {t.name}
              <span style={{ fontSize: '0.6rem', color: colors.muted }}>{'→'} {t.to.name}</span>
            </div>
          ))}
        </div>
      )}

      <div style={{ fontSize: '0.9rem', fontWeight: 700, color: colors.textBright, marginBottom: 12 }}>
        {issue.title}
      </div>

      <div style={{ display: 'flex', gap: 16, marginBottom: 16, fontSize: '0.68rem', color: colors.muted }}>
        <span>Type: <span style={{ color: colors.text }}>{issue.type}</span></span>
        {issue.priority && <span>Priority: <span style={{ color: colors.text }}>{issue.priority}</span></span>}
        {issue.assignee && <span>Assignee: <span style={{ color: colors.text }}>{issue.assignee}</span></span>}
      </div>

      {issue.labels.length > 0 && (
        <div style={{ display: 'flex', gap: 4, flexWrap: 'wrap', marginBottom: 16 }}>
          {issue.labels.map((l) => (
            <span key={l} style={{
              fontSize: '0.6rem',
              padding: '1px 6px',
              borderRadius: 8,
              backgroundColor: 'rgba(255,255,255,0.06)',
              color: colors.text,
            }}>
              {l}
            </span>
          ))}
        </div>
      )}

      {issue.description && (
        <div style={{
          fontSize: '0.78rem',
          lineHeight: 1.6,
          color: colors.text,
          whiteSpace: 'pre-wrap',
          padding: '12px 14px',
          borderRadius: 8,
          border: `1px solid ${colors.borderSubtle}`,
          backgroundColor: 'rgba(255,255,255,0.02)',
          maxHeight: 400,
          overflow: 'auto',
        }}>
          {issue.description}
        </div>
      )}

      {issue.parentKey && (
        <div style={{ marginTop: 14, fontSize: '0.68rem', color: colors.muted }}>
          Parent: <span style={{ color: colors.accent, fontFamily: 'monospace', fontWeight: 600 }}>{issue.parentKey}</span>
        </div>
      )}

      {childIssues.length > 0 && (
        <div style={{ marginTop: 14 }}>
          <div style={{ fontSize: '0.62rem', color: colors.muted, fontWeight: 600, marginBottom: 4 }}>
            Subtasks ({childIssues.length})
          </div>
          {childIssues.map((child) => (
            <div key={child.key} style={{ display: 'flex', alignItems: 'center', gap: 6, padding: '3px 0', fontSize: '0.7rem' }}>
              <span style={{
                width: 6, height: 6, borderRadius: '50%',
                backgroundColor: statusColor((child as any).status_category ?? child.statusCategory ?? 'todo'),
                flexShrink: 0,
              }} />
              <span style={{ color: colors.accent, fontFamily: 'monospace', fontWeight: 600, fontSize: '0.65rem' }}>{child.key}</span>
              <span style={{ color: colors.text }}>{child.title}</span>
            </div>
          ))}
        </div>
      )}

      {issueLinks.length > 0 && (
        <div style={{ marginTop: 14 }}>
          <div style={{ fontSize: '0.62rem', color: colors.muted, fontWeight: 600, marginBottom: 4 }}>
            Links ({issueLinks.length})
          </div>
          {issueLinks.map((link, i) => (
            <div key={i} style={{ display: 'flex', alignItems: 'center', gap: 6, padding: '3px 0', fontSize: '0.68rem' }}>
              <span style={{
                color: link.link_type === 'pr' ? '#c084fc' : link.link_type === 'pipeline' ? colors.warning : colors.accent,
                fontWeight: 600, fontSize: '0.6rem', minWidth: 50,
              }}>
                {link.link_type === 'pr' ? 'PR' : link.link_type === 'pipeline' ? 'Build' : 'Branch'}
              </span>
              <span style={{ color: colors.text, fontFamily: 'monospace', fontSize: '0.65rem' }}>
                {link.link_label}
              </span>
            </div>
          ))}
        </div>
      )}

      <div style={{ marginTop: 12, fontSize: '0.62rem', color: colors.muted }}>
        Updated {new Date(issue.updated).toLocaleString()}
      </div>
    </div>
  );
};

// ── Shared styles ──

const inputStyle: React.CSSProperties = {
  padding: '6px 10px',
  borderRadius: 6,
  border: `1px solid ${colors.borderSubtle}`,
  backgroundColor: 'rgba(255,255,255,0.03)',
  color: colors.text,
  fontSize: '0.75rem',
  outline: 'none',
  fontFamily: 'inherit',
  width: '100%',
  boxSizing: 'border-box',
};

function btnStyle(color: string, enabled: boolean): React.CSSProperties {
  return {
    padding: '5px 14px',
    borderRadius: 6,
    border: `1px solid ${color}`,
    backgroundColor: 'transparent',
    color,
    fontSize: '0.7rem',
    fontWeight: 600,
    cursor: enabled ? 'pointer' : 'default',
    opacity: enabled ? 1 : 0.4,
    fontFamily: 'inherit',
  };
}

const backBtnStyle: React.CSSProperties = {
  padding: '2px 8px',
  borderRadius: 4,
  border: `1px solid ${colors.borderSubtle}`,
  backgroundColor: 'transparent',
  color: colors.muted,
  fontSize: '0.75rem',
  cursor: 'pointer',
  fontFamily: 'inherit',
};

// ── Main component ──

interface TrackerPaneProps {
  paneId: string;
  title: string;
  isActive: boolean;
}

const TrackerPane: React.FC<TrackerPaneProps> = ({ paneId: _p, title: _t, isActive: _a }) => {
  const [view, setView] = useState<View>({ kind: 'sources' });
  const [accounts, setAccounts] = useState<TrackerAccount[]>([]);
  const [refreshing, setRefreshing] = useState(false);

  useEffect(() => { ensureKeyframes(); }, []);

  const refreshAccounts = useCallback(() => {
    setRefreshing(true);
    window.electronAPI
      .trackerGetAccounts()
      .then(setAccounts)
      .finally(() => setRefreshing(false));
  }, []);

  useEffect(() => { refreshAccounts(); }, [refreshAccounts]);

  return (
    <div
      style={{
        width: '100%',
        height: '100%',
        overflow: 'auto',
        backgroundColor: colors.bg,
        color: colors.text,
        fontFamily: '-apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif',
      }}
    >
      {view.kind === 'sources' && (
        <SourceList
          accounts={accounts}
          loading={refreshing}
          onSelect={(account) =>
            setView({
              kind: 'projects',
              accountId: account.id,
              sourceLabel: account.provider === 'jira' ? 'Jira' : 'Azure DevOps',
            })
          }
          onRefresh={refreshAccounts}
        />
      )}

      {view.kind === 'projects' && (
        <ProjectList
          accountId={view.accountId}
          sourceLabel={view.sourceLabel}
          onSelect={(key, name) =>
            setView({
              kind: 'issues',
              accountId: view.accountId,
              sourceLabel: view.sourceLabel,
              projectKey: key,
              projectName: name,
            })
          }
          onBack={() => setView({ kind: 'sources' })}
        />
      )}

      {view.kind === 'issues' && (
        <IssueList
          accountId={view.accountId}
          projectKey={view.projectKey}
          projectName={view.projectName}
          onSelect={(issue) =>
            setView({ kind: 'issue-detail', accountId: view.accountId, sourceLabel: view.sourceLabel, issue })
          }
          onBack={() =>
            setView({ kind: 'projects', accountId: view.accountId, sourceLabel: view.sourceLabel })
          }
        />
      )}

      {view.kind === 'issue-detail' && (
        <IssueDetail
          issue={view.issue}
          onBack={() =>
            setView({
              kind: 'issues',
              accountId: view.accountId,
              sourceLabel: view.sourceLabel,
              projectKey: view.issue.projectKey,
              projectName: view.issue.projectKey,
            })
          }
        />
      )}
    </div>
  );
};

export default TrackerPane;
