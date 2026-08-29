import React, { useMemo, useState } from 'react';
import type { RecentAgentSession } from '../../../main/shared/ipcTypes';
import type { AgentProvider } from '../types/pane';
import { useProjectSessions } from '../hooks/useProjectSessions';
import { useConfigContext } from '../contexts/ConfigContext';
import { listProjects } from '../lib/projectRegistry';
import { resolveProject } from '../lib/projectIdentity';
import {
  buildHistoryGroups,
  syntheticDaemonRow,
  type HistoryRow,
} from '../lib/sessionHistoryGroups';
import { ProjectMark } from '../components/ProjectMark';
import { fuzzyScoreAny } from '../lib/fuzzy';
import { fmtTokens, fmtUSD } from '../lib/sessionStats';
import { useSessionAnalytics } from '../hooks/useSessionAnalytics';
import { History, X } from '../components/icons';

/**
 * Session history browser, grouped by project. Each known project (the
 * registry behind the Projects settings page) gets a section listing the
 * sessions `claude --resume` would offer for that directory — read from the
 * CLI's own transcript files, so what's in here is exactly what Claude itself
 * remembers, not whichever rows the daemon happens to still hold.
 *
 * The daemon list still matters for what transcripts can't cover: managed
 * providers (codex/opencode/pi) keep their rows, merged into their project's
 * section, and sessions from directories that aren't registered projects land
 * in a trailing "Other directories" section. A daemon row for a session the
 * transcripts also list is folded into one row (the transcript's summary wins
 * as the label; the daemon contributes model/archived).
 *
 * Reached from the sidebar's History footer row or the command palette.
 * Clicking a row respawns the session as an agent (`--resume`); the row then
 * leaves this list because the layout now represents it.
 */

