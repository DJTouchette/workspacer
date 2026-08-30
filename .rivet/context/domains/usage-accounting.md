---
title: Usage Accounting: Twin Pricing Tables, Subagent Transcripts & Rate-Limit Windows
tags: [usage, pricing, cost, rate-limits, cross-language, twin-structure, subagents]
related_paths:
  - "apps/desktop/src/main/services/modelUsage.ts"
  - "apps/desktop/src/main/services/analyticsBackfill.ts"
  - "apps/desktop/src/main/services/sessionStore/usageAccumulator.ts"
  - "apps/desktop/src/main/services/sessionStore/analyticsWriter.ts"
  - "services/claudemon/src/session/usage.rs"
  - "services/claudemon/src/session/pricing.rs"
  - "services/claudemon/src/session/state.rs"
owner: Damien Touchette
last_reviewed: 2026-07-11
---

# Usage Accounting: Twin Pricing Tables, Subagent Transcripts & Rate-Limit Windows

## Overview
Token/cost accounting is implemented independently in the Electron main process (`modelUsage.ts` + accumulator/writer), the Rust claudemon daemon (`usage.rs` + `pricing.rs`), and the wks-tui crate's own usage module — `usage.rs`'s header states it is "ported verbatim" from wks-tui's copy and mirrors `modelUsage.ts`. Every implementation folds a Claude Code transcript's per-turn `usage` block (input/output/cache tokens + model id) into cumulative cost, a point-in-time context-window gauge, and a per-model split. Separately, rate-limit window state (5h/7d/monthly) is sourced only from Claude's `statusLine`/stream `rate_limit_event`, never hooks or transcripts.

## Key modules
- `apps/desktop/src/main/services/modelUsage.ts` — Electron pricing table (`MODEL_RATES`, longest-prefix match), `turnCostUSD`, `contextTokensOf`, `contextLimitFor`, `emptyUsage`. No override-file support.
- `apps/desktop/src/main/services/sessionStore/usageAccumulator.ts` — `SessionUsageAccumulator.applyUsage`, live per-session fold, dedup by message id, per-model `slice` split, persists newly-seen model ids to `config.claude.seenModels`.
- `apps/desktop/src/main/services/analyticsBackfill.ts` — one-shot re-derivation of historical `session_history` rows from raw transcripts (`foldTranscriptFile`, `recomputeSession`), marker-guarded via the `_backfills` table (`BACKFILL_NAME = 'transcript-usage-v2'`); also folds `subagents/*.jsonl` with `forceSidechain=true`.
- `apps/desktop/src/main/services/sessionStore/analyticsWriter.ts` — `writeHistory` snapshots session usage into `sessionHistory.record` + `sessionHistory.recordModels` (the `session_model_usage` split table).
- `services/claudemon/src/session/usage.rs` — Rust mirror: `Usage`, `rates_for`, `from_transcript` (dedup + sidechain handling identical to the TS accumulator), `usage_for_path` (walks `<stem>/subagents/*.jsonl`).
- `services/claudemon/src/session/pricing.rs` — shared cross-provider pricing table: `BUILTIN` prefix list (Claude + OpenAI/Codex models), mtime-cached `~/.workspacer/model-rates.json` overrides, `rates_for`, `estimate_cost` (used by managed-provider status lines that report tokens but no dollars, e.g. Codex).
- `services/claudemon/src/session/state.rs` — `StatusLine` struct + `StatusLine::from_claude_json`, the sole source of `five_hour_pct`/`seven_day_pct`/`monthly_pct` + `*_resets_at`.
- `services/claudemon/src/providers/claude_stream.rs` — `rate_limit_event` handling: buckets `rateLimitType` (`five_hour | seven_day* | overage`) into five-hour/seven-day/monthly fields.

