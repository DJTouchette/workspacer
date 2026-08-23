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
