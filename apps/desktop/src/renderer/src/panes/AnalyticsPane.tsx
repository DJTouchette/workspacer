import React, { useCallback, useEffect, useRef, useState } from 'react';
import type { AnalyticsSummary, AnalyticsBucket, SessionHistoryRecord } from '../types/analytics';
import { usePageVisible } from '../hooks/usePageVisible';
import { shortModelLabel } from '../lib/modelLabel';
import { AgentLogo } from '../components/agentLogos';
import { BarChart3 } from '../components/icons';
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
        fontSize: '0.58rem',
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
      fontSize: '0.58rem',
      color: 'var(--wks-text-faint)',
      textTransform: 'uppercase',
      letterSpacing: '0.08em',
      fontWeight: 600,
      margin: '20px 0 8px',
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

/** "By model": a stacked share bar over a legend of name / tokens / share% /
 *  cost. Deliberately no session counts — a session can span models (and
 *  subagents), so per-model sessions would mislead. */
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
              color: 'var(--wks-text-secondary)',
              fontVariantNumeric: 'tabular-nums',
            }}
          >
            {fmtTokens(m.tokens)} tok
          </span>
          <span
            style={{
              fontSize: '0.7rem',
              color: 'var(--wks-text-faint)',
              minWidth: 34,
              textAlign: 'right',
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

const Empty: React.FC = () => (
  <div style={{ fontSize: '0.7rem', color: 'var(--wks-text-faint)', padding: '8px 0' }}>
    No data yet.
  </div>
);

/** Segmented toggle button (time-range picker). */
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

/** Selectable time ranges; `days: null` = all time. Default is the last month. */
const RANGES = [
  { id: '24h', label: '24h', caption: 'last 24 hours', days: 1 },
  { id: '7d', label: '7d', caption: 'last 7 days', days: 7 },
  { id: '30d', label: '30d', caption: 'last 30 days', days: 30 },
  { id: '90d', label: '90d', caption: 'last 90 days', days: 90 },
  { id: 'all', label: 'All', caption: 'all time', days: null },
] as const;
type RangeId = (typeof RANGES)[number]['id'];

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

  // Time range — defaults to the last month; persisted like the provider filter.
  const [rangeId, setRangeId] = useState<RangeId>(() => {
    try {
      const v = localStorage.getItem('wks-usage-range');
      return RANGES.some((r) => r.id === v) ? (v as RangeId) : '30d';
    } catch {
      return '30d';
    }
  });
  const pickRange = (v: RangeId) => {
    setRangeId(v);
    try {
      localStorage.setItem('wks-usage-range', v);
    } catch {
      /* private mode */
    }
  };
  const range = RANGES.find((r) => r.id === rangeId) ?? RANGES[2];

  const refresh = useCallback(() => {
    // 'all' ⇒ no scope; otherwise scope totals/breakdowns to one backend. The
    // byProvider split inside the summary spans all backends (same time window)
    // so the combined picture stays visible while filtered.
    const scope = provider === 'all' ? undefined : provider;
    const since =
      range.days !== null
        ? new Date(Date.now() - range.days * 86_400_000).toISOString()
        : undefined;
    window.electronAPI
      .analyticsSummary?.(scope, since)
      .then(setSummary)
      .catch(() => {});
    window.electronAPI
      .analyticsRecent?.(100, scope, since)
      .then((r) => setRecent(Array.isArray(r) ? r : []))
      .catch(() => {});
  }, [provider, range.days]);

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

  // Initial load on mount, and whenever a filter changes.
  useEffect(() => {
    refresh();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [provider, rangeId]);

  const t = summary?.totals;
  const byDay = summary?.byDay ?? [];
  const periodSpend = byDay.reduce((sum, d) => sum + d.costUSD, 0);
  const weekSpend = byDay.slice(-7).reduce((sum, d) => sum + d.costUSD, 0);

  return (
    <div
      style={{
        position: 'relative',
        height: '100%',
        overflow: 'hidden',
        background: 'var(--wks-bg-base)',
      }}
    >
      {/* Soft accent glow behind the hero — same decoration as the spawn
          dialog and Overview, fixed while the content scrolls beneath it. */}
      <div
        aria-hidden
        style={{
          position: 'absolute',
          top: '-22%',
          left: '50%',
          transform: 'translateX(-50%)',
          width: 720,
          height: 720,
          borderRadius: '50%',
          background:
            'radial-gradient(circle, color-mix(in srgb, var(--wks-accent) 8%, transparent) 0%, transparent 65%)',
          pointerEvents: 'none',
        }}
      />

      <div style={{ position: 'relative', height: '100%', overflowY: 'auto' }}>
        <div
          style={{
            maxWidth: 860,
            margin: '0 auto',
            padding: '44px 28px 40px',
            boxSizing: 'border-box',
            animation: 'wks-fade-in 0.25s ease-out',
          }}
        >
          {/* ── Hero ────────────────────────────────────────────────────── */}
          <div style={{ textAlign: 'center', marginBottom: 24 }}>
            <div
              style={{
                width: 64,
                height: 64,
                margin: '0 auto',
                borderRadius: '50%',
                display: 'flex',
                alignItems: 'center',
                justifyContent: 'center',
                border: '1px solid var(--wks-border-input)',
                background: 'color-mix(in srgb, var(--wks-accent) 5%, transparent)',
                color: 'var(--wks-accent-text, var(--wks-text-primary))',
              }}
            >
              <BarChart3 size={26} strokeWidth={1.7} />
            </div>
            <div
              style={{
                marginTop: 16,
                fontSize: '1.05rem',
                fontWeight: 650,
                letterSpacing: '-0.01em',
                color: 'var(--wks-text-primary)',
              }}
            >
              Usage &amp; cost
            </div>
            <div
              style={{
                marginTop: 6,
                fontSize: '0.72rem',
                color: 'var(--wks-text-muted)',
                fontVariantNumeric: 'tabular-nums',
                display: 'flex',
                justifyContent: 'center',
                alignItems: 'center',
                gap: 7,
                flexWrap: 'wrap',
              }}
            >
              <span>{range.caption}</span>
              <span style={{ color: 'var(--wks-text-disabled)' }}>·</span>
              <span>{provider === 'all' ? 'all agents' : providerLabel(provider)}</span>
            </div>
          </div>

          {/* ── Controls: provider filter · time range · refresh ─────────── */}
          {/* Provider pills scope every figure to one backend, or fold them all
              together. The "By provider" card below always shows the full split. */}
          <div
            style={{
              display: 'flex',
              justifyContent: 'center',
              alignItems: 'center',
              gap: 12,
              flexWrap: 'wrap',
              marginBottom: 26,
            }}
          >
            <div style={{ display: 'flex', gap: 6, flexWrap: 'wrap' }}>
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
            <div
              style={{
                display: 'flex',
                background: 'var(--wks-bg-surface)',
                border: '1px solid var(--wks-border-subtle)',
                borderRadius: 'var(--wks-radius-md)',
                padding: 3,
              }}
            >
              {RANGES.map((r) => (
                <SegBtn key={r.id} active={rangeId === r.id} onClick={() => pickRange(r.id)}>
                  {r.label}
                </SegBtn>
              ))}
            </div>
            <button onClick={refresh} style={{ ...refreshBtn, marginLeft: 0 }} title="Refresh">
              ↻
            </button>
          </div>

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

          {/* Daily spend + provider split */}
          <div
            style={{
              display: 'grid',
              gridTemplateColumns: 'minmax(280px, 1.7fr) minmax(220px, 1fr)',
              gap: 12,
              marginBottom: 14,
            }}
          >
            <ChartCard title="Daily spend" caption={`${fmtUSD(periodSpend)} this period`}>
              <CostBars data={byDay} />
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

          <SectionTitle>Sessions</SectionTitle>
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
        </div>
      </div>
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