## Failure modes
- Unknown/new model ids silently fall back to Sonnet-tier defaults (`DEFAULT_RATES`/`Rates::default` = $3/$15, 200k) in `modelUsage.ts` and `usage.rs`; `pricing.rs`'s `rates_for`/`estimate_cost` instead return `None` for managed-provider cost estimation ("an invented rate would be worse than a blank readout") — the two failure philosophies differ between Claude-transcript costing and cross-provider cost estimation.
- `model-rates.json` at `~/.workspacer/model-rates.json` is read (mtime-cached) only by claudemon's `pricing.rs`; `modelUsage.ts` has no equivalent override mechanism — user rate overrides apply to claudemon-sourced numbers (Codex estimate, daemon-derived costs) but not to the desktop app's own transcript-derived costs.
- Cost/token dedup is by assistant message `id`; a `null`/missing id causes re-counting on transcript replay in `analyticsBackfill.ts`'s `foldTranscriptFile` (falls back to `row.uuid`), a documented edge the historical backfill (`transcript-usage-v2`) exists to correct.
- `analyticsBackfill` v1→v2 history: v1 mis-priced dated `claude-opus-4-2…` ids at the generic Opus rate and left stale `session_model_usage` rows for model keys no longer produced, double-counting `summary()` UNIONs — fixed by clearing `session_model_usage` per session before re-recording (`clearModels`/`DELETE FROM session_model_usage WHERE session_id=?`).
- Rate-limit windows only populate from the `statusLine` command / stream `rate_limit_event`; `rate_limits` is entirely absent on non-Pro/Max accounts and before the first API response, so all `*_pct`/`*_resets_at` fields stay `None` — callers must treat absence as "unknown," not "0%".

## Gotchas
- **Triple-mirrored pricing tables must be edited in lockstep**: `MODEL_RATES` in `modelUsage.ts`, `BUILTIN` in `pricing.rs`, and wks-tui's own copy. **KNOWN DIVERGENCE**: `pricing.rs` line 57 has `"claude-opus-4-1"` (without trailing dash) while `modelUsage.ts` line 67 correctly has `'claude-opus-4-1-'` (with trailing dash). Editing rates in one place silently diverges GUI vs daemon vs TUI cost figures with no compile-time or runtime check tying them together — this inconsistency is already present and exemplifies the danger.
- Longest-prefix matching is intentional and order-sensitive: e.g. `claude-opus-4-1-` (with trailing dash) must not be swallowed by the shorter `claude-opus` prefix, and dated ids like `claude-opus-4-20250514` need the separate `claude-opus-4-20` key since they don't match the `claude-opus-4-0` alias — see the long comment block in `modelUsage.ts` lines 48-62, duplicated conceptually in `pricing.rs`. **The missing trailing dash in `pricing.rs` means the daemon will mismatch Opus 4.1-dated model ids against a shorter prefix than the Electron app would use.**
- Cache pricing constants are fixed multipliers baked into code, not table-driven, in both TS and the Claude branch of Rust: cache-write = `input * 1.25`, cache-read = `input * 0.1` (default) — but `pricing.rs`'s generic (multi-provider) path instead supports a per-model `cached_input` override (used for OpenAI/Codex, whose wire reports an explicit cached rate), so Claude and Codex costing diverge in mechanism even within the single Rust codebase.
- Context-window default is 200k tokens (`contextLimit`/`context_limit`); both implementations infer a 1M-mode promotion heuristically — once any turn's observed context exceeds 200k, the *session's high-water mark* (not the current turn) pins the limit at 1,000,000, since the transcript `model` id carries no `[1m]` suffix.
- Sidechain (subagent) turns count toward cumulative cost/tokens and the per-model split at their own model's rates, but must never move the main-thread context gauge or `model` field — enforced identically by `if (!sidechain)` in `usageAccumulator.ts`/`analyticsBackfill.ts` and `if !sidechain` in `usage.rs`; subagent transcripts live at `<transcript-stem>/subagents/*.jsonl` and are walked separately (`usage_for_path`, `subagentFilesFor`).
- `overage` rate-limit events (monthly window) previously misfiled into the 5h gauge; `claude_stream.rs`'s `rate_limit_event` handler now explicitly buckets by `rateLimitType` (`is_overage` check) with regression tests (`rate_limit_event_buckets_by_window_type`) pinning that `overage` never lands in `five_hour_*`.
- Every provider adapter (`claude_stream.rs`, `codex.rs`, `codex_rollout.rs`, `opencode.rs`) surfaces its own cost/usage fields into this same pipeline via `state.rs`'s `StatusLine`/session usage structs — a new adapter that gets cost wiring wrong corrupts `session_history`/`session_model_usage` analytics silently, since there is no cross-check against the transcript-derived numbers.

