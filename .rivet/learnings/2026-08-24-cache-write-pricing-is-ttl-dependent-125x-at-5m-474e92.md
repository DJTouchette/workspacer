---
title: Cache-write pricing is TTL-dependent (1.25x at 5m, 2x at 1h), and only two engines price it
date: 2026-08-24
confidence: high
related_paths:
  - apps/desktop/src/main/services/modelUsage.ts
  - services/claudemon/src/session/usage.rs
  - services/claudemon/src/session/pricing.rs
  - contracts/model-pricing-cases.json
promoted: false
---

# Cache-write pricing is TTL-dependent (1.25x at 5m, 2x at 1h), and only two engines price it

## Observation
Claude bills a prompt-cache WRITE at a multiple of the base input rate chosen by the write's lifetime: 1.25x at the 5-minute TTL and 2x at the 1-hour TTL. Reads are 0.1x. Both costing engines (modelUsage.ts turnCostUSD, claudemon session/usage.rs turn_cost_usd) hardcoded 1.25x, which is the 5-minute rate, while this project's sessions are almost entirely 1-hour. The per-turn TTL split has always been on disk: usage.cache_creation.{ephemeral_5m_input_tokens, ephemeral_1h_input_tokens}. Contrary to the scout brief, there is NOT a third copy of the cache-write multiplier: session/pricing.rs::estimate_cost has no cache-write concept at all. It is the cross-provider path, enabled only via UsageAcc::estimate_costs() which is set by codex.rs and codex_rollout.rs only, and OpenAI does not bill cache writes. It prices cache READS at the per-model cached_input rate with a 0.1x fallback, which is correct.</observation>
<parameter name="impact">Displayed cost understated 1-hour cache writes: a 1M-token opus write showed $6.25 where $10.00 was due. There were also FIVE type mirrors to widen, not four: the Rust Usage struct, enrich.go, main-process SessionUsage (modelUsage.ts + ipcTypes.ts), the renderer SessionUsage, and the TUI Usage. The statusLine has its own extra mirror pair (claudemonStatusLineBridge.ts mapping + SessionStatusLine declared in BOTH claudeSessionStore.ts and renderer/types/claudeSession.ts).</observation>
<parameter name="recommendation">Never hardcode a cache multiplier. Read the TTL split; when a turn reports writes with no cache_creation block, price at the 1-hour (dearer) rate, because assuming the cheaper one reads as a lower bill than the account will see. The two engines are now pinned to each other by the cacheMultiplierCases block in contracts/model-pricing-cases.json, read by modelPricingContract.test.ts and usage.rs::matches_shared_cache_multiplier_contract. Note serde rename_all=camelCase produces expectedUsd, not expectedUSD, so that field needs an explicit rename.</recommendation>
<parameter name="suggested_doc">usage-accounting
