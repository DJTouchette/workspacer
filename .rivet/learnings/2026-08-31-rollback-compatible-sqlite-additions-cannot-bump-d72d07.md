---
title: Rollback-compatible SQLite additions cannot bump claudemon user_version
date: 2026-08-31
confidence: high
suggested_doc: claudemon-sqlite-store
related_paths:
  - services/claudemon/src/store/schema.rs
  - services/claudemon/src/store/mod.rs
promoted: false
---

# Rollback-compatible SQLite additions cannot bump claudemon user_version

## Observation
claudemon schema v8 has a downgrade guard that refuses any database whose PRAGMA user_version is greater than 8 before considering whether changes are additive. A normal v9 bump for nullable canonical model-selection columns would therefore break the slice requirement that the prior daemon can reopen the database and read requested_model.

## Impact
Additive columns intended for a rolling rollback must be catalog-checked on every open while retaining user_version 8, or the prior binary cannot open the DB at all. Dual-writing requested_model alone is insufficient if the schema stamp blocks startup.

## Recommendation
For rollback-compatible nullable columns during the v8 support window, use an idempotent pragma_table_info migration that runs on every open and leave user_version at 8; add a test that opens twice and reads requested_model through the prior schema projection.
