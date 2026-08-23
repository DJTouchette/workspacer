---
title: wks-tui: standalone Rust TUI over claudemon REST+SSE
tags: [tui, rust, claudemon-client, sse, vim]
related_paths:
  - "apps/tui/src/**/*.rs"
  - "apps/tui/Cargo.toml"
owner: Damien Touchette
last_reviewed: 2026-08-16
---

# wks-tui: standalone Rust TUI over claudemon REST+SSE

## Overview
`apps/tui` is a standalone Rust/ratatui terminal client for workspacer agents that needs only **claudemon** running — no Electron. `apps/tui/src/claudemon.rs` hand-rolls HTTP/SSE over raw `TcpStream` against claudemon's loopback REST surface (list/transcript/approve/answer/message/signal/spawn/git + `/events` SSE + per-session `/sessions/:id/stream` PTY SSE). Since 2026-07 the TUI *also* defaults to being a thin client of the hub bus (`apps/tui/src/bus.rs`) for driving verbs (message/approve/answer/spawn/model/permission-mode), auto-spawning hub+brain for a loopback URL; `--direct` (or an unreachable bus) falls back to pure claudemon. Regardless of transport, the agent **list** is always pulled from claudemon directly (`fetch_agents` in `apps/tui/src/app/tasks.rs`) because the bus serves the desktop's enriched camelCase shape, which doesn't deserialize into the TUI's `Agent` model.

## Key modules
- `apps/tui/src/main.rs` — CLI (`Cli`), daemon bootstrap, the single `tokio::select!` event loop wiring keys/SSE/AppMsg/PTY chunks/statusline/bus events into `App`.
- `apps/tui/src/claudemon.rs` — the REST+SSE client (`Claudemon`), `spawn_events`/`spawn_status_lines` reconnect loops, `read_pty_stream`, `StreamEnd`, hand-rolled HTTP parsing (`parse_http_json`/`parse_http_status`).
- `apps/tui/src/bus.rs` — `BusClient` (reconnecting WebSocket over the hub `/bus` protocol) and `Driver`, which routes each agent-driving verb to the bus capability when connected, else straight to `Claudemon`.
- `apps/tui/src/app/mod.rs` — `App` state, `AppMsg` enum, `apply_msg`/`apply_bus_event` reducers, `set_agents` (stable ordering + orphan filtering), `dispatch`/`git_dispatch` fire-and-refresh helpers.
- `apps/tui/src/app/tasks.rs` — free async task bodies (`fetch_agents`, `fetch_transcript`, `fetch_git_*`, `seed_prompt`, `bracketed_paste`) shared between `App` methods and spawned tokio tasks.
- `apps/tui/src/app/input/` — key dispatch, routes every keypress through `Keymap::resolve`. Split out of a single `input.rs` in b03c1732: `dispatch.rs` (the resolve/route entrypoint), `nav.rs`, `panes.rs`, `pickers.rs` (fuzzy pickers incl. handoff), `questions.rs`, `query.rs`, `dialogs.rs`, `testutil.rs`.
- `apps/tui/src/keys.rs` — `Chord`/`Action`/`Context`/`Keymap`: leader-key (space) which-key menu, `Ctrl-w` window splits, vim counts, `:` cmdline verbs, config-overridable bindings.
- `apps/tui/src/daemons.rs` — `ensure()`/`Daemons` guard: auto-spawns `claudemon serve --hook-port 7890 --api-port 7891` (and the hub+brain in bus mode) if not already listening on loopback; only stops what it started.
- `apps/tui/src/terminal.rs` — the warm per-session terminal emulator (`Term`) fed by PTY chunks.
- `apps/tui/src/types.rs` — `Agent` (claudemon's snake_case wire shape), transcript folding (`turns_from_conversation`).

## Failure modes
- Both SSE loops (`spawn_events` for `/events`, `spawn_status_lines` for `/statusline/stream`) own independent reconnect-with-backoff (500ms → 8s cap); a mid-stream `Disconnected` is only emitted if `Connected` was previously sent, so a never-connected daemon doesn't spuriously flip UI state.
- `read_pty_stream` distinguishes `StreamEnd::Disconnected` (retry) from `StreamEnd::NoPty` (404 — an external/observed session with no PTY; caller must not retry and instead falls back to transcript view via `AppMsg::TerminalUnavailable` → `App::mark_no_terminal`).
- `apps/tui/src/bus.rs`'s `run()` loop fails all in-flight `Command::Call`s with `Err("bus disconnected")` on any WS drop before reconnecting — callers can hang if they don't handle that `Err` path.
- `Driver::spawn_managed` over the bus **forces approvals on** even when `yolo` is requested — a capability gap only exercised over REST (see comment in `apps/tui/src/bus.rs`).
- HTTP plumbing (`get_json`/`post_json`/`post_status`) opens a fresh `TcpStream` per call with `Connection: close`; no connection pooling, no timeout wrapper visible — a hung claudemon can block a request indefinitely.
- `App::dispatch`/`git_dispatch` are fire-and-refresh: on success they toast + re-pull the list/transcript; on error they only toast — no retry, no rollback beyond `SendFailed` restoring the composer text for message sends specifically.

## Gotchas
- **REST contract drift**: `apps/tui/src/claudemon.rs` and the desktop's daemon client are two independent implementations of the same claudemon HTTP surface. Any endpoint/shape change in claudemon (e.g. `/sessions`, `/sessions/:id/model`, `/sessions/:id/permission-mode`) must be mirrored here or the TUI silently breaks (wrong Result, missing field, or a 409 whose `{ok:false,error}` body isn't parsed the same way).
- **Twin driving paths**: every driving verb (`message`, `approve`, `answer_*`, `signal`, `spawn`, `spawn_managed`, `set_model`, `set_permission_mode`, `handoff`, `terminal_input`, `resize`) exists twice — once as a `Claudemon` REST method and once as a `Driver` bus/REST branch in `apps/tui/src/bus.rs`. Adding a new capability requires updating both, plus the corresponding hub brain capability, or the two transports drift.
- **List transport is fixed**: `App::refresh()` always calls `claudemon.list()` even in bus mode — this is intentional (bus serves a different shape) but easy to "fix" incorrectly by trying to route it through `Driver`.
- **PTY vs managed model-switch cliff**: `Claudemon::set_model` 409s for `claude` PTY sessions (they switch via the `/model` slash command on the message path instead); callers must check `provider_for()` before choosing REST/bus vs. slash-command routing.
- **base64 double-encoding conventions**: PTY bytes are base64 both over claudemon's `/sessions/:id/stream` SSE `data:` lines and over the bus's `pty.bytes.*` topic (`ev.data.as_str()` in `App::apply_bus_event`) — the two paths decode independently but must land in the same `feed_pty`/`Term` sink.
- **`apps/tui/src/keys.rs` exhaustiveness**: `Action::name`/`from_name` are hand-maintained parallel match arms (string ⇄ enum); a new `Action` variant added to one without the other breaks config round-tripping silently.
- **Auto-spawn ownership**: `daemons::ensure` only kills daemons *it* started (tracked in `Daemons.children`); an already-running claudemon/hub (e.g. Electron-owned) is left alone — don't assume TUI exit always tears down the daemon.
- No test file was found in `apps/tui` matching the parent task's touched files (`gitQueries.ts`/`ReviewPane.tsx`) — those are desktop-side and orthogonal to this domain; the TUI's own git review pane (`ReviewState` in `apps/tui/src/app/mod.rs`) is a separate parallel implementation worth checking if git-status/diff semantics change on the daemon side.

