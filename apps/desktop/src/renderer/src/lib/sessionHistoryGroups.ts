/**
 * Builds the Sessions pane's project-grouped history from its two sources:
 *
 * - Per-project transcript listings (what `claude --resume` offers in that
 *   directory). These are the primary content, present even for sessions the
 *   daemon has long forgotten.
 * - The daemon's rows: permanent history (`sessions.history`,
 *   docs/session-roster.md §5) when the backend serves it, otherwise the
 *   bounded `sessions.recent` list. This is the only source for managed
 *   providers (codex/opencode/pi), for claude rows past the lister's per-dir
 *   cap, and for everything in directories that aren't registered projects.
 *
 * A session both sources know is ONE row: the transcript's summary is the
 * label (unless the user explicitly named the agent), the daemon contributes
 * model/archived/hidden, and resuming goes through the daemon row so a
 * recorded stream transport or model survives.
 */
import type { HistoryAgentSession, RecentAgentSession } from '../../../main/shared/ipcTypes';
import type { ProjectTranscriptSession } from '../hooks/useProjectSessions';
import { recentSessionLabel } from './recentSessionFilter';
import { basenameOf } from './projectIdentity';
import { projectKey } from './projectKey';

/** One resumable row, whichever source(s) know about it. */
export interface HistoryRow {
  sessionId: string;
  label: string;
  provider: string;
  cwd: string;
  /** Unix ms; 0 when neither source carried a parseable timestamp. */
  updatedAt: number;
  archived: boolean;
  model: string;
  /** Cost recorded for this session, or UNDEFINED when nothing was recorded.
   *  Never 0 — see RecentAgentSession.costUSD for why a stored zero counts as
   *  an absence here. Transcript-only rows (no daemon row) always have none. */
  costUSD?: number;
  /** Cumulative billed tokens recorded for this session; same absence rule. */
  billedTokens?: number;
  /** The daemon's row, when it has one — resumed as-is to keep transport/model. */
  daemon?: RecentAgentSession;
  /** Hidden by the user (shared archive): off every sidebar, still in History. */
  hidden?: boolean;
  /** Listed on the shared sidebar (`sessionOpen`). */
  onSidebar?: boolean;
  /** Not resumable from here: the layout already holds it or it is live
   *  (only set when the caller keeps such rows; see `keepExcluded`). */
  blocked?: boolean;
  /** Permanent history knows this session, so "Delete from history" applies. */
  inHistory?: boolean;
}

/** A project section, or the trailing catch-all (dir '') for daemon rows in
 *  directories that aren't registered projects. */
export interface HistoryGroup {
  key: string;
  dir: string;
  rows: HistoryRow[];
}

export const OTHER_GROUP_KEY = '::other';

function isHistoryRow(d: RecentAgentSession): d is HistoryAgentSession {
  return typeof (d as Partial<HistoryAgentSession>).hidden === 'boolean';
}

/** The History-only facts a daemon row carries (none for a recent row). */
function historyFacts(
  d: RecentAgentSession | undefined,
): Pick<HistoryRow, 'hidden' | 'onSidebar' | 'inHistory'> {
  if (!d || !isHistoryRow(d)) return {};
  return { hidden: d.hidden, onSidebar: d.onSidebar, inHistory: true };
}

