---
title: Codex and Copilot both persist usage on disk at boot; Copilot's is the richest of all three providers
date: 2026-08-28
confidence: high
suggested_doc: usage-accounting
related_paths:
  - services/claudemon/src/providers/copilot.rs
  - services/claudemon/src/providers/codex_rollout.rs
  - apps/desktop/src/main/services/keepWarmService.ts
promoted: true
promoted_to: usage-accounting
---

# Codex and Copilot both persist usage on disk at boot; Copilot's is the richest of all three providers

## Observation
Verified against live artifacts on 2026-08-28. CODEX: every rollout at ~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl carries `event_msg` payloads of type `token_count` whose `rate_limits` object holds `primary` (used_percent, window_minutes 300, resets_at), `secondary` (window_minutes 10080), `credits`, and `plan_type` — i.e. the 5h/weekly gauges are on disk, readable at boot by tailing the newest rollout for the last token_count. Also ~/.codex/state_5.sqlite `threads.tokens_used` holds a cumulative per-thread token count (54 threads, 222.6M tokens) but with no model split and no cost. Codex auth.json carries `account_id` — a real attribution key. COPILOT: ~/.copilot/session-store.db has a full `assistant_usage_events` table (session_id, turn_index, agent_id, model, input/output/cache_read/cache_write/reasoning tokens, total_nano_aiu, request_multiplier, api_endpoint, finish_reason, token_details_json) — and token_details_json embeds Copilot's OWN price table (costPerBatch in nano-AIU per token type), so cost need not be estimated from our table at all. But it has NO account/user column; ~/.copilot/config.json lists `loggedInUsers` (two GitHub logins here) and `lastLoggedInUser`, so mapping usage rows to a login would be a guess. CLAUDE: ~/.claude/stats-cache.json holds `modelUsage` per model + `dailyModelTokens` + `dailyActivity`, but it is LAZILY recomputed by the CLI — lastComputedDate was 2026-08-09 while sessions ran through 2026-08-28 (19 days stale) and every costUSD field is 0. Do not treat it as authoritative.

## Impact
Contradicts the assumption (encoded in keepWarmService.ts:26) that "no sessionless usage query exists" for Codex — true for a network query, false for on-disk state. Copilot quota specifically has no queryable API: copilot.rs:128 records that copilot_internal/v2/token 403s to a gh OAuth token.

## Recommendation
For boot-time usage prefer: Claude = OAuth /api/oauth/usage (exact); Codex = newest rollout's last token_count.rate_limits (exact but as-of-last-turn); Copilot = assistant_usage_events aggregation (exact tokens/AIU, no quota headroom, no account attribution). Never use stats-cache.json for anything time-sensitive.
