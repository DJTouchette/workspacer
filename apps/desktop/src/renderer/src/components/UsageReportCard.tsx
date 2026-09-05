import React, { useState } from 'react';
import type { UsageReportAccount, UsageReportWire } from '../../../main/shared/usageReport';
import {
  USAGE_EXPECTED_LEGEND,
  usageAccountIdentity,
  usagePaceFallbackLabel,
  usagePaceLook,
  usagePacingRows,
  usageStaleNote,
} from '../lib/usagePacing';
import { UsageDetailDialog } from './claude/UsageDetailDialog';
import { Surface } from './Surface';

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
  const staleNote = usageStaleNote(report, account);
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
            {/* One wrapping line per window. The numeric pair stays adjacent
                and the reset and the verdict are what wrap away first, which is
                what keeps a 320px rail readable without a second layout. */}
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
              {/* An absent measurement says so. It is never turned into "0%
                  used" or its mirror "100% left" — see UsagePacingRow.left. */}
              <span>{row.usedPct === undefined ? 'Usage unknown' : `${row.usedPct}% used`}</span>
              {row.left !== undefined && (
                <span style={{ color: 'var(--wks-text-secondary)' }}>{row.left}% left</span>
              )}
              {row.resetPhrase !== undefined && (
                <span style={{ color: 'var(--wks-text-secondary)' }}>{row.resetPhrase}</span>
              )}
              <span style={{ color: usagePaceLook(row.verdict).color }}>
                {usagePaceLook(row.verdict).label ?? usagePaceFallbackLabel(report, account)}
              </span>
            </div>
            <div
              // The meter carries the whole row in its accessible name: window,
              // what is spent, what is left and when it resets. Colour states
              // none of that, and the tick is a 2px mark.
              {...(row.usedPct === undefined
                ? {}
                : {
                    role: 'meter',
                    'aria-valuemin': 0,
                    'aria-valuemax': 100,
                    'aria-valuenow': row.usedPct,
                    'aria-valuetext': row.description,
                    'aria-label': `${row.label} allowance`,
                  })}
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
                  title={`${Math.round(row.expected)}% expected by now`}
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
        {/* The tick explained once, in words, for the rows that actually drew
            one. A marker whose meaning is carried by position alone is a marker
            only its author can read. */}
        {rows.some((row) => row.expected !== undefined) && (
          <div
            style={{
              display: 'flex',
              alignItems: 'center',
              gap: 5,
              fontSize: '0.6rem',
              color: 'var(--wks-text-secondary)',
              marginTop: 8,
            }}
          >
            <span
              aria-hidden="true"
              style={{
                display: 'inline-block',
                height: 9,
                width: 2,
                flexShrink: 0,
                background: 'var(--wks-accent)',
              }}
            />
            {USAGE_EXPECTED_LEGEND}
          </div>
        )}
        {/* A percentage kept from an observation nobody has confirmed since is
            history. It stays on screen, but it says which it is. */}
        {staleNote !== undefined && (
          <div
            style={{
              fontSize: '0.6rem',
              color: 'var(--wks-text-secondary)',
              marginTop: 8,
              overflowWrap: 'anywhere',
            }}
          >
            {staleNote}
          </div>
        )}
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
