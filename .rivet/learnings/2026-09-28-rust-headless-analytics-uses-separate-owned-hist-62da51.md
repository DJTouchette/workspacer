---
title: Rust headless analytics uses separate owned history and source failures stay visible
date: 2026-09-28
confidence: high
suggested_doc: usage-accounting
related_paths:
  - services/hub-rs/src/services/analytics.rs
  - contracts/analytics-history-cases.json
promoted: false
---

# Rust headless analytics uses separate owned history and source failures stay visible

## Observation
The Node headless analytics path observes only during analytics queries, persists the shared desktop session_history and session_model_usage schema, fingerprints main/subagent transcript inputs, and retains recorded totals after transcripts disappear. Rust now ports those semantics to its own headless-analytics.sqlite, reads daemon snapshots solely through EmbeddedClient, and adds a coalesced owned observer to retain terminal sessions before projection eviction. Rates reuse claudemon builtin pricing through a read-only accessor.

## Recommendation
Run the analytics-history shared fixture through both Node companion and Rust, plus model-pricing/cache TTL corpus. Never open claudemon state.db from analytics; failed engine/history reads must reject instead of fabricate zero totals. Persisted zero legacy totals remain explicitly counted as unrecordedSessions.