## Hand-authored notes (2026-07-30) — ui/ module split + modal_rect

- The former single-file ui renderer (2736 lines) split into `apps/tui/src/ui/` (one module per screen: `chrome`, `sidebar`, `dashboard`, `detail`, `chat`, `panes`, `overlays`, `review`, `runs`; `mod.rs` keeps `render()`, `ModalY`/`modal_rect`, `wrap`). Children inherit the prelude via `use super::*;` (a private `use` is visible to descendants); moved items are `pub(super)` — struct FIELDS need it too (only the test build catches a missed one, E0451). Commit `a6edd3a` is in `.git-blame-ignore-revs`.
- **Every overlay rect must go through `ui::modal_rect(area, w, h, ModalY)`.** The old hand-rolled centred rects clamped inconsistently, and ratatui panics when a widget rect leaves the buffer — the TUI aborted at real sizes (notes at 80x3, spawn/rename at 10x4); also `Ord::clamp` panics when min > max (whichkey crashed under 18 columns). A new overlay must use `modal_rect`, and the TestBackend render harness (`apps/tui/src/ui_render_tests.rs`) is what actually exercises draw paths — the older ui tests only covered string helpers.

## Hand-authored notes (2026-08-16) — federation

- In bus mode the TUI shows the merged federated fleet: `apps/tui/src/federation.rs` (`RemoteFleet` — per-hub session store, hub-stamped `agent.snapshot` ingest, seed/re-seed via `federation.peers` + `hub:<peer>/sessions.snapshots`, tombstones while a peer is down, plus the camelCase-row→`Agent` adapter). `Driver` qualifies per-session verbs as `hub:<peer>/<method>`; remote question answers route via `agents.sendMessage`; inherently local ops (PTY terminal, git) toast "local only" for remote sessions. Sidebar rows are hub-tagged; needs-you and the `m` jump span machines. None of this exists under `--direct` (claudemon has no federation). Sparse brain-only peer rows are ACCEPTED (since 2026-08-16): `fold_row` overlays sparse refreshes onto rich rows without clobbering enrichment, stopped-history rows ride the existing orphan curation, and `RemoteFleet::summary()` counts only live sessions so a brain's resumable-stopped backlog doesn't inflate the dashboard. See `modules/hub-federation.md`.
