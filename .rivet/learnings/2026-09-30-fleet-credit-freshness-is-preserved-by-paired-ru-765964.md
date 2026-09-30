---
title: Fleet credit freshness is preserved by paired Rust status projections
date: 2026-09-30
confidence: high
suggested_doc: session-lifecycle
related_paths:
  - services/hub-rs/src/services/live_streams.rs
  - services/hub-rs/src/services/thresholds.rs
  - services/hub-rs/src/services/library_watch.rs
promoted: false
---

# Fleet credit freshness is preserved by paired Rust status projections

## Observation
Legacy fleetview.go must prefer raw status_line because Go incremental updates leave statusLine stale. Rust live_streams::prepare updates status_line and snapshots::compat-derived statusLine together under the snapshot write lock, before sidebar visibility filtering. Therefore wakes reading the compat credit flag is not evidence of stale telemetry. Thresholds separately preserve raw counter precedence and raw-context presence blocks stale health fallback. Live directory consumers recompute from current snapshots; the removed cwd cache is not an ambient-path authorization boundary.

## Recommendation
Trace the actual projection writer before inferring stale-data bugs from different accessor spellings; retain full internal fleet lookup independently of sidebar filtering.
