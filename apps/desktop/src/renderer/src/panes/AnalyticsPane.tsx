import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { AnalyticsSummary, AnalyticsBucket, SessionHistoryRecord } from '../types/analytics';
import { usePageVisible } from '../hooks/usePageVisible';
import { shortModelLabel } from '../lib/modelLabel';
import { AgentLogo } from '../components/agentLogos';
import type { AgentProvider } from '../types/pane';

/** Provider filter selection. 'all' folds every backend together. */
type ProviderFilter = 'all' | AgentProvider;
const PROVIDER_FILTERS: { value: ProviderFilter; label: string }[] = [
  { value: 'all', label: 'All' },
  { value: 'claude', label: 'Claude' },
  { value: 'codex', label: 'Codex' },
  { value: 'opencode', label: 'OpenCode' },
];
function providerLabel(key: string): string {
  switch (key) {
    case 'claude':
      return 'Claude';
    case 'codex':
      return 'Codex';
    case 'opencode':
      return 'OpenCode';
    default:
      return key || '(unknown)';
  }
}

function basename(p: string): string {
  return p
    ? p
        .replace(/[/\\]+$/, '')
        .split(/[/\\]/)
        .pop() || p
    : '(none)';
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
  return shortModelLabel(m) || '(unknown)';
}

/** Stat tile — mockup "Usage" card: uppercase mono label on top, large mono
 *  value, optional sub-line beneath. Flat panel surface, not glass. */
const Stat: React.FC<{ label: string; value: string; sub?: string; color?: string }> = ({
  label,
  value,
  sub,
  color,
}) => (
  <div
    style={{
      flex: 1,
      minWidth: 120,
      padding: '15px 16px',
      borderRadius: 'var(--wks-radius-md, 13px)',
      background: 'var(--wks-bg-raised)',
      border: '1px solid var(--wks-border-subtle)',
    }}
  >
    <div
      style={{
        fontSize: '0.6rem',
        color: 'var(--wks-text-faint)',
        textTransform: 'uppercase',
        letterSpacing: '0.08em',
      }}
    >
      {label}
    </div>
    <div
      style={{
        fontSize: '1.5rem',
        fontWeight: 700,
        color: color || 'var(--wks-text-primary)',
        fontVariantNumeric: 'tabular-nums',
        marginTop: 8,
      }}
    >
      {value}
    </div>
    {sub && (
      <div
        style={{
          fontSize: '0.62rem',
          color: 'var(--wks-text-secondary)',
          marginTop: 3,
          fontVariantNumeric: 'tabular-nums',
        }}
      >
        {sub}
      </div>
    )}
  </div>
);

const SectionTitle: React.FC<{ children: React.ReactNode }> = ({ children }) => (
  <div
    style={{
      fontSize: '0.62rem',
      color: 'var(--wks-text-faint)',
      textTransform: 'uppercase',
      letterSpacing: '0.05em',
      fontWeight: 600,
      margin: '18px 0 8px',
    }}
  >
    {children}
  </div>
);

