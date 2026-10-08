// Small shared helpers: escaping, formatting, and the icon/mark markup every
// view uses. No state lives here.

export const esc = (s) =>
  String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));

export const basename = (p) => {
  p = String(p || '').replace(/[/\\]+$/, '');
  const i = Math.max(p.lastIndexOf('/'), p.lastIndexOf('\\'));
  return i >= 0 ? p.slice(i + 1) : p;
};
export const oneLine = (s, n) => String(s ?? '').split('\n')[0].slice(0, n || 80);
export const firstLine = (c) => String(c || '').trim().split('\n')[0].replace(/^[#>*\-\s`]+/, '').trim();
export const clamp = (n, lo, hi) => Math.max(lo, Math.min(hi, n));
export const errText = (e) => String((e && e.message) || e || 'failed').replace(/^Error:\s*/, '');

/** "624K", "1.2M" — native's gauge::token_label. */
export function fmtTokens(n) {
  n = Number(n) || 0;
  if (n < 1000) return String(Math.round(n));
  if (n < 1e6) return Math.round(n / 1000) + 'K';
  const m = n / 1e6;
  return (m >= 10 ? Math.round(m) : m.toFixed(1)) + 'M';
}
export function fmtUSD(n) {
  n = Number(n) || 0;
  return n >= 0.01 ? '$' + n.toFixed(2) : n > 0 ? '<$0.01' : '$0.00';
}
/** "18s", "1m 12s", "2h 4m" — native's elapsed form. */
export function fmtElapsed(ms) {
  const s = Math.max(0, Math.round((Number(ms) || 0) / 1000));
  if (s < 60) return s + 's';
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m ${String(s % 60).padStart(2, '0')}s`;
  return `${Math.floor(m / 60)}h ${m % 60}m`;
}
/** "just now", "4m ago", "2h ago", "3d ago" — native cache.rs `ago`. */
export function ago(ms) {
  const minutes = Math.floor(Math.max(0, ms) / 60000);
  if (minutes === 0) return 'just now';
  if (minutes < 60) return `${minutes}m ago`;
  if (minutes < 48 * 60) return `${Math.floor(minutes / 60)}h ago`;
  return `${Math.floor(minutes / (24 * 60))}d ago`;
}
/** A short when for lists: "now", "21:34", "yesterday", "Oct 2". */
export function when(ts) {
  if (!ts) return '';
  const d = new Date(ts), now = new Date();
  if (Date.now() - ts < 60000) return 'now';
  if (d.toDateString() === now.toDateString()) {
    return d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
  }
  const y = new Date(now); y.setDate(now.getDate() - 1);
  if (d.toDateString() === y.toDateString()) return 'yesterday';
  return d.toLocaleDateString([], { month: 'short', day: 'numeric' });
}
/** "2h 14m", "3d 4h", "12m" until a reset (seconds). */
export function resetsIn(resetsAtSec, nowMs = Date.now()) {
  const secs = Math.max(0, resetsAtSec - Math.floor(nowMs / 1000));
  const days = Math.floor(secs / 86400), hours = Math.floor((secs % 86400) / 3600), minutes = Math.floor((secs % 3600) / 60);
  if (!days && !hours) return `${Math.max(1, minutes)}m`;
  if (!days) return `${hours}h ${minutes}m`;
  return `${days}d ${hours}h`;
}

/** FNV-style short hash for signatures (questions, picks). */
export function hash(str) {
  let h = 0;
  for (let i = 0; i < str.length; i++) h = ((h << 5) - h + str.charCodeAt(i)) | 0;
  return String(h >>> 0);
}

// ── markup ─────────────────────────────────────────────────────────────────
/** A Lucide icon from the sprite. `cls` carries size (s12…s20) and tone. */
export const ic = (name, cls = '') =>
  `<svg class="ic ${cls}" aria-hidden="true"><use href="./icons.svg#i-${name}"/></svg>`;
/** A provider mark: Claude keeps its clay, Codex (OpenAI) takes text. */
export const mark = (provider, cls = '') => {
  const p = provider === 'codex' ? 'codex' : provider === 'claude' || !provider ? 'claude' : 'other';
  if (p === 'other') return `<svg class="pm other ${cls}" aria-hidden="true"><use href="./icons.svg#i-bot"/></svg>`;
  return `<svg class="pm ${p} ${cls}" aria-hidden="true"><use href="./icons.svg#b-${p === 'codex' ? 'openai' : 'claude'}"/></svg>`;
};
/** Native's working indicator: the brand mark with its bar bouncing. */
export const spinner = (cls = '') => `<span class="spin ${cls}" aria-hidden="true">{<span class="trk"><i></i></span>}</span>`;
export const brand = `<span class="brand" aria-hidden="true">{<span class="bar"></span>}</span>`;

export const PROVIDER_LABEL = { claude: 'Claude', codex: 'Codex', copilot: 'GitHub Copilot', opencode: 'OpenCode', pi: 'Pi' };
export const providerName = (p) => PROVIDER_LABEL[p || 'claude'] || p || 'Agent';

/** shortModelLabel (desktop lib/modelLabel.ts): keeps a `[1m]` marker. */
export function shortModel(m) {
  if (!m || typeof m !== 'string') return '';
  return m.replace(/^[\w.-]+\//, '').replace(/^claude-/, '').replace(/-\d{6,}$/, '');
}
/** A human model name: "opus-4-8" → "Opus 4.8", "gpt-5.6-sol" → "GPT-5.6 Sol". */
export function modelName(m) {
  let s = shortModel(m);
  if (!s) return '';
  const window = /\[1m\]$/i.test(s) ? ' (1M)' : '';
  s = s.replace(/\[1m\]$/i, '');
  if (/^gpt-/i.test(s)) {
    const [head, ...rest] = s.split('-').slice(1);
    return 'GPT-' + head + (rest.length ? ' ' + rest.map(cap).join(' ') : '') + window;
  }
  const parts = s.split('-');
  const name = cap(parts.shift());
  const nums = [];
  while (parts.length && /^\d+$/.test(parts[0])) nums.push(parts.shift());
  return [name, nums.join('.'), ...parts.map(cap)].filter(Boolean).join(' ') + window;
}
const cap = (w) => (w ? w.charAt(0).toUpperCase() + w.slice(1) : '');

/** Copy text; resolves to whether it worked. */
export async function copyText(text) {
  try { await navigator.clipboard.writeText(text); return true; } catch { return false; }
}
