/**
 * The transcript-backed session history, per project: for each known project
 * directory, the sessions `claude --resume` would offer there — read by main
 * from ~/.claude/projects/<encoded>/*.jsonl (claudeListSessionsForDir), newest
 * first, capped at 20 per directory by the lister.
 *
 * Fetched when the consumer mounts (the Sessions pane opens on click, so
 * mount time IS the freshness point) rather than polled — the underlying
 * reads are mtime-cached in main, but there's no reason to touch dozens of
 * directories once a minute for a pane that isn't open. Web parity comes for
 * free: the polyfill answers claudeListSessionsForDir via the brain's
 * `claude.sessionsForDir`.
 */
import { useEffect, useState } from 'react';

export interface ProjectTranscriptSession {
  sessionId: string;
  /** ISO timestamp — the transcript's own, else its file mtime. */
  timestamp: string;
  /** First user message or Claude's summary line — what `--resume` shows. */
  summary: string;
}

export interface ProjectSessionsResult {
  /** Keyed by the exact dir string passed in (not projectKey-normalized). */
  byDir: Record<string, ProjectTranscriptSession[]>;
  loading: boolean;
}

export function useProjectSessions(dirs: string[]): ProjectSessionsResult {
  const [byDir, setByDir] = useState<Record<string, ProjectTranscriptSession[]>>({});
  const [loading, setLoading] = useState(dirs.length > 0);

  // Keyed by content, not array identity — callers rebuild the list per render.
  const dirsKey = dirs.join('\n');

  useEffect(() => {
    const list = dirsKey ? dirsKey.split('\n') : [];
    if (list.length === 0) {
      setByDir({});
      setLoading(false);
      return;
    }
    let alive = true;
    setLoading(true);
    // One IPC per project; main bounds the per-call file IO and caches
    // headers by (mtime, size), so the fan-out is cheap after the first open.
    Promise.all(
      list.map(async (dir) => {
        try {
          // Optional-chained: absent on older preloads.
          const rows = await window.electronAPI.claudeListSessionsForDir?.(dir);
          return [dir, rows ?? []] as const;
        } catch {
          return [dir, []] as const;
        }
      }),
    ).then((entries) => {
      if (!alive) return;
      setByDir(Object.fromEntries(entries));
      setLoading(false);
    });
    return () => {
      alive = false;
    };
  }, [dirsKey]);

  return { byDir, loading };
}
