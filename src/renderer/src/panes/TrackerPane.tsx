import React, { useState, useEffect, useCallback } from 'react';
import type { TrackerAccount, TrackerProject, TrackerIssue, ProviderInfo } from '../types/tracker';
import { claudeColors as colors, ensureKeyframes } from '../components/claude-shared';

// ── Sub-views ──

type View =
  | { kind: 'accounts' }
  | { kind: 'add-account' }
  | { kind: 'projects'; accountId: string }
  | { kind: 'issues'; accountId: string; projectKey: string; projectName: string }
  | { kind: 'issue-detail'; accountId: string; issue: TrackerIssue };

// ── Status pill colors ──

function statusColor(cat: string): string {
  if (cat === 'done') return colors.success;
  if (cat === 'in_progress') return colors.accent;
  return colors.muted;
}

// ── Add Account Form ──

const AddAccountForm: React.FC<{
  providers: ProviderInfo[];
  onAdded: () => void;
  onCancel: () => void;
}> = ({ providers, onAdded, onCancel }) => {
  const [selectedProvider, setSelectedProvider] = useState(providers[0]?.id ?? '');
  const [label, setLabel] = useState('');
  const [config, setConfig] = useState<Record<string, string>>({});
  const [token, setToken] = useState('');
  const [error, setError] = useState('');
  const [loading, setLoading] = useState(false);

  const provider = providers.find(p => p.id === selectedProvider);

  const handleSubmit = async () => {
    if (!provider) return;
    setError('');
    setLoading(true);
    try {
      await window.electronAPI.trackerAddAccount(selectedProvider, label || provider.name, config, token);
      onAdded();
    } catch (err: any) {
      setError(err?.message ?? 'Failed to add account');
    } finally {
      setLoading(false);
    }
  };

  return (
    <div style={{ padding: '20px 24px', maxWidth: 480 }}>
      <div style={{ fontSize: '0.9rem', fontWeight: 700, color: colors.textBright, marginBottom: 16 }}>
        Add Account
      </div>

      {providers.length > 1 && (
        <div style={{ marginBottom: 12 }}>
          <FieldLabel>Provider</FieldLabel>
          <select
            value={selectedProvider}
            onChange={e => { setSelectedProvider(e.target.value); setConfig({}); }}
            style={inputStyle}
          >
            {providers.map(p => <option key={p.id} value={p.id}>{p.name}</option>)}
          </select>
        </div>
      )}

      <div style={{ marginBottom: 12 }}>
        <FieldLabel>Account Label</FieldLabel>
        <input
          value={label}
          onChange={e => setLabel(e.target.value)}
          placeholder={provider?.name ?? 'My Account'}
          style={inputStyle}
        />
      </div>

      {provider?.configFields.map(field => (
        <div key={field.key} style={{ marginBottom: 12 }}>
          <FieldLabel>{field.label}{field.required && ' *'}</FieldLabel>
          <input
            type={field.type}
            value={config[field.key] ?? ''}
            onChange={e => setConfig({ ...config, [field.key]: e.target.value })}
            placeholder={field.placeholder}
            style={inputStyle}
          />
        </div>
      ))}

      <div style={{ marginBottom: 12 }}>
        <FieldLabel>{provider?.tokenField.label ?? 'API Token'} *</FieldLabel>
        <input
          type="password"
          value={token}
          onChange={e => setToken(e.target.value)}
          placeholder={provider?.tokenField.placeholder ?? 'Paste token'}
          style={inputStyle}
        />
        {provider?.tokenField.helpText && (
          <div style={{ fontSize: '0.6rem', color: colors.muted, marginTop: 4 }}>
            {provider.tokenField.helpText}
          </div>
        )}
      </div>

      {error && (
        <div style={{ fontSize: '0.72rem', color: colors.error, marginBottom: 12 }}>
          {error}
        </div>
      )}

      <div style={{ display: 'flex', gap: 8 }}>
        <button onClick={handleSubmit} disabled={loading || !token} style={btnStyle(colors.accent, !!(token && !loading))}>
          {loading ? 'Connecting...' : 'Connect'}
        </button>
        <button onClick={onCancel} style={btnStyle(colors.muted, true)}>Cancel</button>
      </div>
    </div>
  );
};

// ── Account List ──

