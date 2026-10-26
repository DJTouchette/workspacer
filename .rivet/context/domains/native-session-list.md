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
| Which ended sessions the fleet list shows | hub `snapshots::visible` (native asks `uncurated`) |
| Which extra sessions this device keeps listed | `wks_native::kept` (policy) over `Settings::kept_open` |
| Hidden by archive | hub shared archive, else device `Settings::archived` |
| Hidden finished children | device `Settings::cleared_children` |
| Which hub device state belongs to | `Settings::hub_scopes` (address → `hub:<id>`) |
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
  layout names are shown. Native sends `sessions.snapshots {uncurated: true}`
  (honored by both the Rust hub and the desktop hub), so another client's
  open panes never decide which ended sessions native lists.
- `kept_open` stamps are "last seen open" (refreshed hourly). Entries expire
  after seven days, are dropped when archived, when they end while this run
  watched them, or when the controller reports them in `View::kept_gone`
  (neither `sessions.snapshot` nor `sessions.recent` has them). Reads are
  capped at 64, newest first.
- When the hub outlives the app (attached, not `--local`), the app records
  `Settings::closed_at` for the hub on quit. At the next start that value is
  read once and cleared; a kept session first seen stopped whose
  `lastActivity` is after it (plus a two-minute grace) finished while you
  were away and is let go instead of shown as Paused. A crash leaves no
  close on record, so everything stays Paused (conservative). `--local`
  ignores it: the app's quit stopped everything it owned.

## Device state and hub identity

The Rust hub writes a persistent id to `<data_dir>/hub-id.json` and sends it
as `hubId` in every hello (embedded and remote clients expose it as
`hub_id()`). Native starts in the scope remembered for the address it
dialed, and once a fleet read answers it moves to `hub:<id>`: per-hub maps
are merged into that scope, the turn-timing directory and remembered
terminals file are renamed, and the address is aliased so the next launch
starts there. `--local` and `workspacer serve` on the same directory, or
`localhost` vs `127.0.0.1`, therefore share kept, archived and named
sessions. Hubs without an id (the desktop hub) stay keyed by address.

`Settings` is still one file, but saves are atomic (temp file + rename) and
`save_merged` keeps the file's entries for every hub scope this process does
not own, so a `--local` window and an attached one no longer overwrite each
other. Two processes on the *same* hub remain last-writer-wins for that hub.

The analytics watcher runs one final pass (2s engine budget) when the hub
closes, so a session started or ended inside the 5-second pass interval
before shutdown still reaches the durable record.
