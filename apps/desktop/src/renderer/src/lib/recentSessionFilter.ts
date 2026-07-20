/**
 * Which of the daemon's sessions belong in the sidebar's "Recent" list: rows
 * the current layout does NOT already represent. An agent card (live or
 * stopped-with-respawn) is the affordance for its own session — repeating the
 * session below it would offer two ways to resume the same conversation, and
 * resuming a session that's live under another client would double-drive it,
 * so non-stopped rows are excluded too.
 */
import type { RecentAgentSession } from '../../../main/shared/ipcTypes';
import type { AgentWorkspace } from '../types/pane';

/**
 * Display label for a RECENT row: an explicitly-given agent name wins, then
 * the provider's auto-generated conversation title, then the directory name.
 * A recorded name that just equals the cwd basename is the spawn-time default
 * (deriveAgentName), not the user naming it — the title is more telling.
 */
export function recentSessionLabel(s: RecentAgentSession): string {
  const dirName = s.cwd.split('/').filter(Boolean).pop() || '';
  const explicitName = s.name && s.name !== dirName ? s.name : '';
  return explicitName || s.title || s.name || dirName || s.sessionId.slice(0, 8);
}

export function filterResumableSessions(
  sessions: RecentAgentSession[],
  agents: AgentWorkspace[],
  ptyMapping: Record<string, string>,
  limit = 20,
): RecentAgentSession[] {
  const inLayout = new Set<string>(Object.values(ptyMapping));
  for (const a of agents) {
    if (a.sessionId) inLayout.add(a.sessionId);
    if (a.lastSessionId) inLayout.add(a.lastSessionId);
    for (const t of a.tabs) {
      for (const p of t.panes) {
        if (p.resumeSessionId) inLayout.add(p.resumeSessionId);
        if (p.attachSessionId) inLayout.add(p.attachSessionId);
      }
    }
  }
  return sessions
    .filter((s) => s.mode === 'stopped' && s.cwd && !inLayout.has(s.sessionId))
    .slice(0, limit);
}