/** Panel for a card-style chart block (mockup "Daily spend" / "By model"). */
const ChartCard: React.FC<{
  title: string;
  caption?: string;
  children: React.ReactNode;
  style?: React.CSSProperties;
}> = ({ title, caption, children, style }) => (
  <div
    style={{
      background: 'var(--wks-bg-raised)',
      border: '1px solid var(--wks-border-subtle)',
      borderRadius: 'var(--wks-radius-md, 14px)',
      padding: '17px 18px 15px',
      ...style,
    }}
  >
    <div
      style={{
        display: 'flex',
        alignItems: 'baseline',
        justifyContent: 'space-between',
        marginBottom: 6,
        gap: 12,
      }}
    >
      <span style={{ fontSize: '0.8rem', fontWeight: 700, color: 'var(--wks-text-primary)' }}>
        {title}
      </span>
      {caption && (
        <span
          style={{
            fontSize: '0.62rem',
            color: 'var(--wks-text-faint)',
            fontVariantNumeric: 'tabular-nums',
          }}
        >
          {caption}
        </span>
      )}
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
          <div
            key={d.key}
            title={`${d.key}: ${fmtUSD(d.costUSD)} · ${d.sessions} sessions`}
            style={{
              flex: 1,
              minWidth: 3,
              display: 'flex',
              flexDirection: 'column',
              justifyContent: 'flex-end',
              height: '100%',
            }}
          >
            <div
              style={{
                height: `${Math.max(2, (d.costUSD / max) * 100)}%`,
                background:
                  'linear-gradient(180deg, var(--wks-accent), color-mix(in srgb, var(--wks-accent) 45%, var(--wks-bg-base)))',
                borderRadius: '5px 5px 0 0',
              }}
            />
          </div>
        ))}
      </div>
      <div style={{ display: 'flex', gap: 4, marginTop: 7 }}>
        {data.map((d, i) => (
          <span
            key={d.key}
            style={{
              flex: 1,
              minWidth: 3,
              textAlign: 'center',
              fontSize: '0.58rem',
              color: 'var(--wks-text-faint)',
              fontVariantNumeric: 'tabular-nums',
              whiteSpace: 'nowrap',
            }}
          >
            {i % step === 0 ? dayLabel(d.key) : ''}
          </span>
        ))}
      </div>
    </>
  );
};

/** Stable per-model colour pool drawn from the active theme. */
const MODEL_COLORS = [
  'var(--wks-accent)',
  'var(--wks-purple)',
  'var(--wks-busy)',
  'var(--wks-success)',
  'var(--wks-warning)',
  'var(--wks-error)',
];

/** Mockup "By model": a stacked share bar over a legend of name / share% / cost. */
const ModelShare: React.FC<{ rows: AnalyticsBucket[] }> = ({ rows }) => {
  if (rows.length === 0) return <Empty />;
  const total = Math.max(
    0.0001,
    rows.reduce((sum, r) => sum + r.costUSD, 0),
  );
  const ranked = [...rows].sort((a, b) => b.costUSD - a.costUSD);
  const withMeta = ranked.map((r, i) => ({
    ...r,
    color: MODEL_COLORS[i % MODEL_COLORS.length],
    share: Math.round((r.costUSD / total) * 100),
  }));
  return (
    <>
      <div
        style={{
          display: 'flex',
          height: 12,
          borderRadius: 'var(--wks-radius-pill)',
          overflow: 'hidden',
          background: 'var(--wks-bg-base)',
          marginBottom: 4,
        }}
      >
        {withMeta.map((m) => (
          <span
            key={m.key}
            title={`${shortModel(m.key)} · ${m.share}%`}
            style={{ background: m.color, width: `${(m.costUSD / total) * 100}%` }}
          />
        ))}
      </div>
      {withMeta.map((m) => (
        <div
          key={m.key}
          style={{
            display: 'flex',
            alignItems: 'center',
            gap: 10,
            padding: '9px 0',
            borderTop: '1px solid var(--wks-border-subtle)',
          }}
        >
          <span
            style={{ width: 9, height: 9, borderRadius: 3, flex: 'none', background: m.color }}
          />
          <span
            style={{
              flex: 1,
              fontSize: '0.78rem',
              color: 'var(--wks-text-primary)',
              overflow: 'hidden',
              textOverflow: 'ellipsis',
              whiteSpace: 'nowrap',
            }}
            title={m.key}
          >
            {shortModel(m.key)}
          </span>
          <span
            style={{
              fontSize: '0.7rem',
              color: 'var(--wks-text-faint)',
              fontVariantNumeric: 'tabular-nums',
            }}
          >
            {m.share}%
          </span>
          <span
            style={{
              fontSize: '0.7rem',
              color: 'var(--wks-accent)',
              fontWeight: 600,
              minWidth: 54,
              textAlign: 'right',
              fontVariantNumeric: 'tabular-nums',
            }}
          >
            {fmtUSD(m.costUSD)}
          </span>
        </div>
      ))}
    </>
  );
};

