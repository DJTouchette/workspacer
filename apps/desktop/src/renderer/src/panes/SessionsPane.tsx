import React, { useMemo, useState } from 'react';
import type { RecentAgentSession } from '../../../main/shared/ipcTypes';
import type { AgentProvider } from '../types/pane';
import { recentSessionLabel } from '../lib/recentSessionFilter';
import { fuzzyScoreAny } from '../lib/fuzzy';
import { History, X } from '../components/icons';

/**
 * Session history browser — every resumable daemon session that has no card in
 * the current layout, as a searchable list. This replaced the sidebar's
 * EARLIER/RECENT dock: the sidebar stays pure live triage and points here via
 * its History footer row (also reachable from the command palette). Clicking a
 * row respawns the session as an agent (`--resume`); the row then leaves this
 * list because the layout now represents it.
 */

/** Same provider hues the sidebar cards use. */
const PROVIDER_HUE: Record<AgentProvider, string> = {
  claude: '#e67e80',
  codex: '#7fbbb3',
  opencode: '#d699b6',
  pi: '#83c092',
};

/** Compact relative age: 45s → "45s", then 2m / 3h / 2d. */
function relTime(ms: number): string {
  const s = Math.max(0, Math.round(ms / 1000));
  if (s < 60) return `${s}s`;
  const m = Math.round(s / 60);
  if (m < 60) return `${m}m`;
  const h = Math.round(m / 60);
  if (h < 24) return `${h}h`;
  return `${Math.round(h / 24)}d`;
}

interface SessionsPaneProps {
  /** Resumable daemon sessions (already filtered against the live layout). */
  sessions: RecentAgentSession[];
  /** Bring a session back as an agent (spawn with --resume). */
  onResume?: (session: RecentAgentSession) => void;
}

