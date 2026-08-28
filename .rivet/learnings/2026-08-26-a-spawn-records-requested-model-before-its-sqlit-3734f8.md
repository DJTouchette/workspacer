---
title: A spawn records requested_model before its SQLite row exists, so the write has to ride the row-creating INSERT
date: 2026-08-26
confidence: high
suggested_doc: claudemon-sqlite-store
related_paths:
  - services/claudemon/src/store/mod.rs
  - services/claudemon/src/store/schema.rs
  - services/claudemon/src/session/store.rs
  - services/claudemon/src/daemon/mod.rs
  - services/claudemon/src/daemon/spawn.rs
promoted: false
---

# A spawn records requested_model before its SQLite row exists, so the write has to ride the row-creating INSERT

## Observation
`SessionStore::set_requested_model` is called from the spawn handlers, but a session's `sessions` row is not created until its FIRST HOOK EVENT arrives (`upsert_session_tx`, driven by the persistence task off `store.subscribe_hooks()`). So a spawn-time `UPDATE sessions SET requested_model = ... WHERE id = ?` matches zero rows, and the INSERT that follows would leave the column NULL. `SessionStore` also holds no `Db` handle at all — persistence is deliberately out-of-band via the hook broadcast — so there is no in-store write path either. The working shape is to thread the value INTO the row-creating statement (`Db::record_event_with_requested_model`, read from `SessionStore::requested_model` in the persistence task) with `COALESCE(sessions.requested_model, excluded.requested_model)` on conflict, exactly how the neighbouring `model` column is handled. The spawn-time UPDATE is still worth keeping for a session whose row already exists (a resume).

## Impact
Any future "remember X about a session across a daemon restart" field hits the same ordering hole, and the failure is silent: the column is simply always NULL and only shows up as wrong behaviour after a restart. It cost a debugging round here on `requested_model`, which is the only carrier of a `[1m]` window choice (Claude Code strips the marker from the transcript's model id).

## Recommendation
For new persisted per-session fields, add them to `RestoredSession` + `hydrate` + the `upsert_session_tx` INSERT/COALESCE in one go, and put the value on the persistence task's path rather than writing it at spawn. Migrations that ADD COLUMN must use a catalog-checked step body (see `add_heartbeat_provider` / `add_session_requested_model`), never a bare SQL const.