const AccountList: React.FC<{
  accounts: TrackerAccount[];
  onSelect: (accountId: string) => void;
  onAdd: () => void;
  onRemove: (accountId: string) => void;
}> = ({ accounts, onSelect, onAdd, onRemove }) => (
  <div style={{ padding: '20px 24px' }}>
    <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 16 }}>
      <div style={{ fontSize: '0.9rem', fontWeight: 700, color: colors.textBright }}>
        Issue Tracker
      </div>
      <button onClick={onAdd} style={btnStyle(colors.accent, true)}>+ Add Account</button>
    </div>

    {accounts.length === 0 && (
      <div style={{ textAlign: 'center', marginTop: 40, color: colors.muted }}>
        <div style={{ fontSize: '1.5rem', marginBottom: 8, opacity: 0.4 }}>{'\u{1F4CB}'}</div>
        <div style={{ fontSize: '0.8rem' }}>No accounts connected</div>
        <div style={{ fontSize: '0.7rem', marginTop: 4 }}>Add a Jira account to get started</div>
      </div>
    )}

    <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
      {accounts.map(account => (
        <div
          key={account.id}
          onClick={() => onSelect(account.id)}
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
            <div style={{ fontSize: '0.8rem', fontWeight: 600, color: colors.textBright }}>{account.label}</div>
            <div style={{ fontSize: '0.65rem', color: colors.muted, marginTop: 2 }}>
              {account.provider} {'\u00B7'} {account.config.url ?? account.config.email ?? ''}
            </div>
          </div>
          <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
            <span style={{ fontSize: '0.65rem', color: colors.accent }}>{'\u2192'}</span>
            <span
              onClick={(e) => { e.stopPropagation(); onRemove(account.id); }}
              style={{ fontSize: '0.7rem', color: colors.error, cursor: 'pointer', padding: '2px 4px' }}
              title="Remove account"
            >
              {'\u00D7'}
            </span>
          </div>
        </div>
      ))}
    </div>
  </div>
);

// ── Project List ──