/** Same provider hues the sidebar cards use. */
const PROVIDER_HUE: Record<AgentProvider, string> = {
  claude: '#e67e80',
  codex: '#7fbbb3',
  copilot: '#dbbc7f',
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

/** One figure in the History header strip: dim label, bright value. */
const HistoryTotal: React.FC<{ label: string; value: string }> = ({ label, value }) => (
  <span style={{ display: 'inline-flex', alignItems: 'baseline', gap: 5 }}>
    <span
      style={{
        fontSize: '0.62rem',
        textTransform: 'uppercase',
        letterSpacing: '0.08em',
        color: 'var(--wks-text-faint)',
      }}
    >
      {label}
    </span>
    <span style={{ color: 'var(--wks-text-primary)', fontWeight: 650 }}>{value}</span>
  </span>
);

interface SessionsPaneProps {
  /** Resumable daemon sessions (already filtered against the live layout). */
  sessions: RecentAgentSession[];
  /** Why the daemon's list couldn't be read, or null when it could. Headless
   *  hubs have no `sessions.recent` provider, and answering [] made this pane
   *  state — confidently, in prose — that the user had no past sessions. */
  unavailable?: string | null;
  /** Session ids the layout holds or the daemon reports live — transcript
   *  rows for these are hidden (resuming one would double-drive it). */
  excludeSessionIds?: string[];
  /** Bring a session back as an agent (spawn with --resume). */
  onResume?: (session: RecentAgentSession) => void;
}

const SessionsPane: React.FC<SessionsPaneProps> = ({
  sessions,
  unavailable,
  excludeSessionIds,
  onResume,
}) => {
  const [query, setQuery] = useState('');
  const { config } = useConfigContext();
  // The history DB itself, not the daemon's list. It is the only source for
  // TRANSCRIPT-ONLY rows — sessions claudemon has long forgotten but that the
  // desktop still recorded cost and tokens for — and the only source of a
  // lifetime total for this pane's header.
  const analytics = useSessionAnalytics();

  const projects = useMemo(() => listProjects(config), [config]);
  const projectDirs = useMemo(() => projects.map((p) => p.dir), [projects]);
  const { byDir: transcriptsByDir, loading } = useProjectSessions(projectDirs);

  const groups = useMemo(
    () => buildHistoryGroups(projectDirs, transcriptsByDir, sessions, excludeSessionIds),
    [projectDirs, transcriptsByDir, sessions, excludeSessionIds],
  );

  /** Groups after search: rows filtered by score (best first), empty groups
   *  dropped. Without a query, groups keep their registry order (pinned
   *  first, then recency) and rows stay newest-first. */
  const visible = useMemo(() => {
    const q = query.trim();
    if (!q) return groups;
    return groups
      .map((g) => {
        const projectLabel = g.dir ? (resolveProject(g.dir, config.projects)?.label ?? '') : '';
        const rows = g.rows
          .map((r) => ({
            r,
            score: fuzzyScoreAny(q, [r.label, r.cwd, r.provider, r.model, projectLabel]),
          }))
          .filter((x) => x.score > -Infinity)
          .sort((a, b) => b.score - a.score)
          .map((x) => x.r);
        return { ...g, rows };
      })
      .filter((g) => g.rows.length > 0);
  }, [groups, query, config.projects]);

  const total = useMemo(() => groups.reduce((n, g) => n + g.rows.length, 0), [groups]);

  const resume = (row: HistoryRow) => onResume?.(row.daemon ?? syntheticDaemonRow(row));

  return (
    <div
      style={{
        height: '100%',
        overflowY: 'auto',
        backgroundColor: 'var(--wks-bg-base)',
        color: 'var(--wks-text-secondary)',
        fontFamily: 'var(--wks-font-sans)',
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
            History
          </span>
          <span style={{ fontSize: '0.72rem', color: 'var(--wks-text-faint)' }}>
            {loading ? 'loading…' : `${total} resumable`}
          </span>
        </div>
        <div style={{ marginTop: 4, fontSize: '0.72rem', color: 'var(--wks-text-muted)' }}>
          Past conversations in each of your projects. Click one to bring it back as an agent.
        </div>

        {/* What this history actually cost, straight from the desktop's own
            session-history store. Three states, kept apart: figures we have,
            rows nobody recorded a figure for (counted, not hidden), and a
            store we cannot reach (said out loud, never rendered as zeros). */}
        <div
          style={{
            display: 'flex',
            alignItems: 'baseline',
            gap: 14,
            flexWrap: 'wrap',
            marginTop: 12,
            padding: '10px 12px',
            borderRadius: 'var(--wks-radius-md)',
            border: '1px solid var(--wks-border-subtle)',
            background: 'var(--wks-bg-raised)',
            fontSize: '0.72rem',
            fontVariantNumeric: 'tabular-nums',
          }}
        >
          {analytics.loading ? (
            <span style={{ color: 'var(--wks-text-faint)' }}>Reading recorded usage…</span>
          ) : analytics.unavailable ? (
            <span style={{ color: 'var(--wks-text-faint)' }}>
              Recorded usage is unavailable here ({analytics.unavailable}) — that is what could not
              be read, not a total of zero.
            </span>
          ) : analytics.summary ? (
            <>
              <HistoryTotal label="All time" value={fmtUSD(analytics.summary.totals.costUSD)} />
              <HistoryTotal
                label="Tokens"
                value={fmtTokens(
                  analytics.summary.totals.inputTokens + analytics.summary.totals.outputTokens,
                )}
              />
              <HistoryTotal
                label="Recorded"
                value={`${analytics.summary.totals.sessions} session${
                  analytics.summary.totals.sessions === 1 ? '' : 's'
                }`}
              />
              {analytics.unrecordedSessions > 0 && (
                <span
                  title={
                    analytics.unrecordedComplete
                      ? 'These sessions were recorded, but no usage was ever written for them. They are not $0.00 — they are unmeasured.'
                      : 'Counted over the most recent rows only, so the real number is at least this. The session count beside it is the whole store.'
                  }
                  style={{ color: 'var(--wks-text-faint)' }}
                >
                  {analytics.unrecordedComplete ? '' : 'at least '}
                  {analytics.unrecordedSessions} with no usage recorded
                </span>
              )}
            </>
          ) : (
            <span style={{ color: 'var(--wks-text-faint)' }}>No usage recorded yet.</span>
          )}
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
            placeholder="Search by message, project, directory, model…"
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

        {/* Groups */}
        {visible.length === 0 && (
          <div
            style={{
              padding: '40px 0',
              textAlign: 'center',
              fontSize: '0.8rem',
              color: 'var(--wks-text-faint)',
            }}
          >
            {loading
              ? 'Reading session transcripts…'
              : query
                ? `No sessions match “${query.trim()}”`
                : unavailable
                  ? `This server can’t list past sessions (${unavailable}) — so this is what could not be read, not what isn’t there.`
                  : 'No past sessions — everything is already in your workspace.'}
          </div>
        )}
        {visible.map((g) => (
          <div key={g.key} style={{ marginTop: 22 }}>
            {/* Section header: the project's mark and name, or the catch-all. */}
            <div
              style={{
                display: 'flex',
                alignItems: 'baseline',
                gap: 8,
                padding: '0 2px 8px',
              }}
            >
              {g.dir ? (
                <>
                  <ProjectMark cwd={g.dir} projects={config.projects} size={13} withLabel />
                  <span
                    style={{
                      fontSize: '0.62rem',
                      fontFamily: 'var(--wks-font-mono)',
                      color: 'var(--wks-text-faint)',
                      overflow: 'hidden',
                      textOverflow: 'ellipsis',
                      whiteSpace: 'nowrap',
                      minWidth: 0,
                    }}
                  >
                    {g.dir}
                  </span>
                </>
              ) : (
                <span
                  style={{
                    fontSize: '0.7rem',
                    fontWeight: 600,
                    textTransform: 'uppercase',
                    letterSpacing: '0.06em',
                    color: 'var(--wks-text-muted)',
                  }}
                >
                  Other directories
                </span>
              )}
              <span
                style={{
                  marginLeft: 'auto',
                  flexShrink: 0,
                  fontSize: '0.66rem',
                  fontFamily: 'var(--wks-font-mono)',
                  color: 'var(--wks-text-faint)',
                }}
              >
                {g.rows.length}
              </span>
            </div>
            {g.rows.map((s) => {
              const hue = PROVIDER_HUE[s.provider as AgentProvider] ?? 'var(--wks-accent)';
              const age = s.updatedAt ? relTime(Date.now() - s.updatedAt) : '';
              // Inside a project section the header already names the dir;
              // the sub-line is only needed for catch-all rows and models.
              const subline = [g.dir ? '' : s.cwd, s.model].filter(Boolean).join(' · ');
              // The daemon-joined figures first (same store, already on the
              // row); the direct history read covers transcript-only rows the
              // daemon has forgotten, which is most of a long history.
              const fromHistory = analytics.bySessionId[s.sessionId];
              const cost = s.costUSD ?? fromHistory?.costUSD;
              const billed = s.billedTokens ?? fromHistory?.billedTokens;
              return (
                <div
                  key={s.sessionId}
                  role="button"
                  tabIndex={0}
                  onClick={() => resume(s)}
                  onKeyDown={(e) => {
                    if (e.key === 'Enter') resume(s);
                  }}
                  title={`Resume as an agent\n${s.cwd}${s.model ? `\n${s.model}` : ''}`}
                  style={{
                    display: 'flex',
                    alignItems: 'center',
                    gap: 10,
                    padding: subline ? '10px 12px' : '8px 12px',
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
                      {s.label}
                    </span>
                    {subline && (
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
                        {subline}
                      </span>
                    )}
                  </span>
                  {/* What this session actually cost, from the history DB's
                      join. A row with nothing recorded renders NOTHING here —
                      not "$0.00", which would claim a measurement that was
                      never taken (a third of the rows on a real machine are
                      un-costed placeholders). */}
                  {(cost !== undefined || billed !== undefined) && (
                    <span
                      title={
                        billed !== undefined
                          ? `${fmtTokens(billed)} tokens billed (cumulative)`
                          : undefined
                      }
                      style={{
                        flexShrink: 0,
                        fontSize: '0.66rem',
                        fontFamily: 'var(--wks-font-mono)',
                        color: 'var(--wks-text-muted)',
                        fontVariantNumeric: 'tabular-nums',
                      }}
                    >
                      {[
                        cost !== undefined ? fmtUSD(cost) : '',
                        billed !== undefined ? fmtTokens(billed) : '',
                      ]
                        .filter(Boolean)
                        .join(' · ')}
                    </span>
                  )}
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
        ))}
      </div>
    </div>
  );
};

export default SessionsPane;