## Hand-authored notes (2026-08-24/28) — cache multipliers, derived tokens, and what is actually available at boot

- **Cache-write pricing is TTL-DEPENDENT and both engines hardcoded the cheap
  rate.** Claude bills a prompt-cache WRITE at a multiple of the base input rate
  chosen by the write's lifetime: **1.25x at the 5-minute TTL, 2x at the 1-hour
  TTL** (reads are 0.1x). `modelUsage.ts` `turnCostUSD` and
  `services/claudemon/src/session/usage.rs` `turn_cost_usd` both hardcoded 1.25x
  — the 5-minute rate — while this project's sessions are almost entirely
  1-hour. The per-turn TTL split has always been on disk:
  `usage.cache_creation.{ephemeral_5m_input_tokens, ephemeral_1h_input_tokens}`.
  Displayed cost understated 1-hour writes by 1.6x (a 1M-token opus write showed
  $6.25 where $10.00 was due). **Never hardcode a cache multiplier.** Read the
  TTL split; when a turn reports writes with no `cache_creation` block, price at
  the 1-hour (dearer) rate, because assuming the cheaper one reads as a lower
  bill than the account will see. The two engines are now pinned to each other by
  the `cacheMultiplierCases` block in `contracts/model-pricing-cases.json`, read
  by `modelPricingContract.test.ts` and
  `usage.rs::matches_shared_cache_multiplier_contract`.
  *Correction to the earlier "third copy" belief:* `session/pricing.rs`'s
  `estimate_cost` has NO cache-write concept at all. It is the cross-provider
  path, enabled only via `UsageAcc::estimate_costs()` (set by `codex.rs` and
  `codex_rollout.rs` only), and OpenAI does not bill cache writes; it prices
  cache READS at the per-model `cached_input` rate with a 0.1x fallback, which is
  correct. *Widening trap:* there are **FIVE** type mirrors, not four — the Rust
  `Usage` struct, `enrich.go`, main-process `SessionUsage`
  (`modelUsage.ts` + `ipcTypes.ts`), the renderer `SessionUsage`, and the TUI
  `Usage` — plus the statusLine's own extra pair
  (`claudemonStatusLineBridge.ts`'s mapping and `SessionStatusLine`, declared in
  BOTH `claudeSessionStore.ts` and `renderer/types/claudeSession.ts`). Note serde
  `rename_all=camelCase` produces `expectedUsd`, not `expectedUSD`, so that field
  needs an explicit rename.
- **A wrong context WINDOW produces absurd TOKEN counts, because managed sessions
  derive tokens as pct × window.** Managed (non-Claude) sessions have no direct
  context-token count: `contextTokensFromStatusLine` (`claudeSessionStore.ts`)
  computes `contextUsedPct / 100 * contextWindowSize`, feeding both the bus row
  (`hubCapabilities.ts` `agents.list`) and `session.peakContext`. So (1) any bug
  that inflates the WINDOW inflates the reported TOKENS by the same factor — the
  `claude_stream.rs` `.max()` across `modelUsage` entries (a 1M sub-agent
  inflating a 200k parent) showed up as "crazy token numbers", not only as a wrong
  meter; and (2) `context_used_pct` was read UNCLAMPED off the provider payload in
  `StatusLine::from_claude_json` while `UsageAcc::status_line` already clamped the
  percentage it COMPUTES. Both are now clamped, at the parser and again at the
  consumer — keep both: the parser one covers the four clients that render
  `context_used_pct` directly (TUI, /m, remote.html, desktop), the consumer one
  covers producers other than Claude's statusLine. **When a session reports an
  impossible token count, check the WINDOW first**, and check whether that session
  is managed (usage-less) so its tokens are derived rather than counted. A bad
  percentage also poisons `peakContext`, which drives the drift alarm that
  disarms a claimed window — one wrong reading becomes a permanently hidden meter.
