import React, { useState, useEffect } from 'react';
import { colors, formatTokens, formatCost } from '../utils';
import { api } from '../hooks/useApi';
import type { UsageSummary } from '../types';

const UsageDashboard: React.FC<{ onClose: () => void }> = ({ onClose }) => {
  const [data, setData] = useState<UsageSummary | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');

  useEffect(() => {
    api.getUsage()
      .then(d => { setData(d); setLoading(false); })
      .catch(e => { setError(e.message || 'Failed to load'); setLoading(false); });
  }, []);

  return (
    <div onClick={onClose} style={{
      position: 'fixed', inset: 0, zIndex: 200,
      backgroundColor: 'rgba(0,0,0,0.6)',
      display: 'flex', justifyContent: 'center', alignItems: 'stretch',
      padding: '2vh 4vw',
    }}>
      <div onClick={e => e.stopPropagation()} style={{
        flex: 1, maxWidth: 900,
        backgroundColor: colors.bgSurface,
        border: `1px solid ${colors.border}`,
        borderRadius: 12,
        display: 'flex', flexDirection: 'column', overflow: 'hidden',
      }}>
        {/* Header */}
        <div style={{
          padding: '14px 20px', borderBottom: `1px solid ${colors.borderSubtle}`,
          display: 'flex', alignItems: 'center', justifyContent: 'space-between',
        }}>
          <div>
            <div style={{ fontSize: '0.92rem', fontWeight: 700, color: colors.textBright }}>Usage Scanner</div>
            <div style={{ fontSize: '0.58rem', color: colors.textMuted, marginTop: 2 }}>Claude Code session history from ~/.claude/projects/</div>
          </div>
          <span onClick={onClose} style={{ cursor: 'pointer', color: colors.textMuted, fontSize: '1rem' }}>{'\u00D7'}</span>
        </div>

        {/* Body */}
        <div style={{ flex: 1, overflow: 'auto', padding: 20 }}>
          {loading && <div style={{ textAlign: 'center', color: colors.textMuted, padding: 40, fontSize: '0.72rem' }}>Scanning sessions...</div>}
          {error && <div style={{ textAlign: 'center', color: colors.error, padding: 40, fontSize: '0.72rem' }}>{error}</div>}
          {data && <UsageContent data={data} />}
        </div>
      </div>
    </div>
  );
};

const UsageContent: React.FC<{ data: UsageSummary }> = ({ data }) => {
  const [tab, setTab] = useState<'overview' | 'projects' | 'sessions'>('overview');

  return (
    <div>
      {/* Total cost banner */}
      <div style={{
        textAlign: 'center', marginBottom: 20, padding: '16px 0',
        borderRadius: 10, border: `1px solid ${colors.accent}30`,
        backgroundColor: `${colors.accent}08`,
      }}>
        <div style={{ fontSize: '1.8rem', fontWeight: 800, color: colors.accent, letterSpacing: '-0.03em' }}>
          {data.totalCost < 0.01 ? '<$0.01' : `$${data.totalCost.toFixed(2)}`}
        </div>
        <div style={{ fontSize: '0.62rem', color: colors.textMuted, marginTop: 4 }}>
          Total estimated cost across {data.totalSessions} session{data.totalSessions !== 1 ? 's' : ''}
        </div>
      </div>

      {/* Token breakdown */}
      <div style={{ display: 'grid', gridTemplateColumns: 'repeat(4, 1fr)', gap: 10, marginBottom: 20 }}>
        {[
          { label: 'Input', value: data.totalInputTokens, color: colors.accent },
          { label: 'Output', value: data.totalOutputTokens, color: colors.success },
          { label: 'Cache Read', value: data.totalCacheRead, color: colors.purple },
          { label: 'Cache Write', value: data.totalCacheWrite, color: colors.warning },
        ].map(item => (
          <div key={item.label} style={{
            padding: '10px 12px', borderRadius: 8,
            border: `1px solid ${colors.borderSubtle}`,
            backgroundColor: colors.bg,
          }}>
            <div style={{ fontSize: '0.55rem', color: colors.textMuted, marginBottom: 4, textTransform: 'uppercase', letterSpacing: '0.05em' }}>
              {item.label}
            </div>
            <div style={{ fontSize: '0.85rem', fontWeight: 700, color: item.color }}>
              {formatTokens(item.value)}
            </div>
          </div>
        ))}
      </div>

      {/* Tabs */}
      <div style={{ display: 'flex', gap: 4, marginBottom: 14 }}>
        {(['overview', 'projects', 'sessions'] as const).map(t => (
          <button key={t} onClick={() => setTab(t)} style={{
            padding: '4px 10px', borderRadius: 5, cursor: 'pointer',
            border: tab === t ? `1px solid ${colors.accent}` : `1px solid ${colors.borderSubtle}`,
            backgroundColor: tab === t ? `${colors.accent}15` : 'transparent',
            color: tab === t ? colors.accent : colors.textMuted,
            fontSize: '0.65rem', fontWeight: 600, textTransform: 'capitalize',
          }}>
            {t}
            {t === 'projects' && <span style={{ marginLeft: 4, fontSize: '0.55rem', opacity: 0.7 }}>{data.byProject?.length ?? 0}</span>}
            {t === 'sessions' && <span style={{ marginLeft: 4, fontSize: '0.55rem', opacity: 0.7 }}>{data.recentSessions?.length ?? 0}</span>}
          </button>
        ))}
      </div>

      {tab === 'overview' && <OverviewTab data={data} />}
      {tab === 'projects' && <ProjectsTab data={data} />}
      {tab === 'sessions' && <SessionsTab data={data} />}
    </div>
  );
};

