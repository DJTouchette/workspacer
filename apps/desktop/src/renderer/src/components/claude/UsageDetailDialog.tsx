/**
 * Usage detail — what the status bar's usage readout opens when clicked.
 *
 * The toolbar is the cramped surface, so it stays a glance: one chip, threshold
 * colour, a percentage. Everything else workspacer actually knows about this
 * session's usage lives here, where there is room to lay it out.
 *
 * The dialog is built from the data present rather than a fixed layout. Account
 * windows come from `usageWindows()`, which lists only the windows a provider
 * genuinely reported: Claude has no monthly window unless extra usage is enabled
 * on the account, and Codex reports two windows and never a monthly one. A
 * window with a reset time but no percentage renders its reset row and no meter,
 * and a window whose length the provider does not report is simply labelled
 * without one. Nothing here draws a zeroed meter for something that is absent,
 * and a window that starts arriving later shows up with no further work.
 *
 * There are deliberately no absolute quota figures ("$8.40 of $45.00"). The
 * OAuth usage payload claudemon polls does carry some, but the field names come
 * from a checked-in capture and could not be re-verified against the live
 * endpoint, so showing them would risk a confidently wrong number.
 *
 * Cost is labelled an estimate everywhere it appears here, because that is what
 * it is: a local rate table applied to token counts. It is not a billed figure
 * and must not be read as one.
 *
 * The prompt-cache section follows the same omit-rather-than-zero rule as the
 * windows. Codex reports a cache-read subset of its input and nothing about
 * writes, so its breakdown carries no write row at all rather than a 0 that
 * would claim it wrote nothing, and a session whose provider itemizes nothing
 * gets no section.
 */

import React from 'react';
import { createPortal } from 'react-dom';
import { BarChart3, X, AlertTriangle } from 'lucide-react';
import {
  deriveSessionStats,
  cacheBreakdown,
  usageWindows,
  fmtWindowLength,
  fmtResetAt,
  fmtResetIn,
  fmtTokens,
  fmtUSD,
  ctxColor,
} from '../../lib/sessionStats';
import type { SessionStatsSource } from '../../lib/sessionStats';
import { AgentLogo } from '../agentLogos';
import type { AgentProvider } from '../../types/pane';

/** Meter matching the family the sidebar, agent cards and status bar all use:
 *  subtle-border track, threshold-coloured fill, 2% floor so a tiny non-zero
 *  reading still shows a sliver. */
const Meter: React.FC<{ pct: number }> = ({ pct }) => {
  const clamped = Math.max(0, Math.min(100, pct));
  return (
    <div
      style={{
        height: 6,
        borderRadius: 'var(--wks-radius-pill)',
        background: 'var(--wks-border-subtle)',
        overflow: 'hidden',
      }}
    >
      <div
        style={{
          height: '100%',
          width: `${clamped > 0 ? Math.max(2, clamped) : 0}%`,
          borderRadius: 'var(--wks-radius-pill)',
          background: ctxColor(clamped),
        }}
      />
    </div>
  );
};

/** Flat accent bar for a share that has no good/bad direction.
 *  {@link Meter} colours by threshold, which is right for a limit being consumed
 *  and wrong for a cache hit rate: it would paint an excellent 95% red. There is
 *  no defensible threshold for "a good hit rate", so this one states the share
 *  and claims nothing about it. */
const ShareBar: React.FC<{ pct: number }> = ({ pct }) => {
  const clamped = Math.max(0, Math.min(100, pct));
  return (
    <div
      style={{
        height: 6,
        borderRadius: 'var(--wks-radius-pill)',
        background: 'var(--wks-border-subtle)',
        overflow: 'hidden',
      }}
    >
      <div
        style={{
          height: '100%',
          width: `${clamped > 0 ? Math.max(2, clamped) : 0}%`,
          borderRadius: 'var(--wks-radius-pill)',
          background: 'var(--wks-accent)',
        }}
      />
    </div>
  );
};

/** A label/value line for the session facts that are numbers, not meters. */
const Fact: React.FC<{ label: string; value: string }> = ({ label, value }) => (
  <div
    style={{
      display: 'flex',
      justifyContent: 'space-between',
      alignItems: 'baseline',
      gap: 12,
      fontSize: '0.72rem',
      padding: '4px 0',
    }}
  >
    <span style={{ color: 'var(--wks-text-muted)' }}>{label}</span>
    <span
      style={{
        color: 'var(--wks-text-primary)',
        fontVariantNumeric: 'tabular-nums',
        fontWeight: 500,
        textAlign: 'right',
        minWidth: 0,
      }}
    >
      {value}
    </span>
  </div>
);

const SectionTitle: React.FC<{ children: React.ReactNode }> = ({ children }) => (
  <div
    style={{
      fontSize: '0.6rem',
      letterSpacing: '0.08em',
      textTransform: 'uppercase',
      color: 'var(--wks-text-faint)',
      fontWeight: 600,
      marginBottom: 8,
    }}
  >
    {children}
  </div>
);

