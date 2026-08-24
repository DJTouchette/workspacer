---
title: History pane titles stop at 40 rows
date: 2026-08-17
promoted: true
---

# History pane titles stop at 40 rows

## Observation
The Sessions pane ('History') is uncapped (App.tsx passes Infinity to filterResumableSessions) but recentSessions.ts only title-enriches the first 40 daemon rows (TITLE_ENRICH_LIMIT) — rows beyond 40 fall back to name/dirname labels. Also: 'History' UI = daemon GET /sessions joined with session_history SQLite (names/cost only); session_history itself is the ANALYTICS store, not the resumable list — daemon rows are the sole truth for resumability.

## Disposition
Merged with e228b5 into .rivet/context/domains/session-lifecycle.md (History pane is transcript-backed; daemon rows remain the resumability truth, 2026-08-17).
