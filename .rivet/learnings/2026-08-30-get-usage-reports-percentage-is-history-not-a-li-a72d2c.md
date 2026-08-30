---
title: GET /usage/report's percentage is history, not a live window — decide currency from resets_at
date: 2026-08-30
confidence: high
suggested_doc: usage-accounting
related_paths:
  - apps/desktop/src/main/services/keepWarmLogic.ts
  - apps/desktop/src/main/services/keepWarmService.ts
  - services/claudemon/src/session/usage_report.rs
  - services/claudemon/src/providers/codex_usage.rs
  - services/hub/internal/capspec/claudemoncallers_test.go
promoted: false
---

# GET /usage/report's percentage is history, not a live window — decide currency from resets_at

## Observation
claudemon's `GET /usage/report` returns Codex's 5h window as `used_percent: {state:'ok', value:67}` with a `resets_at` that is TWO DAYS IN THE PAST and `is_current:false` (probed live 2026-08-30 against the running daemon on :7891). The percentage is the last figure the Codex CLI wrote to its rollout — real history, and a false present. Anything that treats a non-zero percentage as "a window is running" will be wrong. keep-warm's `windowActive()` does exactly that (`five_hour_pct > 0` → active), so piping the report's percentage straight into it would suppress keep-warm's Codex ping permanently. `fiveHourWindowFromReport()` in keepWarmLogic.ts therefore carries ONLY `resets_at` across, and decides currency from `resets_at` vs now rather than from the report's own `is_current` (which was computed at `generated_at` and is the staler of the two). It returns three distinct answers: `{five_hour_resets_at}` = running, `{}` = definitely lapsed, `null` = unreadable — and the middle one is the whole value of reading the report, because "definitely lapsed" is an answer no live-status-line source can give on a cold start.

## Impact
This is /usage/report's FIRST CLIENT (the route shipped in 0.160.0 with no caller in any commit). Any future consumer — the pending RateLimitCard fallback especially — hits the same trap: a card that renders `used_percent` without checking `resets_at`/`is_current` shows a 67% gauge for a window that closed two days ago. Also: closing a capspec orphan means DELETING its `claudemonRouteCallers` row, not just adding the caller; `TestEveryClaudemonRouteHasACallerOrADeclaredReason` fails a stale declaration (verified by re-adding the row and watching it go red).

## Recommendation
Never render or branch on the report's `used_percent` without also reading that window's `resets_at`. `/usage` and `/usage/report` are NOT interchangeable: `/usage` is the DEFAULT Claude login only and will make a blocking network fetch when its cache is stale; `/usage/report` is every provider and every account, always immediate, never fetching. keep-warm's Claude path stays on `/usage` for both reasons.
