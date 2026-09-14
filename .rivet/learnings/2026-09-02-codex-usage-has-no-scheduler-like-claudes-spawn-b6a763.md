---
title: Codex usage has no scheduler like Claude's spawn_poller — it's a per-request disk re-read
date: 2026-09-02
confidence: high
suggested_doc: usage-accounting
related_paths:
  - services/claudemon/src/providers/codex_usage.rs
  - services/claudemon/src/session/usage_report.rs
  - services/claudemon/src/session/account_usage.rs
  - services/claudemon/src/daemon/api.rs
promoted: false
---

# Codex usage has no scheduler like Claude's spawn_poller — it's a per-request disk re-read

## Observation
Unlike Claude's account_usage::spawn_poller (services/claudemon/src/session/account_usage.rs, registered in daemon/mod.rs:106), there is no scheduled background poller for Codex. codex_usage::read_from_disk() (services/claudemon/src/providers/codex_usage.rs) is called synchronously and freshly on every single GET /usage/report request (usage_report.rs:603), tail-scanning up to 8 newest $CODEX_HOME/sessions/**/rollout-*.jsonl files for the last token_count.rate_limits event. There is no SessionStore cache for it. This means /usage/report is already correct on a cold daemon with zero sessions for Codex (proved by daemon/api.rs's usage_report_answers_a_cold_daemon_for_every_provider test) — a "poll Codex on boot" feature has nothing to schedule at that layer. What IS stale on a cold/stopped daemon is a session ROW's cached status_line field (GET /sessions), which freezes at the last live wire rate_limit_event and is never refreshed once the session stops — a completely separate, uncached data path from /usage/report.

## Impact
Anyone asked to "poll Codex usage on boot" will look for a scheduler to add and find none needed at the /usage/report layer — the actual gap is client-side (RateLimitCard/mobile/TUI reading only live session status lines, never /usage/report) not server-side.

## Recommendation
Before adding Codex boot-polling infrastructure, check whether the real ask is (a) make /usage/report boot-accurate (already true) or (b) make some session-status-line-only client surface show a number with zero sessions (the actual gap, needing GET /usage/report wired into that client, per keepWarmService.ts's precedent).
