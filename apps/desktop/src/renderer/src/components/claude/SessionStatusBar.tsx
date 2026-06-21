import React from 'react';
import type { ClaudeSessionSnapshot } from '../../types/claudeSession';
import { deriveSessionStats } from '../../lib/sessionStats';

/**
 * A compact, single-line status readout — Workspacer's in-app equivalent of
 * Claude Code's terminal status line. Rendered inline in the agent pane's top
 * toolbar (next to the status badge).
 *
 * Data comes from `snapshot.statusLine` (fed by claudemon's /statusline/stream,
 * the only channel carrying Claude's authoritative context-%, cost, and the
 * 5h/7d rate-limit windows). Where the statusLine hasn't arrived yet we fall
 * back to the transcript-derived `usage` so it isn't blank.
 *
 * Note: git branch isn't shown — it lives only in Claude's statusLine *script*
 * (which shells out to git), not in the JSON channel we ingest.
 */

function fmtTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`;
  return `${n}`;
}
function fmtUSD(n: number): string {
  return n >= 0.01 ? `$${n.toFixed(2)}` : n > 0 ? '<$0.01' : '$0.00';
}
function ctxColor(pct: number): string {
  if (pct >= 80) return 'var(--wks-danger, #e05555)';
  if (pct >= 50) return 'var(--wks-warning, #e0a000)';
  return 'var(--wks-success, #3fb950)';
}
function baseName(p: string | undefined): string {
  if (!p) return '';
  return p.replace(/[/\\]+$/, '').split(/[/\\]/).pop() || p;
}

/** A thin vertical rule between HUD groups, replacing the ASCII pipe. */
const Sep: React.FC = () => (
  <span style={{ width: 1, height: 15, flexShrink: 0, background: 'var(--wks-border, #555)', opacity: 0.7 }} />
);

/** Segmented 10-tick context gauge — filled ticks take the threshold color. */
const CtxBar: React.FC<{ pct: number }> = ({ pct }) => {
  const filled = Math.max(0, Math.min(10, Math.round((pct / 100) * 10)));
  const color = ctxColor(pct);
  return (
    <span style={{ display: 'inline-flex', gap: 2, alignItems: 'center' }}>
      {Array.from({ length: 10 }, (_, i) => (
        <span
          key={i}
          style={{
            width: 4, height: 11, borderRadius: 1,
            background: i < filled ? color : 'var(--wks-bg-elevated, #444)',
          }}
        />
      ))}
    </span>
  );
};

interface Props {
  snapshot?: ClaudeSessionSnapshot | null;
  cwd?: string;
}

export const SessionStatusBar: React.FC<Props> = ({ snapshot, cwd }) => {
  const dir = baseName(cwd || snapshot?.cwd);
  const { model, ctxPct, tokens, costUSD: cost, fiveHourPct: five, sevenDayPct: seven } =
    deriveSessionStats(snapshot);

  // Until the first reading arrives, render nothing so the toolbar stays clean.
  const hasAny = model || ctxPct !== undefined || tokens !== undefined || cost !== undefined;
  if (!hasAny) return null;

  return (
    <span
      style={{
        display: 'inline-flex',
        alignItems: 'center',
        gap: 7,
        minWidth: 0,
        overflow: 'hidden',
        fontFamily: 'var(--wks-font-mono, var(--claude-mono-font, monospace))',
        fontSize: '0.64rem',
        whiteSpace: 'nowrap',
      }}
    >
      {dir && <span style={{ color: 'var(--wks-accent-text)', fontWeight: 600 }}>{dir}</span>}
      {model && (<><Sep /><span style={{ color: 'var(--wks-text-secondary)' }}>{model}</span></>)}
      {ctxPct !== undefined && (
        <>
          <Sep />
          <span style={{ display: 'inline-flex', alignItems: 'center', gap: 5, color: ctxColor(ctxPct), fontVariantNumeric: 'tabular-nums' }}>
            <span style={{ color: 'var(--wks-text-faint)' }}>ctx</span>
            <CtxBar pct={ctxPct} />
            {Math.round(ctxPct)}%
          </span>
        </>
      )}
      {(tokens !== undefined || cost !== undefined) && (
        <>
          <Sep />
          <span style={{ display: 'inline-flex', alignItems: 'center', gap: 5, fontVariantNumeric: 'tabular-nums' }}>
            {tokens !== undefined && (
              <span><span style={{ color: 'var(--wks-text-faint)' }}>tok </span><span style={{ color: 'var(--wks-text-secondary)' }}>{fmtTokens(tokens)}</span></span>
            )}
            {cost !== undefined && <span style={{ color: 'var(--wks-accent-text)' }}>{fmtUSD(cost)}</span>}
          </span>
        </>
      )}
      {(five !== undefined || seven !== undefined) && (
        <>
          <Sep />
          <span style={{ color: 'var(--wks-text-secondary)', fontVariantNumeric: 'tabular-nums' }}>
            {five !== undefined && (<><span style={{ color: 'var(--wks-text-faint)' }}>5h </span>{Math.round(five)}%</>)}
            {seven !== undefined && (<> <span style={{ color: 'var(--wks-text-faint)' }}>7d </span>{Math.round(seven)}%</>)}
          </span>
        </>
      )}
    </span>
  );
};

export default SessionStatusBar;