const ProjectList: React.FC<{
  accountId: string;
  onSelect: (projectKey: string, projectName: string) => void;
  onBack: () => void;
}> = ({ accountId, onSelect, onBack }) => {
  const [projects, setProjects] = useState<TrackerProject[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');

  useEffect(() => {
    setLoading(true);
    window.electronAPI.trackerListProjects(accountId)
      .then(p => setProjects(p))
      .catch(e => setError(e?.message ?? 'Failed to load projects'))
      .finally(() => setLoading(false));
  }, [accountId]);

  return (
    <div style={{ padding: '20px 24px' }}>
      <div style={{ display: 'flex', alignItems: 'center', gap: 10, marginBottom: 16 }}>
        <button onClick={onBack} style={backBtnStyle}>{'\u2190'}</button>
        <div style={{ fontSize: '0.9rem', fontWeight: 700, color: colors.textBright }}>Projects</div>
      </div>

      {loading && <div style={{ color: colors.muted, fontSize: '0.75rem' }}>Loading projects...</div>}
      {error && <div style={{ color: colors.error, fontSize: '0.75rem' }}>{error}</div>}

      <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
        {projects.map(p => (
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
  const [search, setSearch] = useState('');

  const load = useCallback((query?: string) => {
    setLoading(true);
    const opts = query
      ? { projectKey, query, maxResults: 30 }
      : { projectKey, maxResults: 50 };
    window.electronAPI.trackerListIssues(accountId, opts)
      .then(i => setIssues(i))
      .finally(() => setLoading(false));
  }, [accountId, projectKey]);

  useEffect(() => { load(); }, [load]);

  const handleSearch = () => { if (search.trim()) load(search.trim()); else load(); };

  return (
    <div style={{ padding: '20px 24px' }}>
      <div style={{ display: 'flex', alignItems: 'center', gap: 10, marginBottom: 12 }}>
        <button onClick={onBack} style={backBtnStyle}>{'\u2190'}</button>
        <div>
          <div style={{ fontSize: '0.9rem', fontWeight: 700, color: colors.textBright }}>{projectName}</div>
          <div style={{ fontSize: '0.6rem', color: colors.muted, fontFamily: 'monospace' }}>{projectKey}</div>
        </div>
      </div>

      {/* Search */}
      <div style={{ display: 'flex', gap: 6, marginBottom: 14 }}>
        <input
          value={search}
          onChange={e => setSearch(e.target.value)}
          onKeyDown={e => { if (e.key === 'Enter') handleSearch(); }}
          placeholder="Search issues..."
          style={{ ...inputStyle, flex: 1 }}
        />
        <button onClick={handleSearch} style={btnStyle(colors.accent, true)}>Search</button>
      </div>

      {loading && <div style={{ color: colors.muted, fontSize: '0.75rem' }}>Loading...</div>}

      <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
        {issues.map(issue => (
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

const IssueDetail: React.FC<{
  issue: TrackerIssue;
  onBack: () => void;
}> = ({ issue, onBack }) => (
  <div style={{ padding: '20px 24px', maxWidth: 700 }}>
    <div style={{ display: 'flex', alignItems: 'center', gap: 10, marginBottom: 16 }}>
      <button onClick={onBack} style={backBtnStyle}>{'\u2190'}</button>
      <span style={{
        color: colors.accent,
        fontWeight: 700,
        fontFamily: 'monospace',
        fontSize: '0.8rem',
      }}>
        {issue.key}
      </span>
      <span style={{
        fontSize: '0.6rem',
        fontWeight: 600,
        padding: '2px 8px',
        borderRadius: 10,
        backgroundColor: 'rgba(255,255,255,0.05)',
        color: statusColor(issue.statusCategory),
      }}>
        {issue.status}
      </span>
    </div>

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
        {issue.labels.map(l => (
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

    <div style={{ marginTop: 12, fontSize: '0.62rem', color: colors.muted }}>
      Updated {new Date(issue.updated).toLocaleString()}
    </div>
  </div>
);

// ── Shared styles ──

const FieldLabel: React.FC<{ children: React.ReactNode }> = ({ children }) => (
  <div style={{ fontSize: '0.68rem', color: colors.muted, marginBottom: 4, fontWeight: 500 }}>
    {children}
  </div>
);

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

const TrackerPane: React.FC<TrackerPaneProps> = ({ paneId, title, isActive }) => {
  const [view, setView] = useState<View>({ kind: 'accounts' });
  const [accounts, setAccounts] = useState<TrackerAccount[]>([]);
  const [providers, setProviders] = useState<ProviderInfo[]>([]);

  useEffect(() => { ensureKeyframes(); }, []);

  const refreshAccounts = useCallback(() => {
    window.electronAPI.trackerGetAccounts().then(setAccounts);
  }, []);

  useEffect(() => {
    window.electronAPI.trackerGetProviders().then(setProviders);
    refreshAccounts();
  }, [refreshAccounts]);

  return (
    <div style={{
      width: '100%',
      height: '100%',
      overflow: 'auto',
      backgroundColor: colors.bg,
      color: colors.text,
      fontFamily: '-apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif',
    }}>
      {view.kind === 'accounts' && (
        <AccountList
          accounts={accounts}
          onSelect={(accountId) => setView({ kind: 'projects', accountId })}
          onAdd={() => setView({ kind: 'add-account' })}
          onRemove={(accountId) => {
            window.electronAPI.trackerRemoveAccount(accountId).then(refreshAccounts);
          }}
        />
      )}

      {view.kind === 'add-account' && (
        <AddAccountForm
          providers={providers}
          onAdded={() => { refreshAccounts(); setView({ kind: 'accounts' }); }}
          onCancel={() => setView({ kind: 'accounts' })}
        />
      )}

      {view.kind === 'projects' && (
        <ProjectList
          accountId={view.accountId}
          onSelect={(key, name) => setView({ kind: 'issues', accountId: view.accountId, projectKey: key, projectName: name })}
          onBack={() => setView({ kind: 'accounts' })}
        />
      )}

      {view.kind === 'issues' && (
        <IssueList
          accountId={view.accountId}
          projectKey={view.projectKey}
          projectName={view.projectName}
          onSelect={(issue) => setView({ kind: 'issue-detail', accountId: view.accountId, issue })}
          onBack={() => setView({ kind: 'projects', accountId: view.accountId })}
        />
      )}

      {view.kind === 'issue-detail' && (
        <IssueDetail
          issue={view.issue}
          onBack={() => setView({ kind: 'issues', accountId: view.accountId, projectKey: view.issue.projectKey, projectName: view.issue.projectKey })}
        />
      )}
    </div>
  );
};

export default TrackerPane;
