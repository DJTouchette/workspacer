import React, { useState } from 'react';
import type { UsageReportAccount, UsageReportWire } from '../../../main/shared/usageReport';
import {
  usageAccountIdentity,
  usagePaceFallbackLabel,
  usagePaceLook,
  usagePacingRows,
} from '../lib/usagePacing';
import { UsageDetailDialog } from './claude/UsageDetailDialog';
import { Surface } from './Surface';
import { fmtResetIn } from '../lib/sessionStats';

/** Report accounts belong to this backend's hub, never to federated sessions. */
export function UsageReportCard({
  report,
  account,
  provider,
  compact,
  nowMs,
}: {
  report: UsageReportWire;
  account: UsageReportAccount;
  provider: string;
  /** Full width and tighter padding, for the Inspector's narrow column. The
   *  card's CONTENT is identical either way — a second layout of the same
   *  numbers is how two surfaces start disagreeing. */
  compact?: boolean;
  /** The clock the rows are judged against. Only a caller that has ALREADY
   *  asked whether this account has a current window passes it, so its answer
   *  and the card's cannot come from two different instants. */
  nowMs?: number;
}) {
  const [open, setOpen] = useState(false);
  const { rows, fields } = usagePacingRows(report, account, nowMs ?? Date.now());
  const identity = usageAccountIdentity(account);
  if (!rows.length) return null;
  return (
    <>
      <Surface
        elevation="raised"
        interactive
        role="button"
        tabIndex={0}
        aria-label={`Show usage detail: ${provider} · ${identity}`}
        onClick={() => setOpen(true)}
        onKeyDown={(e) => {
          if (e.key === 'Enter' || e.key === ' ') {
            e.preventDefault();
            setOpen(true);
          }
        }}
        style={
          compact
            ? { width: '100%', minWidth: 0, padding: 10, cursor: 'pointer' }
            : { flex: '1 1 220px', minWidth: 0, padding: 16, cursor: 'pointer' }
        }
      >
        <div style={{ fontSize: '0.72rem', fontWeight: 600, overflowWrap: 'anywhere' }}>
          {provider} usage · {account.label || identity}
        </div>
        <div
          style={{
            fontSize: '0.6rem',
            color: 'var(--wks-text-secondary)',
            overflowWrap: 'anywhere',
            marginBottom: 8,
          }}
        >
          {identity}
        </div>
        {rows.map((row) => (
          <div
            key={row.key}
            title={row.description}
            aria-label={`${row.label} · ${row.description}`}
            style={{ marginTop: 8 }}
          >
            <div
              style={{
                display: 'flex',
                flexWrap: 'wrap',
                gap: 8,
                fontSize: '0.66rem',
                marginBottom: 4,
              }}
            >
              <span>{row.label}</span>
              <span>{row.pct === undefined ? 'Unknown' : `${Math.round(row.pct)}%`}</span>
              <span style={{ color: usagePaceLook(row.verdict).color }}>
                {usagePaceLook(row.verdict).label ?? usagePaceFallbackLabel(report, account)}
              </span>
              {row.reset !== undefined && (
                <span style={{ color: 'var(--wks-text-secondary)' }}>{fmtResetIn(row.reset)}</span>
              )}
            </div>
            <div
              style={{
                position: 'relative',
                height: 6,
                borderRadius: 'var(--wks-radius-pill)',
                background: 'var(--wks-border-subtle)',
              }}
            >
              {row.pct !== undefined && (
                <div
                  data-testid="usage-consumed"
                  style={{
                    height: '100%',
                    width: `${row.pct}%`,
                    borderRadius: 'var(--wks-radius-pill)',
                    // Same mapper as the word above it, so a bar can never be
                    // one colour while the label claims another.
                    background: usagePaceLook(row.verdict).color,
                  }}
                />
              )}
              {row.expected !== undefined && (
                <span
                  data-testid="usage-expected"
                  style={{
                    position: 'absolute',
                    left: `${row.expected}%`,
                    top: -3,
                    height: 12,
                    width: 2,
                    transform: 'translateX(-1px)',
                    background: 'var(--wks-accent)',
                    boxShadow: '0 0 0 1px var(--wks-bg-raised)',
                  }}
                />
              )}
            </div>
          </div>
        ))}
        <div
          style={{
            fontSize: '0.6rem',
            color: 'var(--wks-text-secondary)',
            marginTop: 8,
            overflowWrap: 'anywhere',
          }}
        >
          {account.source || 'Unknown source'} ·{' '}
          {typeof account.observed_at === 'number' && Number.isFinite(account.observed_at)
            ? `Observed ${new Date(account.observed_at * 1000).toLocaleString()}`
            : 'Observation time unknown'}
        </div>
      </Surface>
      {open && (
        <UsageDetailDialog
          snapshot={{ provider, statusLine: fields }}
          scope="account"
          accountIdentity={identity}
          onClose={() => setOpen(false)}
        />
      )}
    </>
  );
}