- **Codex's cached-token count reaches the cost estimate correctly but never
  reaches the UI.** The wire reports it in two shapes — legacy flat
  `usage.cached_input_tokens` and modern
  `tokenUsage.total/last.cachedInputTokens` (camelCase) — and both `codex.rs` and
  `codex_rollout.rs` parse it into `AgentUpdate::Usage.cached_input_tokens`,
  `UsageAcc::merge` folds it into `cached_input`, and `pricing.rs::estimate_cost`
  bills that subset at the model's discounted `cached_input` rate (gpt-5-codex:
  0.125 vs 1.25 per M, a 10x discount). **So Codex cost is NOT systematically
  overbilled.** But `state.rs`'s `StatusLine` — the thing actually projected to
  desktop/TUI — has no `cached_input` field, and `UsageAcc::status_line` consumes
  `self.cached_input` only to compute `cost_usd`, then discards it. "Can
  workspacer show cache hit/miss for a Codex session" is therefore a small,
  well-scoped gap (add the field to `StatusLine` and populate it in
  `status_line()`), not a missing upstream signal.

### What is available at boot, and what only looks like it should be

- **Account rate-limit windows are blank at boot for ONE reason: a poller gate.**
  `fetch_account_usage()` (`services/claudemon/src/session/account_usage.rs`) has
  NO session dependency — it reads `<root>/.credentials.json` and GETs
  `https://api.anthropic.com/api/oauth/usage`, and `GET /usage`
  (`daemon/api.rs`) already serves it on demand with zero sessions running. The
  gauges are blank because `spawn_poller` iterates
  `store.live_claude_config_roots()`, which filters
  `provider == "claude" && mode != Stopped` — zero live sessions, empty vec, loop
  body never runs. Separately, EVERY renderer usage surface (`UsageDetailDialog`,
  OverviewPane's `RateLimitCard`, `sessionStats.ts`) renders from a session
  snapshot's `statusLine`, so there is no session-free surface to hang a reading
  on. **For Claude this is a wiring change, not an architecture change.**
- **Distinguish the two halves of "usage".** Cumulative cost/tokens is ALREADY
  boot-available (`list_sessions`/`get_session` fold it from the transcript via
  `usage::usage_for_session`, for stopped and archived rows too, plus
  `workspacer.db`); account rate-limit WINDOWS are the half gated above.
- **Codex and Copilot both persist usage on disk** — which contradicts the
  assumption encoded in `keepWarmService.ts` that "no sessionless usage query
  exists" for Codex. True for a network query, false for on-disk state.
  Verified against live artifacts 2026-08-28:
  - **Codex:** every rollout at `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`
    carries `event_msg` payloads of type `token_count` whose `rate_limits` holds
    `primary` (used_percent, window_minutes 300, resets_at), `secondary`
    (window_minutes 10080), `credits` and `plan_type` — the 5h/weekly gauges are
    on disk, readable at boot by tailing the newest rollout for the last
    `token_count`. `~/.codex/state_5.sqlite` `threads.tokens_used` holds a
    cumulative per-thread count but with no model split and no cost.
    `~/.codex/auth.json` carries `account_id`, a real attribution key.
  - **Copilot (the richest of the three):** `~/.copilot/session-store.db` has a
    full `assistant_usage_events` table (session_id, turn_index, agent_id, model,
    input/output/cache_read/cache_write/reasoning tokens, total_nano_aiu,
    request_multiplier, api_endpoint, finish_reason, token_details_json) — and
    `token_details_json` embeds **Copilot's OWN price table** (costPerBatch in
    nano-AIU per token type), so cost need not be estimated from our table at all.
    But it has NO account/user column; `~/.copilot/config.json` lists
    `loggedInUsers` and `lastLoggedInUser`, so mapping rows to a login is a guess.
    Copilot quota specifically has no queryable API (`copilot_internal/v2/token`
    403s to a `gh` OAuth token).
  - **Claude:** `~/.claude/stats-cache.json` holds `modelUsage` per model +
    `dailyModelTokens` + `dailyActivity`, but it is LAZILY recomputed by the CLI —
    `lastComputedDate` was 2026-08-09 while sessions ran through 2026-08-28 (19
    days stale) and every `costUSD` field is 0. **Never use it for anything
    time-sensitive.**
  Boot-time preference: Claude = OAuth `/api/oauth/usage` (exact); Codex = newest
  rollout's last `token_count.rate_limits` (exact but as-of-last-turn); Copilot =
  `assistant_usage_events` aggregation (exact tokens/AIU, no quota headroom, no
  account attribution).

