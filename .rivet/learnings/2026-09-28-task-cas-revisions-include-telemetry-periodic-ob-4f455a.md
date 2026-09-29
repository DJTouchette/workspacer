---
title: Task CAS revisions include telemetry; periodic observers must deduplicate source readings
date: 2026-09-28
suggested_doc: fleet-manager
related_paths:
  - services/hub-rs/src/services/task_store/admission.rs
  - apps/desktop/src/main/services/dispatchHistoryStore.ts
promoted: false
---

# Task CAS revisions include telemetry; periodic observers must deduplicate source readings

## Observation
dispatchHistoryStore.ts compares the entire prior task JSON with the mutated task before incrementing revision, so telemetry fields observedAt/wallMs do advance the CAS revision. Rust TaskStore preserves this meaningful source-mutation behavior but deduplicates identical host snapshots before observe_batch writes, preventing a periodic observer from creating synthetic revisions every tick. Only observations for admitted attempts enter the dedup cache, so a pre-admission snapshot cannot suppress the first real observation.

## Impact
An unconditional Rust snapshot timer would continuously invalidate workflow and Task Inspector expectedTaskRevision even when no provider evidence changed.

## Recommendation
Feed real host snapshot changes or use TaskStore.observe_batch dedup; do not exclude genuine telemetry mutations from revision accounting without an explicit cross-language contract change.
