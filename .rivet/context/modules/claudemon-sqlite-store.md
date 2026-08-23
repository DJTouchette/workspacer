---
title: claudemon SQLite Persistence + Boot Hydration
tags: [claudemon, rust, sqlite, rusqlite, persistence, hydration]
related_paths:
  - "services/claudemon/src/store/mod.rs"
  - "services/claudemon/src/store/schema.rs"
  - "services/claudemon/src/daemon/mod.rs"
  - "services/claudemon/src/session/store.rs"
  - "services/claudemon/src/session/state.rs"
owner: Damien Touchette
last_reviewed: 2026-07-11
---

# claudemon SQLite Persistence + Boot Hydration

## Overview
claudemon keeps a hot in-memory `SessionStore` (`services/claudemon/src/session/store.rs`) for latency-sensitive PTY/mode state, and a cold SQLite store (`services/claudemon/src/store/mod.rs`) that durably records only the hook-event stream and a derived sessions row. On boot the daemon loads the most recent SQLite rows and rehydrates the in-memory list as `Stopped`/resumable sessions, so agents survive a daemon restart even though their live PTY state does not.

## Key modules
- `services/claudemon/src/store/mod.rs` — `Db` handle (`Arc<Mutex<Connection>>`), `record_event` (upsert session + insert event in one transaction), `load_recent_sessions`, `default_db_path`.
- `services/claudemon/src/store/schema.rs` — `migrate()` (user_version-gated, one-shot), `SCHEMA_V1` DDL for `sessions`/`events` + `events_session_time` index.
- `services/claudemon/src/daemon/mod.rs` — wires it all up: opens `Db`, calls `load_recent_sessions(SESSION_HYDRATE_LIMIT)` (= 100) at startup, calls `store.hydrate(sessions)`, and spawns `spawn_persistence_task` subscribed to `store.subscribe_hooks()`.
- `services/claudemon/src/session/store.rs` — `SessionStore::hydrate` (line ~411) turns each `RestoredSession` into a `SessionState::new(...)` forced to `SessionMode::Stopped`, restoring `tool_calls`, `started_at`, `updated_at`; only fills a slot if not already present (`entry.or_insert_with`), so a live session at boot always wins over a hydrated row.
- `services/claudemon/src/session/state.rs` — `SessionState::is_archived` (line 525): archived purely as a display filter once `Stopped` and idle past `ARCHIVE_AFTER_SECONDS`; the SQLite row is never deleted and the session stays resumable.

## Failure modes
- `record_event` runs inside a single `rusqlite::Transaction`: `upsert_session_tx` then `insert_event_tx`, committed together, so an `events` row can never reference a session that failed to upsert (mod.rs:54-61).
- Persistence is out-of-band via `tokio::sync::broadcast`: `spawn_persistence_task` (daemon/mod.rs:370) reads `store.subscribe_hooks()` and does the write on `tokio::task::spawn_blocking`, logging (`tracing::warn`) and swallowing errors on write failure rather than crashing the daemon — a failed persist never blocks the hook response.
- If the persistence task falls behind the broadcast channel it hits `RecvError::Lagged(n)` and logs `skipped = n` — those hook events are silently dropped from SQLite (never retried) while the in-memory store still applied them normally.
- On `RecvError::Closed` the task just logs and exits; no restart/backoff.
- `load_recent_sessions` failing at boot (`Err(err)`) only logs a `tracing::warn`; the daemon still starts with an empty session list rather than failing to boot (daemon/mod.rs:64-72).
- The mutex in `Db` is `.lock().expect("db mutex poisoned")` — a panic while holding the lock (e.g. mid-transaction) poisons it and takes down every subsequent DB call for the process lifetime.

## Gotchas
- **Durable vs ephemeral boundary**: only what `record_event` writes — the `sessions` upsert columns and one `events` row per `HookEvent` — survives a restart. PTY bytes, live `SessionMode`, transcript tailer state, conversation store, and all other rich in-memory `SessionState` fields are gone on restart and rebuilt from scratch (or not at all) via hydration.
- **Upsert is column-preserving, not overwriting**: on conflict, `cwd` only updates `CASE WHEN sessions.cwd = '' THEN excluded.cwd ELSE sessions.cwd END` (keeps first non-empty value), and `model`/`branch` use `COALESCE(sessions.model, excluded.model)` (keep first non-null). Later events with better/different values for these fields will never overwrite an already-set one — only `last_event_at` and (via a separate `UPDATE`) `tool_call_count` unconditionally advance.
- Only `PreToolUse` bumps `tool_call_count` (mod.rs:144-149) — other tool-related events do not increment it, so tool-count parity with the in-memory `SessionState.tool_calls` field depends entirely on that one event type being reliably delivered.
- `name`/`project` are derived once at first insert (`derive_name_from_cwd`, or `payload["name"]`/`payload["project"]` if present) and are **not** part of the `ON CONFLICT` update clause at all — they're frozen at session creation even if the payload later carries a better name.
- `migrate()` is one-shot forward-only: it checks `user_version >= USER_VERSION` and no-ops if so; there is no down-migration or multi-step migration chain. Any schema change requires bumping `USER_VERSION` in `services/claudemon/src/store/schema.rs` and adding a new `SCHEMA_V2`-style batch — editing `SCHEMA_V1` in place will not re-run against existing databases.
- Pragmas set at `Db::open` (WAL journal mode, `synchronous = NORMAL`, `foreign_keys = ON`) are connection-level and re-applied every open; `foreign_keys = ON` makes the `events.session_id` FK to `sessions(id)` actually enforced, reinforcing why the upsert-then-insert ordering inside one transaction matters.
- `default_db_path()` honors `XDG_DATA_HOME` first (`$XDG_DATA_HOME/claudemon/state.db`), else `~/.claudemon/state.db` via `directories::BaseDirs`, else falls back to relative `.claudemon/state.db` if `BaseDirs::new()` fails — three different possible locations depending on environment.
- Hydration is capped at `SESSION_HYDRATE_LIMIT = 100` (daemon/mod.rs:20), newest `last_event_at` first; sessions beyond that limit remain in SQLite but won't reappear in the in-memory list until pruning/archival logic or a manual query surfaces them.
- All `Db` writes serialize through one `Mutex<Connection>` — there's a single connection, not a pool, so heavy concurrent hook traffic serializes on this lock (acceptable per the module's own doc comment, which asserts SQLite itself is the bottleneck, not the mutex).