const SessionsPane: React.FC<SessionsPaneProps> = ({ sessions, onResume }) => {
  const [query, setQuery] = useState('');

  const rows = useMemo(() => {
    const sorted = [...sessions].sort((a, b) => (b.updatedAt || 0) - (a.updatedAt || 0));
    const q = query.trim();
    if (!q) return sorted;
    return sorted
      .map((s) => ({
        s,
        score: fuzzyScoreAny(q, [
          recentSessionLabel(s),
          s.title,
          s.name,
          s.cwd,
          s.model,
          s.provider,
        ]),
      }))
      .filter((x) => x.score > -Infinity)
      .sort((a, b) => b.score - a.score)
      .map((x) => x.s);
  }, [sessions, query]);

  return (
    <div
      style={{
        height: '100%',
        overflowY: 'auto',
        backgroundColor: 'var(--wks-bg-base)',
        color: 'var(--wks-text-secondary)',
        fontFamily: '"Hanken Grotesk", Inter, system-ui, sans-serif',
      }}
    >
      <div style={{ maxWidth: 720, margin: '0 auto', padding: '28px 24px 40px' }}>
        {/* Header */}
        <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
          <span
            style={{
              display: 'flex',
              alignItems: 'center',
              color: 'var(--wks-text-muted)',
            }}
          >
            <History size={18} strokeWidth={1.75} />
          </span>
          <span
            style={{
              fontSize: '1.05rem',
              fontWeight: 650,
              letterSpacing: '-0.01em',
              color: 'var(--wks-text-primary)',
            }}
          >
            Sessions
          </span>
          <span style={{ fontSize: '0.72rem', color: 'var(--wks-text-faint)' }}>
            {sessions.length} resumable
          </span>
        </div>
        <div style={{ marginTop: 4, fontSize: '0.72rem', color: 'var(--wks-text-muted)' }}>
          Past conversations with no card in your workspace. Click one to bring it back as an agent.
        </div>

        {/* Search */}
        <div
          style={{
            display: 'flex',
            alignItems: 'center',
            gap: 8,
            marginTop: 16,
            padding: '0 12px',
            background: 'var(--wks-bg-raised)',
            border: '1px solid var(--wks-border-subtle)',
            borderRadius: 'var(--wks-radius-pill)',
          }}
        >
          <span aria-hidden style={{ color: 'var(--wks-text-faint)', lineHeight: 1 }}>
            ⌕
          </span>
          <input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Search by name, directory, model…"
            spellCheck={false}
            style={{
              flex: 1,
              height: 32,
              background: 'transparent',
              border: 'none',
              outline: 'none',
              fontSize: '0.8rem',
              fontFamily: 'inherit',
              color: 'var(--wks-text-primary)',
            }}
          />
          {query && (
            <button
              onClick={() => setQuery('')}
              title="Clear"
              style={{
                background: 'none',
                border: 'none',
                cursor: 'pointer',
                padding: 0,
                display: 'flex',
                color: 'var(--wks-text-faint)',
              }}
            >
              <X size={12} strokeWidth={2} />
            </button>
          )}
        </div>

        {/* Rows */}
        <div style={{ marginTop: 14 }}>
          {rows.length === 0 && (
            <div
              style={{
                padding: '40px 0',
                textAlign: 'center',
                fontSize: '0.8rem',
                color: 'var(--wks-text-faint)',
              }}
            >
              {query
                ? `No sessions match “${query.trim()}”`
                : 'No past sessions — everything is already in your workspace.'}
            </div>
          )}
          {rows.map((s) => {
            const provider = (s.provider || 'claude') as AgentProvider;
            const hue = PROVIDER_HUE[provider] ?? 'var(--wks-accent)';
            const label = recentSessionLabel(s);
            const age = s.updatedAt ? relTime(Date.now() - s.updatedAt) : '';
            return (
              <div
                key={s.sessionId}
                role="button"
                tabIndex={0}
                onClick={() => onResume?.(s)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter') onResume?.(s);
                }}
                title={`Resume as an agent\n${s.cwd}${s.model ? `\n${s.model}` : ''}`}
                style={{
                  display: 'flex',
                  alignItems: 'center',
                  gap: 10,
                  padding: '10px 12px',
                  marginBottom: 6,
                  borderRadius: 'var(--wks-radius-md)',
                  border: '1px solid var(--wks-border-subtle)',
                  cursor: onResume ? 'pointer' : 'default',
                  opacity: s.archived ? 0.6 : 1,
                  transition: 'background 0.12s, border-color 0.12s',
                }}
                onMouseEnter={(e) => {
                  e.currentTarget.style.background = 'var(--wks-bg-hover)';
                }}
                onMouseLeave={(e) => {
                  e.currentTarget.style.background = 'transparent';
                }}
              >
                <span
                  aria-hidden
                  style={{
                    flexShrink: 0,
                    width: 8,
                    height: 8,
                    borderRadius: '50%',
                    background: hue,
                    opacity: 0.85,
                  }}
                />
                <span style={{ flex: 1, minWidth: 0 }}>
                  <span
                    style={{
                      display: 'block',
                      fontSize: '0.8rem',
                      fontWeight: 500,
                      color: 'var(--wks-text-primary)',
                      overflow: 'hidden',
                      textOverflow: 'ellipsis',
                      whiteSpace: 'nowrap',
                    }}
                  >
                    {label}
                  </span>
                  <span
                    style={{
                      display: 'block',
                      fontSize: '0.66rem',
                      fontFamily: 'var(--wks-font-mono)',
                      color: 'var(--wks-text-faint)',
                      overflow: 'hidden',
                      textOverflow: 'ellipsis',
                      whiteSpace: 'nowrap',
                    }}
                  >
                    {s.cwd}
                    {s.model ? ` · ${s.model}` : ''}
                  </span>
                </span>
                {s.archived && (
                  <span
                    style={{
                      flexShrink: 0,
                      fontSize: '0.6rem',
                      fontWeight: 600,
                      textTransform: 'uppercase',
                      letterSpacing: '0.06em',
                      padding: '2px 8px',
                      borderRadius: 'var(--wks-radius-pill)',
                      color: 'var(--wks-text-faint)',
                      border: '1px solid var(--wks-border-subtle)',
                    }}
                  >
                    Archived
                  </span>
                )}
                {age && (
                  <span
                    style={{
                      flexShrink: 0,
                      fontSize: '0.66rem',
                      fontFamily: 'var(--wks-font-mono)',
                      color: 'var(--wks-text-faint)',
                    }}
                  >
                    {age}
                  </span>
                )}
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
};

export default SessionsPane;
