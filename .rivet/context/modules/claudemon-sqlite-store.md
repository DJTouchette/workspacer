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
last_reviewed: 2026-09-26
---

# Claudemon SQLite persistence and boot hydration

## Durable data versus runtime state

`services/claudemon/src/store/mod.rs` provides `Db`, a shared mutex-protected
SQLite connection. The schema stores hook events, session history/spawn facts,
keep-warm heartbeat records, and execution leases. PTY buffers, live processes,
channels, and pending runtime operations are not restored by reopening this DB.

`default_db_path` prefers XDG_DATA_HOME/claudemon/state.db, then the user’s
.claudemon/state.db, with a relative fallback if no home can be resolved. A
launcher can explicitly pass a different DB path. Do not assume a second set
of daemon ports selects a different database automatically; the serve launcher
has its own alternate-stack path checks.

`Db::open` applies WAL, synchronous NORMAL and foreign-key enforcement, then
migrations. Reads/writes through this handle share one connection mutex. A panic
poisoning that mutex makes later expect/unwrap acquisition fail; there is no
pool of unaffected connections to fall back to.

## Hook persistence and upsert semantics

The daemon subscribes to the hook broadcast and writes on the blocking pool.
`record_event_with_spawn_facts` upserts the session and inserts its event in one
transaction. The hook response does not wait for successful durable storage.
Write failures are logged; broadcast lag loses events from persistence with a
warning, and a closed channel ends the task. Do not claim every accepted hook
has reached disk or that missed persistence events are retried.

`upsert_session_tx` has field-specific merge rules:

- First nonempty cwd and first non-null reported model/branch are preserved.
- Name/project are chosen at insertion, not replaced by later hook payloads.
- Last-event time updates; PreToolUse increments the hook-derived tool counter.
- A supplied canonical requested-model identity replaces the legacy model,
  identity and context-window columns together, including a null window.
  Absent selection facts leave the existing selection untouched.
- A new non-null transcript path replaces the stored path.
- First non-null config_root sticks; empty string is the known default root,
  while null means no account attribution.

These are hook-upsert rules, not a claim that separate model-selection updates
can never change a column. `note_requested_model_selection` updates the
selection for an already-existing row after appropriate control acceptance.

## The row-creation ordering rule

A spawn-time in-memory value can precede the first hook that creates its session
row. An UPDATE at that point may match nothing. The persistence task therefore
reads `SpawnFacts` from the session store and carries requested selection and
config-root attribution into the row-creating INSERT. Keep RestoredSession,
hydration and the INSERT/update rules aligned when adding another durable fact.

Execution leases use their own table precisely because native admission can
precede hook-created history. Lease claims compare the expected prior lease
inside an immediate transaction and increment the generation. Do not fold that
identity into a build stamp or substitute an in-memory spawn generation for the
durable compatibility pin.

Transcript paths retain the provider’s spelling. A profile may symlink its
projects into another root; canonicalizing the stored path can destroy account
attribution. Usage is re-derived from transcript/provider evidence, not a
permanently stale total-cost column in sessions (that column was removed in v7).

## Hydration and retention

At startup the daemon loads up to 100 most-recent session rows and hydrates
Stopped/resumable state, restoring timestamps, tool/user-prompt counts,
transcript path, requested selection and account facts. RestoredSession does
not carry a provider field, and hydrate deliberately does not install its reported
model as fresh live telemetry; this is not full cross-provider snapshot serialization. Existing in-memory rows win over
hydration. Loading history can fail with a warning while the daemon continues;
that differs from a failure to open/migrate the database itself.

Hydration does not revive a process. Requested capacity is not confirmed live
capacity, and retained history is not proof of current liveness. Resume may
recreate state and replay provider history under the appropriate launch path.

Archive display and retention are separate. Stopped rows older than seven days
are hidden by the archive predicate and evicted from memory by maintenance.
`Db::prune_archived` selects by last-event age and preserves the newest 100 rows
regardless of age. It freezes the prune IDs, then deletes matching events and
sessions in one transaction; deterministic tie-breaking keeps both deletes on
the same set. It does not select using the in-memory mode, and it does not delete
the provider’s transcript files.

## Migration and rollback contract

`schema.rs` currently supports user_version 8 and refuses a higher version.
Numbered forward steps commit DDL and their version stamp together. Every step
also needs replay-safe DDL/catalog checks so partially applied older databases
can recover. A bare ADD COLUMN without checking the catalog is not replay-safe.

Canonical selection columns are additive and deliberately unversioned during
the v8 rollback window. The normal catalog check avoids an unnecessary write
transaction when both columns exist; a missing column is rechecked under an
immediate transaction before alteration. Rebuilding sessions must copy both
columns. Raising the version ends a compatibility promise and is not formatting
cleanup. `execution_leases` is also created idempotently outside numbered steps.

Rollback-compatible selection restore compares legacy projections before deciding
that canonical and legacy evidence disagree. A marker-free native-large-window
identity and an older marked spelling can represent the same legacy request.
Canonical selection is published additively on current snapshots; it is not
serde-skipped or safe for clients to reconstruct from an unrelated settings field.

## Verification

From `services/claudemon`:

```bash
cargo test --lib store::
```

Inspect the result count: these tests live in the library, not the binary target.
They cover schema creation/replay/rollback, event transactions, selection/account
round trips and retention. Use API and session-state tests as well when changing
publication or control acknowledgement, because a correct DB row alone does not
prove clients receive the same owner fields.
