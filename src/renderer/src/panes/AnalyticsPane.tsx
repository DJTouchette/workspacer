import React, { useCallback, useEffect, useState } from 'react';
import type { AnalyticsSummary, AnalyticsBucket, SessionHistoryRecord } from '../types/analytics';

function basename(p: string): string {
  return p ? (p.replace(/[/\\]+$/, '').split(/[/\\]/).pop() || p) : '(none)';
}
function fmtUSD(n: number): string {
  return n >= 0.01 ? `$${n.toFixed(2)}` : n > 0 ? '<$0.01' : '$0.00';
}
function fmtTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`;
  return String(n);
}
function fmtDuration(ms: number): string {
  const s = Math.round(ms / 1000);
  if (s < 60) return `${s}s`;
  const m = Math.round(s / 60);
  if (m < 60) return `${m}m`;
  const h = Math.floor(m / 60);
  return `${h}h ${m % 60}m`;
}
function shortModel(m: string | null): string {
  if (!m) return '(unknown)';
  return m.replace(/^claude-/, '').replace(/-\d{6,}$/, '');
}

const Stat: React.FC<{ label: string; value: string; color?: string }> = ({ label, value, color }) => (
  <div style={{
    flex: 1, minWidth: 120, padding: '12px 14px', borderRadius: 10,
    background: 'var(--wks-bg-surface)', border: '1px solid var(--wks-border-subtle)',
  }}>
    <div style={{ fontSize: '1.4rem', fontWeight: 700, color: color || 'var(--wks-text-primary)', fontVariantNumeric: 'tabular-nums' }}>{value}</div>
    <div style={{ fontSize: '0.6rem', color: 'var(--wks-text-faint)', textTransform: 'uppercase', letterSpacing: '0.05em', marginTop: 2 }}>{label}</div>
  </div>
);

const SectionTitle: React.FC<{ children: React.ReactNode }> = ({ children }) => (
  <div style={{ fontSize: '0.62rem', color: 'var(--wks-text-faint)', textTransform: 'uppercase', letterSpacing: '0.05em', fontWeight: 600, margin: '18px 0 8px' }}>{children}</div>
);

/** Horizontal bars for cost across days. */
const CostBars: React.FC<{ data: AnalyticsBucket[] }> = ({ data }) => {
  const max = Math.max(0.0001, ...data.map((d) => d.costUSD));
  if (data.length === 0) return <Empty />;
  return (
    <div style={{ display: 'flex', alignItems: 'flex-end', gap: 3, height: 90, padding: '4px 0' }}>
      {data.map((d) => (
        <div key={d.key} title={`${d.key}: ${fmtUSD(d.costUSD)} · ${d.sessions} sessions`}
          style={{ flex: 1, minWidth: 4, display: 'flex', flexDirection: 'column', justifyContent: 'flex-end', height: '100%' }}>
          <div style={{
            height: `${Math.max(2, (d.costUSD / max) * 100)}%`,
            background: 'var(--wks-accent)', borderRadius: '2px 2px 0 0', opacity: 0.85,
          }} />
        </div>
      ))}
    </div>
  );
};

const BucketTable: React.FC<{ rows: AnalyticsBucket[]; labelOf: (k: string) => string; header: string }> = ({ rows, labelOf, header }) => {
  if (rows.length === 0) return <Empty />;
  return (
    <table style={{ width: '100%', borderCollapse: 'collapse', fontSize: '0.72rem' }}>
      <thead>
        <tr style={{ color: 'var(--wks-text-faint)', textAlign: 'left' }}>
          <th style={th}>{header}</th>
          <th style={thNum}>Sessions</th>
          <th style={thNum}>Tokens</th>
          <th style={thNum}>Cost</th>
        </tr>
      </thead>
      <tbody>
        {rows.map((r) => (
          <tr key={r.key} style={{ borderTop: '1px solid var(--wks-border-subtle)' }}>
            <td style={{ ...td, color: 'var(--wks-text-primary)', fontWeight: 500 }} title={r.key}>{labelOf(r.key)}</td>
            <td style={tdNum}>{r.sessions}</td>
            <td style={tdNum}>{fmtTokens(r.tokens)}</td>
            <td style={{ ...tdNum, color: 'var(--wks-accent)' }}>{fmtUSD(r.costUSD)}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
};

const Empty: React.FC = () => (
  <div style={{ fontSize: '0.7rem', color: 'var(--wks-text-faint)', padding: '8px 0' }}>No data yet.</div>
);

const AnalyticsPane: React.FC<{ title?: string }> = () => {
  const [summary, setSummary] = useState<AnalyticsSummary | null>(null);
  const [recent, setRecent] = useState<SessionHistoryRecord[]>([]);

  const refresh = useCallback(() => {
    window.electronAPI.analyticsSummary?.().then(setSummary).catch(() => {});
    window.electronAPI.analyticsRecent?.(100).then((r) => setRecent(Array.isArray(r) ? r : [])).catch(() => {});
  }, []);

  useEffect(() => {
    refresh();
    const t = setInterval(refresh, 15000);
    return () => clearInterval(t);
  }, [refresh]);

  const t = summary?.totals;

  return (
    <div style={{ height: '100%', overflow: 'auto', padding: '18px 20px', background: 'var(--wks-bg-base)' }}>
      <div style={{ display: 'flex', alignItems: 'center', gap: 10, marginBottom: 14 }}>
        <div style={{ fontSize: '0.95rem', fontWeight: 700, color: 'var(--wks-text-primary)' }}>Analytics</div>
        <div style={{ fontSize: '0.62rem', color: 'var(--wks-text-faint)' }}>across all recorded sessions</div>
        <button onClick={refresh} style={refreshBtn} title="Refresh">↻</button>
      </div>

      {/* Totals */}
      <div style={{ display: 'flex', gap: 10, flexWrap: 'wrap' }}>
        <Stat label="Sessions" value={String(t?.sessions ?? 0)} />
        <Stat label="Total cost" value={fmtUSD(t?.costUSD ?? 0)} color="var(--wks-accent)" />
        <Stat label="Tokens" value={fmtTokens((t?.inputTokens ?? 0) + (t?.outputTokens ?? 0))} />
        <Stat label="Tool calls" value={fmtTokens(t?.toolCalls ?? 0)} />
        <Stat label="Workflow runs" value={String(t?.workflowRuns ?? 0)} />
        <Stat label="Active time" value={fmtDuration(t?.durationMs ?? 0)} />
      </div>

      <SectionTitle>Cost over time (last 30 days)</SectionTitle>
      <CostBars data={summary?.byDay ?? []} />

      <div style={{ display: 'flex', gap: 24, flexWrap: 'wrap', alignItems: 'flex-start' }}>
        <div style={{ flex: 1, minWidth: 280 }}>
          <SectionTitle>By project</SectionTitle>
          <BucketTable rows={summary?.byProject ?? []} labelOf={basename} header="Project" />
        </div>
        <div style={{ flex: 1, minWidth: 280 }}>
          <SectionTitle>By model</SectionTitle>
          <BucketTable rows={summary?.byModel ?? []} labelOf={shortModel} header="Model" />
        </div>
      </div>

      <SectionTitle>Recent sessions</SectionTitle>
      {recent.length === 0 ? <Empty /> : (
        <table style={{ width: '100%', borderCollapse: 'collapse', fontSize: '0.7rem' }}>
          <thead>
            <tr style={{ color: 'var(--wks-text-faint)', textAlign: 'left' }}>
              <th style={th}>Project</th>
              <th style={th}>Model</th>
              <th style={thNum}>Tokens</th>
              <th style={thNum}>Cost</th>
              <th style={thNum}>Tools</th>
              <th style={thNum}>Duration</th>
              <th style={th}>When</th>
            </tr>
          </thead>
          <tbody>
            {recent.map((r) => (
              <tr key={r.sessionId} style={{ borderTop: '1px solid var(--wks-border-subtle)' }}>
                <td style={{ ...td, color: 'var(--wks-text-primary)', fontWeight: 500 }} title={r.cwd}>
                  {r.agentName || basename(r.cwd)}
                  {r.status === 'active' && <span style={{ marginLeft: 6, fontSize: '0.55rem', color: 'var(--wks-success, #4ade80)' }}>● live</span>}
                </td>
                <td style={{ ...td, color: 'var(--wks-text-secondary)' }}>{shortModel(r.model)}</td>
                <td style={tdNum}>{fmtTokens(r.inputTokens + r.outputTokens)}</td>
                <td style={{ ...tdNum, color: 'var(--wks-accent)' }}>{fmtUSD(r.costUSD)}</td>
                <td style={tdNum}>{r.toolCalls}</td>
                <td style={tdNum}>{fmtDuration(r.durationMs)}</td>
                <td style={{ ...td, color: 'var(--wks-text-faint)' }}>{r.startedAt ? new Date(r.startedAt).toLocaleString() : ''}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>
  );
};

const th: React.CSSProperties = { padding: '4px 8px', fontWeight: 600, textTransform: 'uppercase', letterSpacing: '0.04em', fontSize: '0.55rem' };
const thNum: React.CSSProperties = { ...th, textAlign: 'right' };
const td: React.CSSProperties = { padding: '5px 8px', whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis', maxWidth: 220 };
const tdNum: React.CSSProperties = { ...td, textAlign: 'right', fontVariantNumeric: 'tabular-nums', color: 'var(--wks-text-secondary)' };
const refreshBtn: React.CSSProperties = {
  marginLeft: 'auto', width: 24, height: 24, borderRadius: 6, border: '1px solid var(--wks-border-input)',
  background: 'transparent', color: 'var(--wks-text-faint)', cursor: 'pointer', fontSize: '0.85rem',
};

export default AnalyticsPane;
