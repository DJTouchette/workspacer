---
title: Architecture Overview — Three Processes
tags: [architecture, processes, ipc, daemon]
related_paths:
  - "apps/desktop/src/main/index.ts"
  - "services/claudemon/src/main.rs"
  - "services/claudemon/src/cli.rs"
  - "services/hub/cmd/brain/main.go"
  - "services/hub/internal/claudemon/bridge.go"
  - "apps/tui/src/main.rs"
  - "apps/desktop/src/main/shared/ipcChannels.ts"
  - "services/claudemon/src/session/state.rs"
owner: Damien Touchette
last_reviewed: 2026-08-16
---

# Architecture Overview — Three Processes

## Overview
Workspacer runs a three-daemon spine: the Electron desktop shell (TS/React, spawns + supervises), the claudemon observability daemon (Rust, owns session state and transports), and the hub event bus with brain provider (Go, routes capabilities and plugins). The desktop and TUI are thin clients over REST+SSE or WebSocket; claudemon is the single source of truth for what every agent is doing. When remote-connected or headless, all three run independently of the desktop — the hub and brain can serve multiple clients at once. Since 2026-08 the hub can also *federate*: it links outbound to named peer hubs on other machines and republishes their fleet events locally (envelope-stamped with the peer name), so every client still holds exactly one bus connection but sees a merged multi-machine fleet — see `modules/hub-federation.md` / `docs/hub-federation.md`.

## Key modules

- `apps/desktop/src/main/index.ts` — Electron main process: spawns claudemon + hub, registers IPC handlers, bridges daemon events to React renderer, manages graceful shutdown.
- `services/claudemon/src/cli.rs` — CLI dispatch for `serve`, `init`, `wrap`, `watch` subcommands; `serve` binds hook listener (7890) and REST API (7891).
- `services/claudemon/src/session/state.rs` — SessionState enum machine driven by hook events; defines HookEventKind, SessionMode (Unknown/Input/Responding/Approval/Question/Stopped), Pending union, and Plan.
- `services/claudemon/src/daemon/mod.rs` — Axum HTTP servers: hook POST ingress, REST session endpoints, SSE /events stream, PTY wrapper WebSocket.
- `services/hub/cmd/brain/main.go` — Headless provider: connects to hub bus, registers capabilities (spawn/send/list), bridges claudemon /events into agent.snapshot events, owns session store in full-scope mode.
- `services/hub/internal/claudemon/bridge.go` — Consumes claudemon SSE stream, re-publishes as bus agent.* events via mapEvent; reconnects on drop.
- `services/hub/internal/federation/federation.go` — Peer-hub links (peers.json): republishes peer `agent.*`/`workflow.*` events locally with the peer name on the envelope; peer capabilities callable as `hub:<peer>/<method>`.
- `apps/tui/src/main.rs` — Terminal UI: connects to claudemon REST+SSE (or hub bus), spawns daemons if needed, drives agents with vim-style keys.
- `apps/desktop/src/main/services/claudemonSessionClient.ts` — HTTP client for claudemon /sessions, /message, /approve endpoints.

## Failure modes

**Daemon startup ordering:** Electron creates a BrowserWindow, then spawns claudemon, then creates bridges, then starts hub. A hung claudemon startup blocks the bridges but not renderer paint; startup notifications signal failures to the user without crashing.

**SSE reconnect:** The hub bridge (bridge.go:44–58) reconnects on stream drop with a 1-second backoff; while disconnected, session updates don't flow to the bus, but the UI remains usable (stale snapshot).

**Mode-gated endpoints:** claudemon returns HTTP 409 Conflict when /message is called in non-Input mode, /approve in non-Approval mode, etc. The desktop must check session.mode before sending; race-condition sends get rejected and must retry or alert.

**SessionState lifecycle:** When a Stop event fires while live_subagents > 0, the session stays Responding until SubagentStop drains all subagents — a misaligned Stop/SubagentStop sequence can strand the session in Responding or flip it to Input prematurely.

**Hook event parsing:** Unrecognized hook event names are silently ignored (state.rs:542–547); forward-compat depends on new clients ignoring unknown modes and new daemon versions tolerating missing fields.

## Gotchas

**IPC channel exhaustion:** The desktop app has two-way IPC channels (ipcChannels.ts) for every major action (spawn, message, approve, etc.). Adding a new channel requires updating both main (ipc.ts) and renderer (preload.ts). The CLAUDE_SPAWN channel is especially fragile — if the renderer doesn't await the IPC, the session races against daemon startup.

**SessionState transport field:** The transport enum (PTY vs Stream) is serialized onto every snapshot; clients must check it to hide PTY-only affordances (e.g. Term view) on stream sessions. It's back-compatible (defaults to PTY) but omission breaks stream-mode detection.

**Hub scope conflict:** The brain command runs in full-scope (owns session store + events) or catalog-scope (desktop app owns store). Both cannot run against the same hub simultaneously; the desktop must detect and suppress a local hub if one exists remotely.

**Subagent background-task bookkeeping:** live_subagents and parent_turn_ended are non-serialized internal state. They survive stop-then-resume ONLY if the SessionState object isn't reconstructed from disk (state.rs:480–486); resuming a stopped session from SQLite must zero these fields to avoid idle false-negatives.

**Config mtime gate + lock:** Both the desktop TS config service and the brain Go code write config.yaml. Each mtime-gates its refresh, and since 2026-07-31 an O_EXCL cross-process lockfile (`config.yaml.lock`, held across refresh→merge→write by both writers; parameters pinned in `contracts/config-lock.json`) closes the interleaved-write window — see `domains/config.md`.

**TUI bus token discovery:** The TUI reads `~/.config/workspacer/remote-token` to auto-join a desktop-owned hub (main.rs:90). If this token doesn't match the hub's expected token, all bus operations hang reconnecting; pass `--bus-token` explicitly when the desktop's token is unavailable.