export function buildHistoryGroups(
  projectDirs: string[],
  transcriptsByDir: Record<string, ProjectTranscriptSession[]>,
  daemonSessions: RecentAgentSession[],
  excludeSessionIds?: string[],
  /** Keep excluded rows (marked `blocked`) instead of dropping them. Permanent
   *  history shows EVERY session; one the layout holds or that is live is
   *  listed but not resumable from here. */
  opts?: { keepExcluded?: boolean },
): HistoryGroup[] {
  const excluded = new Set(excludeSessionIds ?? []);
  const keep = opts?.keepExcluded === true;
  /** Drop an excluded row, or keep it blocked; null = drop. */
  const gate = (sessionId: string): { blocked?: boolean } | null =>
    excluded.has(sessionId) ? (keep ? { blocked: true } : null) : {};
  const daemonByDir = new Map<string, RecentAgentSession[]>();
  const daemonById = new Map(daemonSessions.map((s) => [s.sessionId, s]));
  for (const s of daemonSessions) {
    const k = s.cwd ? projectKey(s.cwd) : '';
    const list = daemonByDir.get(k);
    if (list) list.push(s);
    else daemonByDir.set(k, [s]);
  }
  const fromDaemon = (d: RecentAgentSession, blocked: { blocked?: boolean }): HistoryRow => ({
    sessionId: d.sessionId,
    label: recentSessionLabel(d),
    provider: d.provider || 'claude',
    cwd: d.cwd,
    updatedAt: d.updatedAt,
    archived: d.archived,
    model: d.model,
    costUSD: d.costUSD,
    billedTokens: d.billedTokens,
    daemon: d,
    ...historyFacts(d),
    ...blocked,
  });

  const out: HistoryGroup[] = [];
  const knownKeys = new Set<string>();
  for (const dir of projectDirs) {
    const k = projectKey(dir);
    knownKeys.add(k);
    const dirName = basenameOf(dir);
    const seen = new Set<string>();
    const rows: HistoryRow[] = [];
    for (const t of transcriptsByDir[dir] ?? []) {
      const blocked = gate(t.sessionId);
      if (!blocked) continue;
      seen.add(t.sessionId);
      const d = daemonById.get(t.sessionId);
      // An explicit agent name beats the transcript summary; a name that just
      // equals the dir basename is the spawn-time default, not a choice.
      const explicitName = d?.name && d.name !== dirName ? d.name : '';
      rows.push({
        sessionId: t.sessionId,
        label: explicitName || t.summary || d?.title || t.sessionId.slice(0, 8),
        provider: d?.provider || 'claude',
        cwd: dir,
        updatedAt: Math.max(Date.parse(t.timestamp) || 0, d?.updatedAt ?? 0),
        archived: d?.archived ?? false,
        model: d?.model || '',
        costUSD: d?.costUSD,
        billedTokens: d?.billedTokens,
        daemon: d,
        ...historyFacts(d),
        ...blocked,
      });
    }
    // Daemon-only rows for this project: managed providers (no Claude
    // transcript exists), plus claude rows older than the lister's per-dir
    // cap that the daemon still holds.
    for (const d of daemonByDir.get(k) ?? []) {
      if (seen.has(d.sessionId)) continue;
      const blocked = gate(d.sessionId);
      if (!blocked) continue;
      rows.push(fromDaemon(d, blocked));
    }
    rows.sort((a, b) => b.updatedAt - a.updatedAt);
    if (rows.length > 0) out.push({ key: k, dir, rows });
  }

  // Sessions from directories that aren't registered projects — daemon rows
  // only (there's no project to scan transcripts for). Kept so nothing that
  // was resumable before the project grouping becomes invisible.
  const other: HistoryRow[] = [];
  for (const [k, list] of daemonByDir) {
    if (knownKeys.has(k)) continue;
    for (const d of list) {
      const blocked = gate(d.sessionId);
      if (!blocked) continue;
      other.push(fromDaemon(d, blocked));
    }
  }
  other.sort((a, b) => b.updatedAt - a.updatedAt);
  if (other.length > 0) out.push({ key: OTHER_GROUP_KEY, dir: '', rows: other });
  return out;
}

/** What resuming a transcript-only row hands to the spawn path: the same wire
 *  shape a daemon row has, with the daemon-only facts at their zero values.
 *  transport 'pty' reads as "no recorded choice" there, so the config default
 *  decides — same as a legacy daemon row. */
export function syntheticDaemonRow(row: HistoryRow): RecentAgentSession {
  return {
    sessionId: row.sessionId,
    provider: 'claude',
    cwd: row.cwd,
    mode: 'stopped',
    transport: 'pty',
    archived: row.archived,
    updatedAt: row.updatedAt,
    startedAt: 0,
    name: '',
    title: row.label,
    model: '',
    // Deliberately absent, not 0: this row exists because a transcript does,
    // and the history DB was never asked about it. A 0 would render as a
    // measured "$0.00" on every surface that takes this shape.
    costUSD: undefined,
    billedTokens: undefined,
  };
}
