---
title: v8 model restoration has a single persistence boundary
date: 2026-08-31
confidence: high
suggested_doc: claudemon-sqlite-store
related_paths:
  - services/claudemon/src/session/windows.rs
  - services/claudemon/src/store/mod.rs
  - services/claudemon/src/store/schema.rs
promoted: false
---

# v8 model restoration has a single persistence boundary

## Observation
restore_persisted_model_selection is only called by services/claudemon/src/store/mod.rs during load/heal, so native-1M canonical/legacy reconciliation can be fixed without changing public session shapes or provider APIs.

## Impact
Keeps the rollback-window persistence repair bounded to SQLite hydration and healing.

## Recommendation
Add round-trip coverage at the store boundary when changing reconciliation semantics.