/** Stable colour per provider so the split reads consistently across renders. */
const PROVIDER_COLORS: Record<string, string> = {
  claude: 'var(--wks-accent)',
  codex: 'var(--wks-success)',
  opencode: 'var(--wks-purple)',
};

/** "By provider": the combined split across coding-agent backends. Always shows
 *  every provider (independent of the active filter) so you can see the whole
 *  picture while drilled into one. Each row carries its brand logo. */
const ProviderShare: React.FC<{
  rows: AnalyticsBucket[];
  active: ProviderFilter;
  onPick: (p: ProviderFilter) => void;
}> = ({ rows, active, onPick }) => {
  if (rows.length === 0) return <Empty />;
  const total = Math.max(
    0.0001,
    rows.reduce((sum, r) => sum + r.costUSD, 0),
  );
  const ranked = [...rows].sort((a, b) => b.costUSD - a.costUSD);
  const colorOf = (k: string) => PROVIDER_COLORS[k] ?? 'var(--wks-busy)';
  return (
    <>
      <div
        style={{
          display: 'flex',
          height: 12,
          borderRadius: 'var(--wks-radius-pill)',
          overflow: 'hidden',
          background: 'var(--wks-bg-base)',
          marginBottom: 4,
        }}
      >
        {ranked.map((m) => (
          <span
            key={m.key}
            title={`${providerLabel(m.key)} · ${Math.round((m.costUSD / total) * 100)}%`}
            style={{ background: colorOf(m.key), width: `${(m.costUSD / total) * 100}%` }}
          />
        ))}
      </div>
      {ranked.map((m) => {
        const share = Math.round((m.costUSD / total) * 100);
        const isActive = active === m.key;
        const provider = (
          ['claude', 'codex', 'opencode'].includes(m.key) ? m.key : 'claude'
        ) as AgentProvider;
        return (
          <div
            key={m.key}
            onClick={() => onPick(isActive ? 'all' : (m.key as ProviderFilter))}
            title={
              isActive
                ? 'Showing this backend — click to clear'
                : `Filter to ${providerLabel(m.key)}`
            }
            style={{
              display: 'flex',
              alignItems: 'center',
              gap: 10,
              padding: '9px 8px',
              margin: '0 -8px',
              borderTop: '1px solid var(--wks-border-subtle)',
              cursor: 'pointer',
              borderRadius: 7,
              background: isActive ? 'var(--wks-bg-selected)' : 'transparent',
            }}
          >
            <span
              style={{
                width: 9,
                height: 9,
                borderRadius: 3,
                flex: 'none',
                background: colorOf(m.key),
              }}
            />
            <AgentLogo
              provider={provider}
              size={14}
              style={{ flex: 'none', color: 'var(--wks-text-secondary)' }}
            />
            <span
              style={{
                flex: 1,
                fontSize: '0.78rem',
                color: 'var(--wks-text-primary)',
                fontWeight: isActive ? 600 : 400,
                overflow: 'hidden',
                textOverflow: 'ellipsis',
                whiteSpace: 'nowrap',
              }}
            >
              {providerLabel(m.key)}
            </span>
            <span
              style={{
                fontSize: '0.66rem',
                color: 'var(--wks-text-faint)',
                fontVariantNumeric: 'tabular-nums',
              }}
            >
              {m.sessions} ses
            </span>
            <span
              style={{
                fontSize: '0.7rem',
                color: 'var(--wks-text-faint)',
                fontVariantNumeric: 'tabular-nums',
              }}
            >
              {share}%
            </span>
            <span
              style={{
                fontSize: '0.7rem',
                color: 'var(--wks-accent)',
                fontWeight: 600,
                minWidth: 54,
                textAlign: 'right',
                fontVariantNumeric: 'tabular-nums',
              }}
            >
              {fmtUSD(m.costUSD)}
            </span>
          </div>
        );
      })}
    </>
  );
};

