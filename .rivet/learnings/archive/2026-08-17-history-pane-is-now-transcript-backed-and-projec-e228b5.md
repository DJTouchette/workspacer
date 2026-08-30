---
title: History pane is now transcript-backed and project-grouped
date: 2026-08-17
promoted: true
---

# History pane is now transcript-backed and project-grouped

## Observation
Rewrote SessionsPane 2026-08-17: content now comes from per-project claude transcript listings (claudeListSessionsForDir per registered project from lib/projectRegistry) merged with daemon rows; grouping/merge logic lives in renderer/src/lib/sessionHistoryGroups.ts (tested). Daemon rows remain the only source for managed providers and unregistered dirs (trailing 'Other directories' group). Transcript-only rows resume via a synthetic RecentAgentSession (transport 'pty' = 'no recorded choice'). App threads historyExcludeIds (layout ids + daemon non-stopped) so transcript rows can't double-drive a live session. SideBar History footer row is now ALWAYS visible (count badge removed — it counted daemon rows, which no longer match pane content); SideBar no longer takes recentSessions. This supersedes the '40-row title cap' concern for claude rows (transcripts carry their own summaries); TITLE_ENRICH_LIMIT still affects daemon-only row titles.

## Disposition
Merged with 258cee into .rivet/context/domains/session-lifecycle.md (History pane is transcript-backed; daemon rows remain the resumability truth, 2026-08-17).
