---
title: Codex cached-token count reaches cost estimate correctly but is never surfaced to the UI
date: 2026-08-24
confidence: high
suggested_doc: usage-accounting
related_paths:
  - services/claudemon/src/providers/codex.rs
  - services/claudemon/src/providers/codex_rollout.rs
  - services/claudemon/src/providers/mod.rs
  - services/claudemon/src/session/pricing.rs
  - services/claudemon/src/session/state.rs
promoted: false
---

# Codex cached-token count reaches cost estimate correctly but is never surfaced to the UI

## Observation
Codex's wire reports a cached-token subset of the input in two shapes: legacy flat `usage.cached_input_tokens` and modern `tokenUsage.total/last.cachedInputTokens` (camelCase). Both codex.rs (live app-server) and codex_rollout.rs (rollout-file tailer) parse it into AgentUpdate::Usage.cached_input_tokens; UsageAcc.merge folds it into UsageAcc.cached_input; pricing.rs::estimate_cost correctly bills that subset at the model's discounted cached_input rate (e.g. gpt-5-codex: 0.125 vs 1.25 per M tokens, a 10x discount) rather than the full input rate. So the codex cost estimate is NOT systematically overbilled. However state.rs's StatusLine struct — the thing actually projected to the desktop/TUI — has no cached_input field. UsageAcc::status_line consumes self.cached_input only to compute cost_usd, then discards it. The daemon has the cached-token count in hand every merge but nothing downstream can currently render "N tokens, M cached" for a Codex session.

## Impact
Anyone asked "can workspacer show cache hit/miss for a Codex session" will find the answer is "the data exists in UsageAcc but is dropped before reaching StatusLine" — a small, well-scoped gap, not a missing upstream signal.

## Recommendation
If a cached-token readout is wanted, add cached_input_tokens: Option<u64> to state.rs::StatusLine and populate it in UsageAcc::status_line() (services/claudemon/src/providers/mod.rs ~line 678) instead of only feeding it to estimate_cost.
