---
title: Account rate-limit windows are unavailable at boot solely because the poller is gated on live_claude_config_roots()
date: 2026-08-28
confidence: high
suggested_doc: usage-accounting
related_paths:
  - services/claudemon/src/session/account_usage.rs
  - services/claudemon/src/session/store.rs
  - services/claudemon/src/daemon/api.rs
  - apps/desktop/src/renderer/src/panes/OverviewPane.tsx
promoted: false
---

# Account rate-limit windows are unavailable at boot solely because the poller is gated on live_claude_config_roots()

## Observation
claudemon's `fetch_account_usage()` (services/claudemon/src/session/account_usage.rs) has NO session dependency — it reads `<root>/.credentials.json` and GETs https://api.anthropic.com/api/oauth/usage. `GET /usage` (daemon/api.rs:328-371) already serves it on demand with zero sessions running. The reason gauges are blank at boot is one line: `spawn_poller` iterates `store.live_claude_config_roots()` (session/store.rs:913), which filters `provider == "claude" && mode != SessionMode::Stopped`. Zero live sessions -> empty vec -> loop body never runs. Separately, EVERY renderer usage surface (UsageDetailDialog, OverviewPane's RateLimitCard, sessionStats.ts) renders from a session snapshot's `statusLine`, so there is no session-free usage surface to hang a reading on even when the daemon has one. Note per-session cumulative token/cost usage IS already boot-available: list_sessions/get_session fold it from the transcript via `usage::usage_for_session` for stopped and archived rows too.

## Impact
Anyone asked to "make usage available at boot" will assume a new data source is needed. For Claude it is a wiring change (call the existing ungated endpoint at startup + add a sessionless surface), not an architecture change.

## Recommendation
Distinguish the two halves of "usage": cumulative cost/tokens (already boot-available from transcripts + workspacer.db) vs account rate-limit windows (blank at boot due to the poller gate).
