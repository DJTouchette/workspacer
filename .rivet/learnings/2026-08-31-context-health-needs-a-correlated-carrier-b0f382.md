---
title: Context health needs a correlated carrier
date: 2026-08-31
confidence: high
suggested_doc: session-lifecycle
related_paths:
  - services/claudemon/src/providers/**
  - services/claudemon/src/session/**
  - apps/desktop/src/main/services/thresholdWatch.ts
  - services/hub/cmd/brain/agentops.go
promoted: false
---

# Context health needs a correlated carrier

## Observation
UsageAcc statusLine may fill contextWindowSize from a model table, and Codex rollout token_count may have cumulative totals without last_token_usage. A notify_when context health predicate cannot infer trust from the ordinary display pair; it needs a separate same-update active numerator/runtime denominator carrier with provider, epoch, and observation time.

## Impact
Reusing display context fields lets requested/catalog windows or cumulative throughput fire a false health wake after compaction, resume, or a provider switch.

## Recommendation
Emit context_health only for correlated runtime pairs; keep it absent for table fallbacks and cumulative-only events, stamp it at the session ownership boundary, and make evaluators fail closed on provenance/freshness/inconsistency.
