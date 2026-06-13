// ── Agent utility helpers shared across workflow/subagent components ──

export const AGENT_PURPLE = '#c084fc';

export const fmtTokens = (n?: number): string => {
  if (!n) return '';
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1000) return `${(n / 1000).toFixed(n >= 10_000 ? 0 : 1)}k`;
  return String(n);
};

export const fmtDuration = (ms?: number): string => {
  if (ms === undefined || ms < 0) return '';
  const s = Math.floor(ms / 1000);
  if (s < 60) return `${s}s`;
  return `${Math.floor(s / 60)}m${String(s % 60).padStart(2, '0')}s`;
};

/** "claude-sonnet-4-6" to "sonnet-4-6" (enough to tell agents apart at a glance) */
export const shortModel = (m?: string): string =>
  m ? m.replace(/^claude-/, '').replace(/-\d{8}$/, '') : '';