const OverviewTab: React.FC<{ data: UsageSummary }> = ({ data }) => {
  // Show top 5 projects by cost
  const topProjects = (data.byProject ?? []).slice(0, 5);

  return (
    <div>
      <SectionTitle>Top Projects by Cost</SectionTitle>
      {topProjects.length === 0 ? (
        <div style={{ color: colors.textMuted, fontSize: '0.68rem', padding: '12px 0' }}>No projects found</div>
      ) : (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
          {topProjects.map(p => {
            const pct = data.totalCost > 0 ? (p.totalCost / data.totalCost) * 100 : 0;
            return (
              <div key={p.projectPath} style={{
                padding: '8px 12px', borderRadius: 6,
                border: `1px solid ${colors.borderSubtle}`,
                backgroundColor: colors.bg,
              }}>
                <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: 4 }}>
                  <span style={{ fontSize: '0.68rem', color: colors.textBright, fontWeight: 600 }}>{p.projectName}</span>
                  <span style={{ fontSize: '0.68rem', color: colors.accent, fontWeight: 700 }}>{formatCost(p.totalCost)}</span>
                </div>
                <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                  <div style={{ flex: 1, height: 4, borderRadius: 2, backgroundColor: `${colors.accent}20` }}>
                    <div style={{ width: `${pct}%`, height: '100%', borderRadius: 2, backgroundColor: colors.accent, transition: 'width 0.3s' }} />
                  </div>
                  <span style={{ fontSize: '0.55rem', color: colors.textMuted }}>{p.sessionCount} session{p.sessionCount !== 1 ? 's' : ''}</span>
                </div>
              </div>
            );
          })}
        </div>
      )}

      {/* Recent activity */}
      <SectionTitle style={{ marginTop: 16 }}>Recent Sessions</SectionTitle>
      {(data.recentSessions ?? []).slice(0, 5).map(s => (
        <SessionRow key={s.sessionId} session={s} />
      ))}
    </div>
  );
};