/** What the dialog needs to draw. A whole snapshot satisfies it, and so does
 *  the bare statusLine the Overview's account card holds — that card covers an
 *  account, not a session, so there is no snapshot to hand over. */
export type UsageDetailSource = SessionStatsSource & { provider?: string };

export const UsageDetailDialog: React.FC<{
  snapshot?: UsageDetailSource | null;
  /** `session` (default) adds the opening session's own model/tokens/cost.
   *  `account` is for surfaces that speak for a whole account and have no one
   *  session behind them, where those figures would be somebody else's. */
  scope?: 'session' | 'account';
  onClose: () => void;
}> = ({ snapshot, scope = 'session', onClose }) => {
  React.useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.stopPropagation();
        onClose();
      }
    };
    window.addEventListener('keydown', onKey, true);
    return () => window.removeEventListener('keydown', onKey, true);
  }, [onClose]);

  const stats = deriveSessionStats(snapshot);
  const windows = usageWindows(stats);
  const cache = scope === 'session' ? cacheBreakdown(snapshot) : null;
  const sl = snapshot?.statusLine;
  const provider = (snapshot?.provider as AgentProvider | undefined) ?? 'claude';

  // Session facts: only the ones that arrived. A missing cost is a session that
  // has not been priced yet, not a free one, so it gets no row. An account-scoped
  // opener contributes none of them: the statusLine it holds belongs to whichever
  // session reported the account's windows last, and that session's cost is not
  // the reader's.
  const facts: Array<{ label: string; value: string }> = [];
  const sessionScoped = scope === 'session';
  if (sessionScoped && stats.model) facts.push({ label: 'Model', value: stats.model });
  if (sessionScoped && stats.ctxPct !== undefined) {
    facts.push({
      label: 'Context used',
      value: sl?.contextWindowSize
        ? `${Math.round(stats.ctxPct)}% of ${fmtTokens(sl.contextWindowSize)}`
        : `${Math.round(stats.ctxPct)}%`,
    });
  }
  if (sessionScoped && sl?.totalInputTokens !== undefined)
    facts.push({ label: 'Input tokens', value: fmtTokens(sl.totalInputTokens) });
  if (sessionScoped && sl?.totalOutputTokens !== undefined)
    facts.push({ label: 'Output tokens', value: fmtTokens(sl.totalOutputTokens) });
  if (sessionScoped && stats.tokens !== undefined)
    facts.push({ label: 'Total tokens', value: fmtTokens(stats.tokens) });
  if (sessionScoped && stats.costUSD !== undefined)
    facts.push({ label: 'Estimated cost', value: fmtUSD(stats.costUSD) });

  const received = sl?.receivedAt ? new Date(sl.receivedAt) : undefined;

  // Portalled: the status bar this opens from lives inside an overflow-hidden
  // toolbar row, and a dialog must not be able to inherit that clip.
  return createPortal(
    <div
      onClick={onClose}
      style={{
        position: 'fixed',
        inset: 0,
        zIndex: 20000,
        background: 'var(--wks-overlay)',
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        animation: 'wks-fade-in 0.15s ease-out',
      }}
    >
      <div
        role="dialog"
        aria-label="Usage detail"
        onClick={(e) => e.stopPropagation()}
        style={{
          width: 420,
          maxWidth: '92vw',
          maxHeight: '86vh',
          overflowY: 'auto',
          background: 'var(--wks-bg-raised)',
          borderRadius: 'var(--wks-radius-lg)',
          boxShadow: 'var(--wks-shadow)',
          padding: '20px 20px 16px',
          boxSizing: 'border-box',
          fontFamily: 'var(--wks-font-sans)',
        }}
      >
        {/* ── Header ─────────────────────────────────────────────────── */}
        <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
          <BarChart3 size={15} strokeWidth={1.9} style={{ color: 'var(--wks-accent)' }} />
          <span style={{ fontSize: '1.05rem', fontWeight: 700, color: 'var(--wks-text-primary)' }}>
            Usage
          </span>
          <AgentLogo provider={provider} size={13} style={{ opacity: 0.7 }} />
          <button
            onClick={onClose}
            aria-label="Close"
            style={{
              marginLeft: 'auto',
              display: 'flex',
              alignItems: 'center',
              background: 'transparent',
              border: 'none',
              cursor: 'pointer',
              color: 'var(--wks-text-muted)',
              padding: 4,
            }}
          >
            <X size={14} strokeWidth={2} />
          </button>
        </div>

        {sl?.rateLimitWarning && (
          <div
            style={{
              marginTop: 12,
              display: 'flex',
              alignItems: 'center',
              gap: 6,
              fontSize: '0.72rem',
              color: 'var(--wks-warning)',
              background: 'color-mix(in srgb, var(--wks-warning) 10%, transparent)',
              borderRadius: 'var(--wks-radius-sm)',
              padding: '6px 10px',
            }}
          >
            <AlertTriangle size={12} strokeWidth={2} style={{ flexShrink: 0 }} />
            {sl.rateLimitWarning}
          </div>
        )}

        {/* ── Account windows ────────────────────────────────────────── */}
        <div style={{ marginTop: 16 }}>
          <SectionTitle>Account limits</SectionTitle>
          {windows.length === 0 ? (
            <div style={{ fontSize: '0.72rem', color: 'var(--wks-text-muted)', lineHeight: 1.5 }}>
              This provider has not reported any rate-limit windows for the account yet.
            </div>
          ) : (
            windows.map((w) => {
              const length = fmtWindowLength(w.windowMins);
              const resetsIn = fmtResetIn(w.resetsAt);
              const resetsAt = fmtResetAt(w.resetsAt);
              return (
                <div key={w.key} style={{ marginBottom: 12 }}>
                  <div
                    style={{
                      display: 'flex',
                      justifyContent: 'space-between',
                      alignItems: 'baseline',
                      gap: 10,
                      fontSize: '0.72rem',
                      marginBottom: 4,
                    }}
                  >
                    <span style={{ color: 'var(--wks-text-secondary)', fontWeight: 500 }}>
                      {w.label}
                    </span>
                    <span
                      style={{
                        color: 'var(--wks-text-primary)',
                        fontVariantNumeric: 'tabular-nums',
                        fontWeight: 600,
                      }}
                    >
                      {w.pct !== undefined ? `${Math.round(w.pct)}%` : ''}
                    </span>
                  </div>
                  {/* No percentage means the provider sent only a reset time.
                      A meter there would read as 0% used, which is a claim we
                      cannot make, so the row goes without one. */}
                  {w.pct !== undefined && <Meter pct={w.pct} />}
                  <div
                    style={{
                      marginTop: 4,
                      fontSize: '0.66rem',
                      color: 'var(--wks-text-muted)',
                      display: 'flex',
                      flexWrap: 'wrap',
                      gap: 8,
                    }}
                  >
                    {length && <span>Window {length}</span>}
                    {resetsIn && <span>Resets in {resetsIn}</span>}
                    {resetsAt && <span style={{ color: 'var(--wks-text-faint)' }}>{resetsAt}</span>}
                  </div>
                </div>
              );
            })
          )}
          {sl?.overageOutOfCredits && (
            <div
              style={{
                fontSize: '0.66rem',
                color: 'var(--wks-text-muted)',
                lineHeight: 1.5,
                marginTop: 2,
              }}
            >
              Extra usage is off for this account, so there is no monthly credit window to show.
            </div>
          )}
        </div>

        {/* ── This session ───────────────────────────────────────────── */}
        {facts.length > 0 && (
          <div style={{ marginTop: 16 }}>
            <SectionTitle>This session</SectionTitle>
            {facts.map((f) => (
              <Fact key={f.label} label={f.label} value={f.value} />
            ))}
          </div>
        )}

        {/* ── Prompt cache ───────────────────────────────────────────── */}
        {cache && (
          <div style={{ marginTop: 16 }}>
            <SectionTitle>Prompt cache</SectionTitle>
            <Fact label="Fresh input" value={fmtTokens(cache.fresh)} />
            {/* Omitted, not zeroed, when the provider never itemizes writes. */}
            {cache.write !== undefined && (
              <Fact label="Written to cache" value={fmtTokens(cache.write)} />
            )}
            <Fact label="Read from cache" value={fmtTokens(cache.read)} />
            {/* A hit rate needs a prompt to be a share of. With nothing counted
                yet the denominator is zero and the rate is undefined, not 0%. */}
            {cache.hitRatePct !== undefined && (
              <>
                <Fact
                  label="Cache hit rate"
                  value={`${Math.round(cache.hitRatePct)}% of ${fmtTokens(cache.total)}`}
                />
                <div style={{ marginTop: 4 }}>
                  <ShareBar pct={cache.hitRatePct} />
                </div>
              </>
            )}
            <div
              style={{
                marginTop: 8,
                fontSize: '0.66rem',
                color: 'var(--wks-text-muted)',
                lineHeight: 1.5,
              }}
            >
              {cache.write === undefined
                ? 'Cumulative over the session. This provider reports how much of the prompt came from cache but not how much was written to it, so writes are not counted here.'
                : 'Cumulative over the session. The hit rate is cache reads over the whole prompt.'}
            </div>
          </div>
        )}

        <div
          style={{
            marginTop: 14,
            fontSize: '0.6rem',
            color: 'var(--wks-text-faint)',
            lineHeight: 1.5,
          }}
        >
          {received
            ? `Reading from ${received.toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' })}. Account limits are shared by every session on the account.`
            : 'Account limits are shared by every session on the account.'}
          {sessionScoped && stats.costUSD !== undefined
            ? ' Cost is estimated locally from a published rate table, not a billed figure.'
            : ''}
        </div>
      </div>
    </div>,
    document.body,
  );
};

export default UsageDetailDialog;
