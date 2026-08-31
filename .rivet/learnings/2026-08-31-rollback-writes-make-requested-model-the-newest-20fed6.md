---
title: Rollback writes make requested_model the newest selection evidence
date: 2026-08-31
confidence: high
suggested_doc: claudemon-sqlite-store
related_paths:
  - services/claudemon/src/session/windows.rs
  - services/claudemon/src/session/store.rs
  - services/claudemon/src/store/mod.rs
  - services/claudemon/src/store/schema.rs
promoted: false
---

# Rollback writes make requested_model the newest selection evidence

## Observation
During the v8 rollback window, a prior daemon can update only sessions.requested_model. If that normalized value disagrees with otherwise-valid canonical selection columns, the legacy value is newer evidence; restoring canonical data would undo the old daemon's live model switch. The next new-daemon event must derive both legacy and canonical fields from the restored selection. Separately, catalog-checking unversioned ADD COLUMN steps is not concurrency-safe unless the check and ALTER share a BEGIN IMMEDIATE transaction.

## Impact
A rollback-era model switch can be silently reverted after reopening, raw legacy syntax can preserve permanent column divergence, and concurrent daemon starts can fail boot with duplicate-column errors.

## Recommendation
On restore, keep matching canonical pairs authoritative but prefer a valid disagreeing requested_model; never overwrite a derived legacy projection with raw restored text; run unversioned catalog checks under a write transaction; preserve both additive columns in future table rebuilds and treat USER_VERSION > 8 as explicitly ending v8 rollback support.
