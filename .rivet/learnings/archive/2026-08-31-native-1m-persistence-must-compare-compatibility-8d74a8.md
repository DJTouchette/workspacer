---
title: Native-1M persistence must compare compatibility projections
date: 2026-08-31
confidence: high
suggested_doc: session-lifecycle
related_paths:
  - services/claudemon/src/session/windows.rs
  - services/claudemon/src/store/mod.rs
  - services/claudemon/src/session/state.rs
promoted: true
promoted_to: claudemon-sqlite-store
---

# Native-1M persistence must compare compatibility projections

## Observation
Claudemon now dual-writes canonical requested_model_identity/requested_context_window and legacy requested_model without changing SQLite user_version. The pending e8349a8d fixes an edge where native-1M Fable/Mythos canonical bare projection and old marked input looked different, causing restore to prefer legacy evidence and lose the canonical 1M selection. The canonical pair remains serde-skipped from public snapshots, so transport consumers cannot yet depend on it.

## Impact
A rollback-compatible persistence reader can silently regress native-1M sessions if it compares raw normalized pairs instead of their legacy projections; public snapshot consumers still lack owner-authored selection provenance.

## Recommendation
Land e8349a8d before building outward transports, and add the pair only additively to snapshot/wire contracts.

## Disposition
Promoted into `.rivet/context/modules/claudemon-sqlite-store.md` WITH TWO CORRECTIONS: the "pending" `e8349a8d` has landed, so the legacy-projection comparison rule is recorded as shipped behaviour; and the canonical pair is NO LONGER serde-skipped — since `66c842df` `requested_selection`/`resolved_context_window` are published on the snapshot, so transport consumers can and do depend on them.