const ProjectsTab: React.FC<{ data: UsageSummary }> = ({ data }) => (
  <div>
    <table style={{ width: '100%', borderCollapse: 'collapse', fontSize: '0.65rem' }}>
      <thead>
        <tr style={{ borderBottom: `1px solid ${colors.borderSubtle}` }}>
          {['Project', 'Sessions', 'Tokens', 'Cost'].map(h => (
            <th key={h} style={{
              textAlign: h === 'Project' ? 'left' : 'right',
              padding: '6px 8px', color: colors.textMuted, fontWeight: 600,
              fontSize: '0.55rem', textTransform: 'uppercase', letterSpacing: '0.05em',
            }}>{h}</th>
          ))}
        </tr>
      </thead>
      <tbody>
        {(data.byProject ?? []).map(p => (
          <tr key={p.projectPath} style={{ borderBottom: `1px solid ${colors.borderSubtle}20` }}>
            <td style={{ padding: '8px', color: colors.textBright, fontWeight: 500 }}>
              <div>{p.projectName}</div>
              <div style={{ fontSize: '0.52rem', color: colors.textMuted, fontFamily: 'monospace' }}>{p.projectPath}</div>
            </td>
            <td style={{ padding: '8px', textAlign: 'right', color: colors.text }}>{p.sessionCount}</td>
            <td style={{ padding: '8px', textAlign: 'right', color: colors.text }}>{formatTokens(p.totalTokens)}</td>
            <td style={{ padding: '8px', textAlign: 'right', color: colors.accent, fontWeight: 700 }}>{formatCost(p.totalCost)}</td>
          </tr>
        ))}
      </tbody>
    </table>
    {(data.byProject ?? []).length === 0 && (
      <div style={{ color: colors.textMuted, fontSize: '0.68rem', padding: '20px 0', textAlign: 'center' }}>No projects found</div>
    )}
  </div>
);

const SessionsTab: React.FC<{ data: UsageSummary }> = ({ data }) => (
  <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
    {(data.recentSessions ?? []).map(s => (
      <SessionRow key={s.sessionId} session={s} />
    ))}
    {(data.recentSessions ?? []).length === 0 && (
      <div style={{ color: colors.textMuted, fontSize: '0.68rem', padding: '20px 0', textAlign: 'center' }}>No sessions found</div>
    )}
  </div>
);

const SessionRow: React.FC<{ session: UsageSummary['recentSessions'][0] }> = ({ session: s }) => {
  const totalTokens = s.inputTokens + s.outputTokens + s.cacheReadTokens + s.cacheWriteTokens;
  const modelShort = s.model ? s.model.split('-').slice(-2).join('-') : 'unknown';
  const ago = timeAgo(s.lastActivity);

  return (
    <div style={{
      padding: '8px 12px', borderRadius: 6,
      border: `1px solid ${colors.borderSubtle}`,
      backgroundColor: colors.bg, marginBottom: 2,
      display: 'flex', alignItems: 'center', gap: 10,
    }}>
      <div style={{ flex: 1, minWidth: 0 }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
          <span style={{ fontSize: '0.65rem', color: colors.textBright, fontWeight: 600 }}>{s.projectName}</span>
          <span style={{
            fontSize: '0.5rem', padding: '1px 5px', borderRadius: 4,
            backgroundColor: `${colors.purple}20`, color: colors.purple,
          }}>{modelShort}</span>
        </div>
        <div style={{ fontSize: '0.52rem', color: colors.textMuted, fontFamily: 'monospace', marginTop: 2 }}>
          {s.sessionId.slice(0, 8)}
        </div>
      </div>
      <div style={{ textAlign: 'right', fontSize: '0.58rem', color: colors.textMuted }}>
        <div>{s.turns} turn{s.turns !== 1 ? 's' : ''}</div>
        <div>{formatTokens(totalTokens)} tokens</div>
      </div>
      <div style={{ textAlign: 'right', minWidth: 50 }}>
        <div style={{ fontSize: '0.68rem', fontWeight: 700, color: colors.accent }}>{formatCost(s.cost)}</div>
        <div style={{ fontSize: '0.5rem', color: colors.textMuted }}>{ago}</div>
      </div>
    </div>
  );
};

const SectionTitle: React.FC<{ children: React.ReactNode; style?: React.CSSProperties }> = ({ children, style }) => (
  <div style={{
    fontSize: '0.62rem', fontWeight: 700, color: colors.textMuted,
    textTransform: 'uppercase', letterSpacing: '0.06em',
    marginBottom: 8, ...style,
  }}>{children}</div>
);

function timeAgo(ts: string): string {
  const ms = Date.now() - new Date(ts).getTime();
  if (ms < 0) return 'just now';
  const s = Math.floor(ms / 1000);
  if (s < 60) return `${s}s ago`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m ago`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h}h ago`;
  const d = Math.floor(h / 24);
  return `${d}d ago`;
}

export default UsageDashboard;
