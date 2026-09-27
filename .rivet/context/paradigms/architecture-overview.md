---
title: Architecture Overview — Clients, Daemons, and Capability Providers
tags: [architecture, processes, ipc, daemon, native, embedded, tokio]
related_paths:
  - "apps/desktop/src/main/index.ts"
  - "services/claudemon/src/main.rs"
  - "services/claudemon/src/cli.rs"
  - "services/hub/cmd/brain/main.go"
  - "services/hub/internal/claudemon/bridge.go"
  - "apps/tui/src/main.rs"
  - "apps/native/src/host.rs"
  - "apps/native/src/host/local.rs"
  - "apps/desktop/src/main/shared/ipcChannels.ts"
  - "services/claudemon/src/session/state.rs"
owner: Damien Touchette
last_reviewed: 2026-09-27
---

# Architecture Overview — Clients, Daemons, and Capability Providers

## Overview
Workspacer separates clients, session execution, and capability routing. The
Electron desktop shell hosts a React renderer and can supervise local services;
claudemon (Rust) owns agent transports and daemon session state; the hub (Go)
routes bus capabilities and events and supervises plugin sidecars. It can also
supervise a brain provider, as in desktop catalog mode; `workspacer serve` owns
its brain as a separate launcher child. The MCP facade is a separate service translating agent tools to bus
calls. This is a process tree, not a fixed three-process deployment.

In desktop host mode, Electron supplies live/enriched session capabilities and
normally delegates file-backed catalog capabilities to a catalog-scope brain.
`WORKSPACER_NO_BRAIN=1` keeps those catalog capabilities in Electron instead.
The renderer normally uses the bus for shared data and IPC for native host
features; `WORKSPACER_DESKTOP_DIRECT=1` selects the pure-IPC data path. See
[the backend seam](../domains/renderer-backend-seam.md).

`workspacer serve` runs the services without Electron, using a full-scope brain
for the live session projection and capability provider. The brain can also
launch a private Node companion to execute shared desktop TypeScript services;
see [headless desktop services](../modules/headless-desktop-services.md).
In desktop remote-client mode, Electron starts no local claudemon, hub, or facade; the renderer connects
to the remote bus. The TUI defaults to the bus, with `--direct` for claudemon
REST/SSE. Providers project claudemon state into client snapshots and add their
own enrichment; daemon state and the desktop snapshot are not identical models.

The native GPUI app remains an existing-hub client by default. Its explicit
`--local` mode embeds claudemon on an owned Tokio backend thread and starts
`workspacer serve --external-claudemon` for hub/brain/MCP services. Typed Tokio
commands and latest-state watch snapshots separate UI lifetime from backend
work. Local session controls can use the embedded channel API; launches and
client projections still use the hub. This phase retains the hub and does not
port the brain's service responsibilities into the native app. See
[native embedding](../modules/native-embedded-backend.md).

Federation links named hubs and republishes selected peer events locally with
peer provenance. Clients can use one bus connection for the merged fleet;
qualified methods use `hub:<peer>/<method>`. See
[hub federation](../modules/hub-federation.md).

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

**SSE reconnect:** The hub bridge (`services/hub/internal/claudemon/bridge.go`) reconnects on stream drop with exponential backoff from 200 ms to 5 seconds, resetting after a stream lasts at least 5 seconds; while disconnected, session updates don't flow to the bus, but the UI remains usable (stale snapshot).

**Message queue versus pending actions:** `/message` accepts live-session
messages immediately or queues them until ready, including while responding or
showing an approval/question. Stopped sessions return 409. `/approve` and
`/answer` still reject mismatched pending modes. See
[the HTTP API contract](../domains/claudemon-http-api.md).

**SessionState lifecycle:** When a Stop event fires while live_subagents > 0, an unblocked session stays Responding until SubagentStop drains all subagents; an existing Approval/Question mode is preserved — a misaligned Stop/SubagentStop sequence can strand the session in Responding or flip it to Input prematurely.

**Hook event parsing:** Unrecognized hook event names are ignored by the state machine. This does not prove readers accept new SessionMode values or changed snapshot shapes; test those wire changes separately and preserve optional-field compatibility.

## Gotchas

**IPC registration:** New channels require aligned main handlers, shared payload
shapes, preload methods, renderer declarations, and backend classification.
Await spawn results before consuming their returned session identity. Runtime
readiness remains the host launcher's responsibility, not something an awaited
renderer IPC alone proves. See [IPC boundary](../modules/ipc-boundary.md).

**SessionState transport field:** The transport enum (PTY vs Stream) is serialized onto every snapshot; clients must check it to hide PTY-only affordances (e.g. Term view) on stream sessions. It's back-compatible (defaults to PTY) but omission breaks stream-mode detection.

**Capability ownership:** The router is single-owner per method. A catalog-scope brain is designed to coexist with Electron: brain owns the delegated catalog and Electron owns live/enriched capabilities. A full-scope brain owns the headless surface. When adopting an existing hub, desktop capability registration must respect the adopted provider ownership; do not register overlapping methods. Remote-client mode is a separate startup choice and does not start a local hub.

**Subagent background-task bookkeeping:** live_subagents and parent_turn_ended are non-serialized internal state. They survive stop-then-resume ONLY if the SessionState object isn't reconstructed from disk; resuming a stopped session from SQLite must zero these fields to avoid idle false-negatives.

**Config mtime gate + lock:** Both the desktop TS config service and the brain Go code write config.yaml. Each mtime-gates its refresh, and since 2026-07-31 an O_EXCL cross-process lockfile (`config.yaml.lock`, held across refresh→merge→write by both writers; parameters pinned in `contracts/config-lock.json`) closes the interleaved-write window — see `domains/config.md`.

**TUI bus token discovery:** Explicit CLI/environment credentials take precedence
over configured/discovered tokens. The default discovery uses the shared
Workspacer config directory’s `remote-token`. A rejected credential is not a
working bus connection; startup may fall back for an unusable loopback bus,
while an explicitly remote endpoint remains the requested target. See
[the TUI guide](../domains/tui-client.md).
