import React, { useCallback, useEffect, useRef, useState } from 'react';
import type { AnalyticsSummary, AnalyticsBucket, SessionHistoryRecord } from '../types/analytics';
import { usePageVisible } from '../hooks/usePageVisible';

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

/** Stat tile — mockup "Usage" card: uppercase mono label on top, large mono
 *  value, optional sub-line beneath. Flat panel surface, not glass. */
const Stat: React.FC<{ label: string; value: string; sub?: string; color?: string }> = ({ label, value, sub, color }) => (
  <div style={{
    flex: 1, minWidth: 120, padding: '15px 16px', borderRadius: 'var(--wks-radius-md, 13px)',
    background: 'var(--wks-bg-raised)',
    border: '1px solid var(--wks-border-subtle)',
  }}>
    <div style={{ fontSize: '0.6rem', color: 'var(--wks-text-faint)', textTransform: 'uppercase', letterSpacing: '0.08em' }}>{label}</div>
    <div style={{ fontSize: '1.5rem', fontWeight: 700, color: color || 'var(--wks-text-primary)', fontVariantNumeric: 'tabular-nums', marginTop: 8 }}>{value}</div>
    {sub && <div style={{ fontSize: '0.62rem', color: 'var(--wks-text-secondary)', marginTop: 3, fontVariantNumeric: 'tabular-nums' }}>{sub}</div>}
  </div>
);

const SectionTitle: React.FC<{ children: React.ReactNode }> = ({ children }) => (
  <div style={{ fontSize: '0.62rem', color: 'var(--wks-text-faint)', textTransform: 'uppercase', letterSpacing: '0.05em', fontWeight: 600, margin: '18px 0 8px' }}>{children}</div>
);

/** Panel for a card-style chart block (mockup "Daily spend" / "By model"). */
const ChartCard: React.FC<{ title: string; caption?: string; children: React.ReactNode; style?: React.CSSProperties }> = ({ title, caption, children, style }) => (
  <div style={{
    background: 'var(--wks-bg-raised)', border: '1px solid var(--wks-border-subtle)',
    borderRadius: 'var(--wks-radius-md, 14px)', padding: '17px 18px 15px', ...style,
  }}>
    <div style={{ display: 'flex', alignItems: 'baseline', justifyContent: 'space-between', marginBottom: 6, gap: 12 }}>
      <span style={{ fontSize: '0.8rem', fontWeight: 700, color: 'var(--wks-text-primary)' }}>{title}</span>
      {caption && <span style={{ fontSize: '0.62rem', color: 'var(--wks-text-faint)', fontVariantNumeric: 'tabular-nums' }}>{caption}</span>}
    </div>
    {children}
  </div>
);

/** Vertical gradient bars for cost across days (mockup "Daily spend").
 *  Day labels render sparsely so 30 buckets stay readable. */
const CostBars: React.FC<{ data: AnalyticsBucket[] }> = ({ data }) => {
  const max = Math.max(0.0001, ...data.map((d) => d.costUSD));
  if (data.length === 0) return <Empty />;
  // Label roughly six evenly-spaced days so a 30-bar series doesn't crowd.
  const step = Math.max(1, Math.ceil(data.length / 6));
  const dayLabel = (key: string): string => {
    const d = new Date(key);
    return Number.isNaN(d.getTime()) ? key.slice(5) : `${d.getMonth() + 1}/${d.getDate()}`;
  };
  return (
    <>
      <div style={{ display: 'flex', alignItems: 'flex-end', gap: 4, height: 150, marginTop: 12 }}>
        {data.map((d) => (
          <div key={d.key} title={`${d.key}: ${fmtUSD(d.costUSD)} · ${d.sessions} sessions`}
            style={{ flex: 1, minWidth: 3, display: 'flex', flexDirection: 'column', justifyContent: 'flex-end', height: '100%' }}>
            <div style={{
              height: `${Math.max(2, (d.costUSD / max) * 100)}%`,
              background: 'linear-gradient(180deg, var(--wks-accent), color-mix(in srgb, var(--wks-accent) 45%, var(--wks-bg-base)))',
              borderRadius: '5px 5px 0 0',
            }} />
          </div>
        ))}
      </div>
      <div style={{ display: 'flex', gap: 4, marginTop: 7 }}>
        {data.map((d, i) => (
          <span key={d.key} style={{ flex: 1, minWidth: 3, textAlign: 'center', fontSize: '0.58rem', color: 'var(--wks-text-faint)', fontVariantNumeric: 'tabular-nums', whiteSpace: 'nowrap' }}>
            {i % step === 0 ? dayLabel(d.key) : ''}
          </span>
        ))}
      </div>
    </>
  );
};

/** Stable per-model colour pool drawn from the active theme. */
const MODEL_COLORS = ['var(--wks-accent)', 'var(--wks-purple)', 'var(--wks-busy)', 'var(--wks-success)', 'var(--wks-warning)', 'var(--wks-error)'];