const BucketTable: React.FC<{
  rows: AnalyticsBucket[];
  labelOf: (k: string) => string;
  header: string;
}> = ({ rows, labelOf, header }) => {
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
            <td style={{ ...td, color: 'var(--wks-text-primary)', fontWeight: 500 }} title={r.key}>
              {labelOf(r.key)}
            </td>
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
  <div style={{ fontSize: '0.7rem', color: 'var(--wks-text-faint)', padding: '8px 0' }}>
    No data yet.
  </div>
);

/** Segmented Overview/Breakdown toggle (mockup style). */
const SegBtn: React.FC<{ active: boolean; onClick: () => void; children: React.ReactNode }> = ({
  active,
  onClick,
  children,
}) => (
  <button
    onClick={onClick}
    style={{
      border: 'none',
      borderRadius: 6,
      padding: '5px 13px',
      cursor: 'pointer',
      fontFamily: 'inherit',
      fontSize: '0.72rem',
      fontWeight: 600,
      background: active ? 'var(--wks-bg-selected)' : 'transparent',
      color: active ? 'var(--wks-text-primary)' : 'var(--wks-text-faint)',
    }}
  >
    {children}
  </button>
);

interface AgentAgg {
  name: string;
  model: string | null;
  tokens: number;
  tools: number;
  costUSD: number;
}

type BreakdownMetric = 'cost' | 'tokens';

const BD_COLS = '1.4fr 1fr 0.8fr 0.6fr 0.8fr 1.3fr 0.8fr';

/** Cost per million tokens — the efficiency readout. */
function fmtPerM(costUSD: number, tokens: number): string {
  if (tokens <= 0) return '—';
  return fmtUSD(costUSD / (tokens / 1_000_000));
}

/** Mockup "Breakdown": per-agent grid (model · tokens · tools · $/M · share bar ·
 *  cost) capped with a Total row. The share bar + sort follow the chosen metric
 *  (cost or tokens), aggregated from recent session records. */
