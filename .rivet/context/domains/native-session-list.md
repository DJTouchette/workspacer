---
title: Native sidebar and session history ownership
tags: [native, sidebar, history, kept, archive, sessions, retention]
related_paths:
  - "apps/native/src/kept.rs"
  - "apps/native/src/ui/kept.rs"
  - "apps/native/src/ui/navigation.rs"
  - "apps/native/src/ui/archive.rs"
  - "apps/native/src/ui/recent.rs"
  - "apps/native/src/history.rs"
  - "apps/native/src/controller.rs"
  - "services/hub-rs/src/services/live_controls.rs"
  - "services/hub-rs/src/services/snapshots.rs"
  - "services/hub-rs/src/services/analytics.rs"
  - "services/claudemon/src/store/mod.rs"
last_reviewed: 2026-10-26
---

# Native sidebar and session history

## Who owns what

| Question | Single owner |
| --- | --- |
| Which sessions exist and their live state | claudemon `SessionStore` (memory) |
| Which ended sessions the fleet list shows | hub `snapshots::visible` |
| Which extra sessions this device keeps listed | `wks_native::kept` (policy) over `Settings::kept_open` |
| Hidden by archive | hub shared archive, else device `Settings::archived` |
| Hidden finished children | device `Settings::cleared_children` |
| History rows | hub `sessions.recent` (engine rows plus durable record) |
| Durable "this session existed" | hub analytics `session_history` (`headless-analytics.sqlite`) |

The sidebar is `controller` fleet rows (`sessions.snapshots`, plus kept ids
read by id) filtered in `Workspace::visible_sessions`. Archive and cleared
children only remove rows. Only `kept::fold` decides to add or let go of a
kept session; change that policy there, not in the UI.

## Retention facts that caused lost and returning sessions

- claudemon persists to SQLite only from Claude hook events. PTY launches get
  hooks; stream (native's only transport) and Codex/other managed sessions do
  not. Every engine restart (each `--local` launch) forgets them. The engine
  also hydrates only the newest 100 rows and evicts stopped rows older than
  seven days from memory on its first maintenance tick.
- The hub's analytics watcher records every session it observes, all
  providers, and never prunes. `sessions.recent` merges those rows for ids the
  engine no longer has (stopped, transport from the launch journal, Claude
  rows with no prompt/tool/token hidden like the engine's empty rows). A
  failed engine read with no durable store is an error, never `[]`.
- The fleet list shows a stopped row for 24h, unless a hub layout document
  exists (written by the desktop/web panes); then only stopped sessions the
  layout names are shown. In bus mode against a desktop hub, the desktop's
  open panes therefore decide which old sessions native lists.
- `kept_open` stamps are "last seen open" (refreshed hourly). Entries expire
  after seven days, are dropped when archived, when they end while this run
  watched them, or when the controller reports them in `View::kept_gone`
  (neither `sessions.snapshot` nor `sessions.recent` has them). Reads are
  capped at 64, newest first; previously the cap took the alphabetically
  first ids and nothing expired.

## Known remaining gaps

- A kept session stopped by a hub that outlives the app (bus mode) still
  reads as Paused until it expires; native cannot tell "app closed" from
  "agent finished while away".
- `Settings` is one JSON file written whole; two native processes on the
  same machine overwrite each other's kept/archived/names.
- Device state is scoped by hub identity (`rust-local:<dir>` or the bus URL),
  so switching between `--local` and attaching to a hub shows different kept,
  archived and renamed sets.
- Sessions shorter than the analytics watcher's pass interval (5s active)
  may never reach the durable record.
