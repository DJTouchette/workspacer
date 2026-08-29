---
title: InspectorCard's Usage tab gated on a LIVE snapshot, so recorded figures were unreachable
date: 2026-08-28
confidence: high
suggested_doc: usage-accounting
promoted: false
---

# InspectorCard's Usage tab gated on a LIVE snapshot, so recorded figures were unreachable

## Observation
Two related traps in the desktop usage-accounting surfaces:

1. InspectorCard's Usage tab short-circuits on `!sl && !usage` (no live statusLine, no transcript usage). A cold start has NEITHER by definition — a restored agent's session is a stopped daemon row that promoteSessionSnapshots drops, so there is no snapshot at all. Any figure sourced from the history DB rather than a live snapshot was therefore unreachable in exactly the case it exists for: the tab rendered "No usage data yet" over a store holding the numbers. Any future "fill this from the record" work on that tab must widen the guard, not just add tiles below it.

2. `session_history.cost_usd / input_tokens / output_tokens` are `DEFAULT 0` and never NULL, so a row created and never written to is indistinguishable from one measured at zero. Verified against the live store on 2026-08-28: 754 rows, $14,968.38, 17.58B tokens, and 239 rows (31.7%) all-zero. Every read path in the desktop therefore reports a stored 0 as UNDEFINED (recentSessions.ts `recorded()`, useSessionAnalytics `recorded()`), and surfaces render a dash rather than "$0.00". Consumers must never `?? 0` these.

Also: `analytics:summary` / `analytics:recent` were wired end to end (main ipc.ts handler, preload, webBackend passthrough, electron.d.ts) with ZERO callers after the analytics pane was deleted; useSessionAnalytics is now the only consumer. The headless brain answers both with a well-formed all-zero stub carrying `unavailable: "headless"` — the same field main sets when its SQLite read throws — so one check covers both.</observation>
<parameter name="impact">A surface can look correctly wired and be structurally unreachable; and an all-zero payload is a routine shape here, not an error, so "$0.00 across 0 sessions" is the default failure mode beside a five-figure database.

## Recommendation
When adding a cold-start/recorded fallback to any usage surface, check the section's own empty-state guard first — several gate on live snapshot fields. Treat unknown / unavailable / zero as three distinct states and never collapse them; useSessionAnalytics and RecordedUsageContext (absentUsageTitle) already carry the reason strings for the "could not read" case.</recommendation>
<parameter name="related_paths">["apps/desktop/src/renderer/src/components/claude/InspectorCard.tsx", "apps/desktop/src/renderer/src/hooks/useSessionAnalytics.ts", "apps/desktop/src/renderer/src/contexts/RecordedUsageContext.tsx", "apps/desktop/src/main/services/sessionHistory.ts", "apps/desktop/src/main/services/recentSessions.ts"]