### Two traps in the desktop usage SURFACES

- **`InspectorCard`'s Usage tab short-circuits on `!sl && !usage`** (no live
  statusLine, no transcript usage) — and a cold start has NEITHER by definition:
  a restored agent's session is a stopped daemon row that
  `promoteSessionSnapshots` drops, so there is no snapshot at all. **Any figure
  sourced from the history DB rather than a live snapshot was therefore
  unreachable in exactly the case it exists for** — the tab rendered "No usage
  data yet" over a store holding the numbers. Any future "fill this from the
  record" work must widen the guard, not just add tiles below it. When adding a
  cold-start fallback to any usage surface, check the section's own empty-state
  guard first; several gate on live snapshot fields.
- **`session_history.cost_usd / input_tokens / output_tokens` are `DEFAULT 0` and
  never NULL**, so a row created and never written to is indistinguishable from
  one measured at zero. Verified against the live store 2026-08-28: 754 rows,
  $14,968.38, 17.58B tokens — and 239 rows (31.7%) all-zero. Every desktop read
  path therefore reports a stored 0 as UNDEFINED (`recentSessions.ts`'s
  `recorded()`, `useSessionAnalytics`'s `recorded()`) and surfaces render a dash
  rather than "$0.00". **Consumers must never `?? 0` these.** Treat unknown /
  unavailable / zero as three distinct states and never collapse them —
  `useSessionAnalytics` and `RecordedUsageContext` (`absentUsageTitle`) already
  carry the reason strings for the "could not read" case. An all-zero payload is a
  routine shape here, not an error, so "$0.00 across 0 sessions" is the default
  failure mode beside a five-figure database. (`analytics:summary`/`analytics:recent`
  were wired end to end with ZERO callers after the analytics pane was deleted;
  `useSessionAnalytics` is now the only consumer, and the headless brain answers
  both with a well-formed all-zero stub carrying `unavailable: "headless"` — the
  same field main sets when its SQLite read throws, so one check covers both.)

### Per-profile attribution: one exact path, two guesses

Verified on a live machine 2026-08-28.

1. **EXACT.** `fetch_account_usage(client, root)` already takes a config root and
   reads `<root>/.credentials.json`; the profile dir
   `~/.claude/accounts/work/.credentials.json` exists with its OWN
   `subscriptionType` ("team", rateLimitTier default_claude_max_5x) distinct from
   the default root's ("max", default_claude_max_20x). Per-profile account windows
   are genuinely fetchable, not inferred.
2. **BUT idle profiles degrade to unknown.** The Work profile's OAuth token was
   EXPIRED (expiresAt 8 days stale); `token_from_credentials` bails locally on
   expiry and never refreshes (rotation is the CLI's job), so a boot-time fetch
   for any profile you have not used recently silently returns nothing — and idle
   profiles are exactly the ones a boot readout is for.
3. **GUESS territory.** `~/.claude/accounts/work/projects` is a SYMLINK to the
   shared `~/.claude/projects`, so both logins' transcripts land in one physical
   directory. `claudeAccountOf()` (`renderer/src/lib/claudeAccount.ts`) and
   `root_from_transcript()` only work because the CLI's path STRING retains the
   profile root — **never canonicalize/realpath a Claude transcript path before
   deriving the account.** And `workspacer.db`'s
   `session_history`/`session_model_usage` have no profile/account/transcript
   column at all, so the ~750 existing history rows cannot be retroactively
   attributed by any means. If per-profile history is wanted, add an account
   column going forward and leave old rows unattributed rather than backfilling a
   guess.
