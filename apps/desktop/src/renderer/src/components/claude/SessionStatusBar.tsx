import React from 'react';
import type { ClaudeSessionSnapshot } from '../../types/claudeSession';

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
function bar(pct: number): string {
  const filled = Math.max(0, Math.min(10, Math.round((pct / 100) * 10)));
  return `[${'#'.repeat(filled)}${'-'.repeat(10 - filled)}]`;
}
function baseName(p: string | undefined): string {
  if (!p) return '';
  return p.replace(/[/\\]+$/, '').split(/[/\\]/).pop() || p;
}

const Sep: React.FC = () => (
  <span style={{ color: 'var(--wks-text-disabled, #555)', opacity: 0.6 }}>|</span>
);

interface Props {
  snapshot?: ClaudeSessionSnapshot | null;
  cwd?: string;
}

export const SessionStatusBar: React.FC<Props> = ({ snapshot, cwd }) => {
  const sl = snapshot?.statusLine;
  const usage = snapshot?.usage;

  const dir = baseName(cwd || snapshot?.cwd);
  const model = sl?.modelDisplay ?? (usage?.model ? usage.model.replace(/^claude-/, '') : undefined);

  // Context %: prefer Claude's own number, else derive from transcript usage.
  const ctxPct =
    sl?.contextUsedPct ??
    (usage && usage.contextLimit > 0 ? (usage.contextTokens / usage.contextLimit) * 100 : undefined);

  // Total tokens: statusLine carries cumulative in+out; fall back to usage.
  const tokens =
    sl?.totalInputTokens !== undefined
      ? (sl.totalInputTokens ?? 0) + (sl.totalOutputTokens ?? 0)
      : usage
        ? usage.totalInputTokens + usage.totalOutputTokens
        : undefined;

  const cost = sl?.costUSD ?? usage?.costUSD;
  const five = sl?.fiveHourPct;
  const seven = sl?.sevenDayPct;

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
        fontFamily: 'var(--claude-mono-font, monospace)',
        fontSize: '0.58rem',
        whiteSpace: 'nowrap',
      }}
    >
      {dir && <span style={{ color: 'var(--wks-accent-soft, #6cb6ff)', fontWeight: 600 }}>{dir}</span>}
      {model && (<><Sep /><span style={{ color: 'var(--wks-text-secondary, #b08fe0)' }}>{model}</span></>)}
      {ctxPct !== undefined && (
        <>
          <Sep />
          <span style={{ color: ctxColor(ctxPct), fontVariantNumeric: 'tabular-nums' }}>
            ctx:{bar(ctxPct)} {Math.round(ctxPct)}%
          </span>
        </>
      )}
      {(tokens !== undefined || cost !== undefined) && (
        <>
          <Sep />
          <span style={{ color: 'var(--wks-accent-soft, #6cb6ff)', fontVariantNumeric: 'tabular-nums' }}>
            {tokens !== undefined ? `tok:${fmtTokens(tokens)}` : ''}
            {cost !== undefined ? ` ${fmtUSD(cost)}` : ''}
          </span>
        </>
      )}
      {(five !== undefined || seven !== undefined) && (
        <>
          <Sep />
          <span style={{ color: 'var(--wks-info, #4a9eff)', fontVariantNumeric: 'tabular-nums' }}>
            {five !== undefined ? `5h:${Math.round(five)}%` : ''}
            {seven !== undefined ? ` 7d:${Math.round(seven)}%` : ''}
          </span>
        </>
      )}
    </span>
  );
};

export default SessionStatusBar;
