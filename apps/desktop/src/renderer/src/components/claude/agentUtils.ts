// ── Agent utility helpers shared across workflow/subagent components ──

export const AGENT_PURPLE = '#c084fc';

export const fmtTokens = (n?: number): string => {
  if (!n) return '';
  // Guard against the 'k' branch's toFixed(0) rounding n/1000 up to 1000:
  // anything that would render as "1000k" (n/1000 >= 999.5) is promoted to M.
  if (n / 1000 >= 999.5) return `${(n / 1_000_000).toFixed(1)}M`;
  // Decide precision from the value that will actually render: anything that
  // rounds to >= 10k drops the decimal, so 9999 → "10k" not "10.0k".
  if (n >= 1000) return `${(n / 1000).toFixed(Math.round(n / 100) / 10 >= 10 ? 0 : 1)}k`;
  return String(n);
};

export const fmtDuration = (ms?: number): string => {
  if (ms === undefined || ms < 0) return '';
  const s = Math.floor(ms / 1000);
  if (s < 60) return `${s}s`;
  return `${Math.floor(s / 60)}m${String(s % 60).padStart(2, '0')}s`;
};

/** "claude-sonnet-4-6" to "sonnet-4-6" (enough to tell agents apart at a glance).
 *  Provider-agnostic — see lib/modelLabel. */
export { shortModelLabel as shortModel } from '../../lib/modelLabel';