const BreakdownTable: React.FC<{ rows: AgentAgg[]; metric: BreakdownMetric }> = ({
  rows,
  metric,
}) => {
  if (rows.length === 0) return <Empty />;
  const valueOf = (r: AgentAgg) => (metric === 'tokens' ? r.tokens : r.costUSD);
  const totalCost = rows.reduce((s, r) => s + r.costUSD, 0);
  const totalTokens = rows.reduce((s, r) => s + r.tokens, 0);
  const totalTools = rows.reduce((s, r) => s + r.tools, 0);
  const totalMetric = Math.max(0.0001, metric === 'tokens' ? totalTokens : totalCost);
  const ranked = [...rows].sort((a, b) => valueOf(b) - valueOf(a));
  const cell: React.CSSProperties = {
    fontSize: '0.72rem',
    color: 'var(--wks-text-secondary)',
    fontVariantNumeric: 'tabular-nums',
    overflow: 'hidden',
    textOverflow: 'ellipsis',
    whiteSpace: 'nowrap',
  };
  const numCell: React.CSSProperties = { ...cell, textAlign: 'right' };
  // Faintly emphasise whichever raw-value column the share is keyed to.
  const hot = (col: BreakdownMetric): React.CSSProperties =>
    col === metric ? { color: 'var(--wks-text-primary)', fontWeight: 600 } : {};
  return (
    <div
      style={{
        background: 'var(--wks-bg-raised)',
        border: '1px solid var(--wks-border-subtle)',
        borderRadius: 'var(--wks-radius-md, 14px)',
        overflow: 'hidden',
      }}
    >
      <div
        style={{
          display: 'grid',
          gridTemplateColumns: BD_COLS,
          gap: 14,
          padding: '11px 16px',
          fontSize: '0.55rem',
          textTransform: 'uppercase',
          letterSpacing: '0.08em',
          color: 'var(--wks-text-faint)',
        }}
      >
        <span>Agent</span>
        <span>Model</span>
        <span style={{ textAlign: 'right' }}>Tokens</span>
        <span style={{ textAlign: 'right' }}>Tools</span>
        <span style={{ textAlign: 'right' }}>$/M</span>
        <span>Share</span>
        <span style={{ textAlign: 'right' }}>Cost</span>
      </div>
      {ranked.map((r, i) => {
        const share = Math.round((valueOf(r) / totalMetric) * 100);
        const color = MODEL_COLORS[i % MODEL_COLORS.length];
        return (
          <div
            key={r.name}
            style={{
              display: 'grid',
              gridTemplateColumns: BD_COLS,
              gap: 14,
              alignItems: 'center',
              padding: '12px 16px',
              borderTop: '1px solid var(--wks-border-subtle)',
            }}
          >
            <span
              style={{ ...cell, color: 'var(--wks-text-primary)', fontWeight: 600 }}
              title={r.name}
            >
              {r.name}
            </span>
            <span style={cell}>{shortModel(r.model)}</span>
            <span style={{ ...numCell, ...hot('tokens') }}>{fmtTokens(r.tokens)}</span>
            <span style={numCell}>{r.tools}</span>
            <span style={numCell}>{fmtPerM(r.costUSD, r.tokens)}</span>
            <span style={{ display: 'flex', alignItems: 'center', gap: 9 }}>
              <span
                style={{
                  flex: 1,
                  height: 6,
                  borderRadius: 'var(--wks-radius-pill)',
                  background: 'var(--wks-bg-base)',
                  overflow: 'hidden',
                }}
              >
                <span
                  style={{
                    display: 'block',
                    height: '100%',
                    borderRadius: 'var(--wks-radius-pill)',
                    background: color,
                    width: `${Math.max(2, share)}%`,
                  }}
                />
              </span>
              <span
                style={{
                  fontSize: '0.64rem',
                  color: 'var(--wks-text-faint)',
                  minWidth: 30,
                  fontVariantNumeric: 'tabular-nums',
                }}
              >
                {share}%
              </span>
            </span>
            <span style={{ ...numCell, color: 'var(--wks-accent)', ...hot('cost') }}>
              {fmtUSD(r.costUSD)}
            </span>
          </div>
        );
      })}
      <div
        style={{
          display: 'grid',
          gridTemplateColumns: BD_COLS,
          gap: 14,
          alignItems: 'center',
          padding: '12px 16px',
          borderTop: '1px solid var(--wks-border)',
          background: 'var(--wks-bg-base)',
        }}
      >
        <span style={{ fontSize: '0.74rem', fontWeight: 700, color: 'var(--wks-text-primary)' }}>
          Total
        </span>
        <span />
        <span style={numCell}>{fmtTokens(totalTokens)}</span>
        <span style={numCell}>{totalTools}</span>
        <span style={numCell}>{fmtPerM(totalCost, totalTokens)}</span>
        <span />
        <span
          style={{
            textAlign: 'right',
            fontSize: '0.78rem',
            fontWeight: 700,
            color: 'var(--wks-accent)',
            fontVariantNumeric: 'tabular-nums',
          }}
        >
          {fmtUSD(totalCost)}
        </span>
      </div>
    </div>
  );
};