/** Mockup "By model": a stacked share bar over a legend of name / share% / cost. */
const ModelShare: React.FC<{ rows: AnalyticsBucket[] }> = ({ rows }) => {
  if (rows.length === 0) return <Empty />;
  const total = Math.max(0.0001, rows.reduce((sum, r) => sum + r.costUSD, 0));
  const ranked = [...rows].sort((a, b) => b.costUSD - a.costUSD);
  const withMeta = ranked.map((r, i) => ({
    ...r, color: MODEL_COLORS[i % MODEL_COLORS.length], share: Math.round((r.costUSD / total) * 100),
  }));
  return (
    <>
      <div style={{ display: 'flex', height: 12, borderRadius: 99, overflow: 'hidden', background: 'var(--wks-bg-base)', marginBottom: 4 }}>
        {withMeta.map((m) => (
          <span key={m.key} title={`${shortModel(m.key)} · ${m.share}%`} style={{ background: m.color, width: `${(m.costUSD / total) * 100}%` }} />
        ))}
      </div>
      {withMeta.map((m) => (
        <div key={m.key} style={{ display: 'flex', alignItems: 'center', gap: 10, padding: '9px 0', borderTop: '1px solid var(--wks-border-subtle)' }}>
          <span style={{ width: 9, height: 9, borderRadius: 3, flex: 'none', background: m.color }} />
          <span style={{ flex: 1, fontSize: '0.78rem', color: 'var(--wks-text-primary)', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }} title={m.key}>{shortModel(m.key)}</span>
          <span style={{ fontSize: '0.7rem', color: 'var(--wks-text-faint)', fontVariantNumeric: 'tabular-nums' }}>{m.share}%</span>
          <span style={{ fontSize: '0.7rem', color: 'var(--wks-accent)', fontWeight: 600, minWidth: 54, textAlign: 'right', fontVariantNumeric: 'tabular-nums' }}>{fmtUSD(m.costUSD)}</span>
        </div>
      ))}
    </>
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
  const pageVisible = usePageVisible();
  const wasHiddenRef = useRef(false);

  const refresh = useCallback(() => {
    window.electronAPI.analyticsSummary?.().then(setSummary).catch(() => {});
    window.electronAPI.analyticsRecent?.(100).then((r) => setRecent(Array.isArray(r) ? r : [])).catch(() => {});
  }, []);

  useEffect(() => {
    if (!pageVisible) {
      wasHiddenRef.current = true;
      return;
    }
    // Refresh immediately when becoming visible after being hidden.
    if (wasHiddenRef.current) {
      wasHiddenRef.current = false;
      refresh();
    }
    const t = setInterval(refresh, 15000);
    return () => clearInterval(t);
  }, [pageVisible, refresh]);

  // Initial load on mount.
  useEffect(() => {
    refresh();
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const t = summary?.totals;
  const byDay = summary?.byDay ?? [];
  const periodSpend = byDay.reduce((sum, d) => sum + d.costUSD, 0);
  const weekSpend = byDay.slice(-7).reduce((sum, d) => sum + d.costUSD, 0);

  return (
    <div style={{ height: '100%', overflow: 'auto', padding: '18px 20px', background: 'var(--wks-bg-base)' }}>
      <div style={{ display: 'flex', alignItems: 'baseline', gap: 11, marginBottom: 18 }}>
        <div style={{ fontSize: '1.05rem', fontWeight: 700, color: 'var(--wks-text-primary)' }}>Usage &amp; cost</div>
        <div style={{ fontSize: '0.72rem', color: 'var(--wks-text-secondary)', fontVariantNumeric: 'tabular-nums' }}>last 30 days · all agents</div>
        <button onClick={refresh} style={refreshBtn} title="Refresh">↻</button>
      </div>

      {/* Totals */}
      <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fit, minmax(150px, 1fr))', gap: 12, marginBottom: 14 }}>
        <Stat label="Sessions" value={String(t?.sessions ?? 0)} />
        <Stat label="Total cost" value={fmtUSD(t?.costUSD ?? 0)} color="var(--wks-accent)" sub={`${fmtUSD(weekSpend)} this week`} />
        <Stat label="Tokens" value={fmtTokens((t?.inputTokens ?? 0) + (t?.outputTokens ?? 0))} />
        <Stat label="Tool calls" value={fmtTokens(t?.toolCalls ?? 0)} />
        <Stat label="Workflow runs" value={String(t?.workflowRuns ?? 0)} />
        <Stat label="Active time" value={fmtDuration(t?.durationMs ?? 0)} />
      </div>

      {/* Daily spend + model share — mockup 1.7fr / 1fr split */}
      <div style={{ display: 'grid', gridTemplateColumns: 'minmax(280px, 1.7fr) minmax(220px, 1fr)', gap: 12, marginBottom: 14 }}>
        <ChartCard title="Daily spend" caption={`${fmtUSD(periodSpend)} this period`}>
          <CostBars data={summary?.byDay ?? []} />
        </ChartCard>
        <ChartCard title="By model">
          <ModelShare rows={summary?.byModel ?? []} />
        </ChartCard>
      </div>

      <SectionTitle>By project</SectionTitle>
      <BucketTable rows={summary?.byProject ?? []} labelOf={basename} header="Project" />

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