const AnalyticsPane: React.FC<{ title?: string }> = () => {
  const [summary, setSummary] = useState<AnalyticsSummary | null>(null);
  const [recent, setRecent] = useState<SessionHistoryRecord[]>([]);
  const pageVisible = usePageVisible();
  const wasHiddenRef = useRef(false);

  const [provider, setProvider] = useState<ProviderFilter>(() => {
    try {
      const v = localStorage.getItem('wks-usage-provider');
      return PROVIDER_FILTERS.some((p) => p.value === v) ? (v as ProviderFilter) : 'all';
    } catch {
      return 'all';
    }
  });
  const pickProvider = (v: ProviderFilter) => {
    setProvider(v);
    try {
      localStorage.setItem('wks-usage-provider', v);
    } catch {
      /* private mode */
    }
  };

  const refresh = useCallback(() => {
    // 'all' ⇒ no scope; otherwise scope totals/breakdowns to one backend. The
    // byProvider split inside the summary is always all so the combined picture
    // stays visible while filtered.
    const scope = provider === 'all' ? undefined : provider;
    window.electronAPI
      .analyticsSummary?.(scope)
      .then(setSummary)
      .catch(() => {});
    window.electronAPI
      .analyticsRecent?.(100, scope)
      .then((r) => setRecent(Array.isArray(r) ? r : []))
      .catch(() => {});
  }, [provider]);

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

  // Initial load on mount, and whenever the provider filter changes.
  useEffect(() => {
    refresh();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [provider]);

  const [usageView, setUsageView] = useState<'overview' | 'breakdown'>(() => {
    try {
      return localStorage.getItem('wks-usage-view') === 'breakdown' ? 'breakdown' : 'overview';
    } catch {
      return 'overview';
    }
  });
  const pickUsageView = (v: 'overview' | 'breakdown') => {
    setUsageView(v);
    try {
      localStorage.setItem('wks-usage-view', v);
    } catch {
      /* private mode */
    }
  };
  const [bdMetric, setBdMetric] = useState<BreakdownMetric>(() => {
    try {
      return localStorage.getItem('wks-breakdown-metric') === 'tokens' ? 'tokens' : 'cost';
    } catch {
      return 'cost';
    }
  });
  const pickBdMetric = (v: BreakdownMetric) => {
    setBdMetric(v);
    try {
      localStorage.setItem('wks-breakdown-metric', v);
    } catch {
      /* private mode */
    }
  };

  const t = summary?.totals;
  const byDay = summary?.byDay ?? [];
  const periodSpend = byDay.reduce((sum, d) => sum + d.costUSD, 0);
  const weekSpend = byDay.slice(-7).reduce((sum, d) => sum + d.costUSD, 0);

  // Per-agent breakdown, rolled up from recent session records (which carry the
  // model / tool / cost detail the day/project buckets don't).
  const agentRows = useMemo<AgentAgg[]>(() => {
    const m = new Map<string, AgentAgg>();
    for (const r of recent) {
      const name = r.agentName || basename(r.cwd);
      const e = m.get(name) ?? { name, model: r.model, tokens: 0, tools: 0, costUSD: 0 };
      e.tokens += r.inputTokens + r.outputTokens;
      e.tools += r.toolCalls;
      e.costUSD += r.costUSD;
      if (!e.model && r.model) e.model = r.model;
      m.set(name, e);
    }
    return [...m.values()].sort((a, b) => b.costUSD - a.costUSD);
  }, [recent]);

  return (
    <div
      style={{
        height: '100%',
        overflow: 'auto',
        padding: '18px 20px',
        background: 'var(--wks-bg-base)',
      }}
    >
      <div style={{ display: 'flex', alignItems: 'baseline', gap: 11, marginBottom: 12 }}>
        <div style={{ fontSize: '1.05rem', fontWeight: 700, color: 'var(--wks-text-primary)' }}>
          Usage &amp; cost
        </div>
        <div
          style={{
            fontSize: '0.72rem',
            color: 'var(--wks-text-secondary)',
            fontVariantNumeric: 'tabular-nums',
          }}
        >
          last 30 days · {provider === 'all' ? 'all agents' : providerLabel(provider)}
        </div>
        <div style={{ flex: 1 }} />
        <div
          style={{
            display: 'flex',
            alignSelf: 'center',
            background: 'var(--wks-bg-surface)',
            border: '1px solid var(--wks-border-subtle)',
            borderRadius: 'var(--wks-radius-md)',
            padding: 3,
          }}
        >
          <SegBtn active={usageView === 'overview'} onClick={() => pickUsageView('overview')}>
            Overview
          </SegBtn>
          <SegBtn active={usageView === 'breakdown'} onClick={() => pickUsageView('breakdown')}>
            Breakdown
          </SegBtn>
        </div>
        <button
          onClick={refresh}
          style={{ ...refreshBtn, alignSelf: 'center', marginLeft: 0 }}
          title="Refresh"
        >
          ↻
        </button>
      </div>

      {/* Provider filter — scope every figure to one backend, or fold them all
          together. The "By provider" card below always shows the full split. */}
      <div style={{ display: 'flex', gap: 6, marginBottom: 18, flexWrap: 'wrap' }}>
        {PROVIDER_FILTERS.map((p) => {
          const active = provider === p.value;
          return (
            <button
              key={p.value}
              onClick={() => pickProvider(p.value)}
              style={{
                display: 'flex',
                alignItems: 'center',
                gap: 6,
                padding: '5px 11px',
                borderRadius: 'var(--wks-radius-pill)',
                cursor: 'pointer',
                fontFamily: 'inherit',
                fontSize: '0.7rem',
                fontWeight: 600,
                border: `1px solid ${active ? 'var(--wks-accent)' : 'var(--wks-border-subtle)'}`,
                background: active
                  ? 'color-mix(in srgb, var(--wks-accent) 16%, transparent)'
                  : 'var(--wks-bg-surface)',
                color: active ? 'var(--wks-text-primary)' : 'var(--wks-text-secondary)',
              }}
            >
              {p.value !== 'all' && (
                <AgentLogo provider={p.value} size={13} style={{ flex: 'none' }} />
              )}
              {p.label}
            </button>
          );
        })}
      </div>

      {usageView === 'overview' ? (
        <>
          {/* Totals */}
          <div
            style={{
              display: 'grid',
              gridTemplateColumns: 'repeat(auto-fit, minmax(150px, 1fr))',
              gap: 12,
              marginBottom: 14,
            }}
          >
            <Stat label="Sessions" value={String(t?.sessions ?? 0)} />
            <Stat
              label="Total cost"
              value={fmtUSD(t?.costUSD ?? 0)}
              color="var(--wks-accent)"
              sub={`${fmtUSD(weekSpend)} this week`}
            />
            <Stat
              label="Tokens"
              value={fmtTokens((t?.inputTokens ?? 0) + (t?.outputTokens ?? 0))}
            />
            <Stat label="Tool calls" value={fmtTokens(t?.toolCalls ?? 0)} />
            <Stat label="Workflow runs" value={String(t?.workflowRuns ?? 0)} />
            <Stat label="Active time" value={fmtDuration(t?.durationMs ?? 0)} />
          </div>

          {/* Daily spend + model share — mockup 1.7fr / 1fr split */}
          <div
            style={{
              display: 'grid',
              gridTemplateColumns: 'minmax(280px, 1.7fr) minmax(220px, 1fr)',
              gap: 12,
              marginBottom: 14,
            }}
          >
            <ChartCard title="Daily spend" caption={`${fmtUSD(periodSpend)} this period`}>
              <CostBars data={summary?.byDay ?? []} />
            </ChartCard>
            <ChartCard title="By provider" caption="all backends">
              <ProviderShare
                rows={summary?.byProvider ?? []}
                active={provider}
                onPick={pickProvider}
              />
            </ChartCard>
          </div>

          <ChartCard title="By model" style={{ marginBottom: 14 }}>
            <ModelShare rows={summary?.byModel ?? []} />
          </ChartCard>
        </>
      ) : (
        <>
          <div
            style={{
              display: 'flex',
              alignItems: 'center',
              justifyContent: 'space-between',
              gap: 12,
              margin: '18px 0 8px',
            }}
          >
            <span
              style={{
                fontSize: '0.62rem',
                color: 'var(--wks-text-faint)',
                textTransform: 'uppercase',
                letterSpacing: '0.05em',
                fontWeight: 600,
              }}
            >
              By agent
            </span>
            <div
              style={{
                display: 'flex',
                background: 'var(--wks-bg-surface)',
                border: '1px solid var(--wks-border-subtle)',
                borderRadius: 'var(--wks-radius-md)',
                padding: 3,
              }}
            >
              <SegBtn active={bdMetric === 'cost'} onClick={() => pickBdMetric('cost')}>
                Cost
              </SegBtn>
              <SegBtn active={bdMetric === 'tokens'} onClick={() => pickBdMetric('tokens')}>
                Tokens
              </SegBtn>
            </div>
          </div>
          <BreakdownTable rows={agentRows} metric={bdMetric} />

          <SectionTitle>By project</SectionTitle>
          <BucketTable rows={summary?.byProject ?? []} labelOf={basename} header="Project" />

          <SectionTitle>Recent sessions</SectionTitle>
          {recent.length === 0 ? (
            <Empty />
          ) : (
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
                    <td
                      style={{ ...td, color: 'var(--wks-text-primary)', fontWeight: 500 }}
                      title={r.cwd}
                    >
                      <span
                        style={{
                          display: 'inline-flex',
                          alignItems: 'center',
                          gap: 6,
                          verticalAlign: 'middle',
                        }}
                      >
                        <AgentLogo
                          provider={(r.provider || 'claude') as AgentProvider}
                          size={12}
                          style={{ flex: 'none', color: 'var(--wks-text-faint)' }}
                        />
                        {r.agentName || basename(r.cwd)}
                      </span>
                      {r.status === 'active' && (
                        <span
                          style={{
                            marginLeft: 6,
                            fontSize: '0.55rem',
                            color: 'var(--wks-success, #4ade80)',
                          }}
                        >
                          ● live
                        </span>
                      )}
                    </td>
                    <td style={{ ...td, color: 'var(--wks-text-secondary)' }}>
                      {shortModel(r.model)}
                    </td>
                    <td style={tdNum}>{fmtTokens(r.inputTokens + r.outputTokens)}</td>
                    <td style={{ ...tdNum, color: 'var(--wks-accent)' }}>{fmtUSD(r.costUSD)}</td>
                    <td style={tdNum}>{r.toolCalls}</td>
                    <td style={tdNum}>{fmtDuration(r.durationMs)}</td>
                    <td style={{ ...td, color: 'var(--wks-text-faint)' }}>
                      {r.startedAt ? new Date(r.startedAt).toLocaleString() : ''}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </>
      )}
    </div>
  );
};

const th: React.CSSProperties = {
  padding: '4px 8px',
  fontWeight: 600,
  textTransform: 'uppercase',
  letterSpacing: '0.04em',
  fontSize: '0.55rem',
};
const thNum: React.CSSProperties = { ...th, textAlign: 'right' };
const td: React.CSSProperties = {
  padding: '5px 8px',
  whiteSpace: 'nowrap',
  overflow: 'hidden',
  textOverflow: 'ellipsis',
  maxWidth: 220,
};
const tdNum: React.CSSProperties = {
  ...td,
  textAlign: 'right',
  fontVariantNumeric: 'tabular-nums',
  color: 'var(--wks-text-secondary)',
};
const refreshBtn: React.CSSProperties = {
  marginLeft: 'auto',
  width: 24,
  height: 24,
  borderRadius: 6,
  border: '1px solid var(--wks-border-input)',
  background: 'transparent',
  color: 'var(--wks-text-faint)',
  cursor: 'pointer',
  fontSize: '0.85rem',
};

export default AnalyticsPane;
