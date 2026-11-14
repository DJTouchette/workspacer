---
title: Architecture Overview — Clients, Daemons, and Capability Providers
tags: [architecture, processes, ipc, daemon, native, embedded, tokio]
related_paths:
  - "apps/native/src/main.rs"
  - "apps/native/src/host.rs"
  - "apps/native/src/host/rust_local.rs"
  - "services/hub-rs/src/backend.rs"
  - "services/hub-rs/src/runtime.rs"
  - "services/hub-rs/src/server.rs"
  - "services/hub-rs/src/mcp.rs"
  - "services/claudemon/src/main.rs"
  - "services/claudemon/src/cli.rs"
  - "services/claudemon/src/daemon/embedded.rs"
  - "services/claudemon/src/session/state.rs"
  - "apps/tui/src/main.rs"
  - "apps/desktop/src/main/index.ts"
owner: Damien Touchette
last_reviewed: 2026-11-14
---

# Architecture Overview — Clients, Daemons, and Capability Providers

## Overview
Workspacer separates clients, session execution, and capability routing. The
shipping stack is Rust. claudemon owns agent transports and session state. The
Rust hub (`services/hub-rs`) routes bus capabilities and events, runs the
desktop services behind them, supervises plugin sidecars and serves the MCP
facade that translates agent tools to bus calls. One `Backend`
(`services/hub-rs/src/backend.rs`) owns an embedded claudemon engine and the
hub in a single process.

That backend runs in two hosts:

- **The native app** (`apps/native`, binary `wks-native`) is the standard
  client. Started with `--local` (what the installers, `make dev` and `./dev`
  do), it embeds the backend on an owned Tokio thread and talks to it in
  process. Started without `--local`, it is a client of an existing hub over
  the bus (`--bus`, default `ws://127.0.0.1:7895/bus`). See
  [native embedding](../modules/native-embedded-backend.md).
- **`workspacer serve`** (`workspacer-rust serve`) runs the same backend
  standalone, and serves the web clients: `/m`, `/remote`, and `/app` from the
  bundled `web/` directory.

The Go hub, the Go `brain` provider and the private Node companion that ran
shared desktop TypeScript services are retired. Their sources exist only in
the pinned historical checkout (`scripts/reference/README.md`). The native
backend starts none of them.

The Electron app (`apps/desktop`) is deprecated; v0.170.0 was its final
release. Its renderer still ships as the `/app` web client, and its main
process code remains in the tree (hub-rs tests, generators and contract
fixtures read files under `apps/desktop`). The renderer documents
([the backend seam](../domains/renderer-backend-seam.md),
[IPC boundary](../modules/ipc-boundary.md)) describe that code, not the native
app.

Other clients share the same bus: the TUI defaults to the bus, with `--direct`
for claudemon REST/SSE; the phone and browser clients connect with a pairing
token. Providers project claudemon state into client snapshots and add their
own enrichment; daemon state and the client snapshot are not identical models.

Periodic host chores (keep-warm, worktree artifact cleanup) start with every
backend that owns its engine (`services/hub-rs/src/services/host_chores.rs`),
so they run under both the native app and `workspacer serve`.

Federation links named hubs and republishes selected peer events locally with
peer provenance; qualified methods use `hub:<peer>/<method>`. See
[hub federation](../modules/hub-federation.md). The native app lists only the
connected hub's own sessions; a federated peer's sessions are not shown there.

## Key modules

- `apps/native/src/main.rs` — argument parsing, mode selection (`--local`, `--bus`, `--demo`), single-instance claim, data-folder selection, window creation.
- `apps/native/src/host/rust_local.rs` — owns the embedded backend: listen ports, config directory, Electron import, readiness and joined shutdown.
- `services/hub-rs/src/backend.rs` — `Backend`: starts the embedded engine, the hub and the MCP facade; starts host chores after readiness; ordered shutdown.
- `services/hub-rs/src/runtime.rs` — the hub core: connections, authentication, capability routing and events.
- `services/hub-rs/src/server.rs`, `server/policy.rs`, `server/web.rs` — the bus WebSocket, Host/Origin policy, and the web clients.
- `services/hub-rs/src/mcp.rs` — the MCP facade; catalog in `services/hub-rs/assets/mcp-tools.json`.
- `services/hub-rs/src/federation.rs` — peer-hub links.
- `services/claudemon/src/cli.rs` — CLI dispatch for `serve`, `init`, `wrap`, `watch` subcommands; `serve` binds hook listener (7890) and REST API (7891) when claudemon runs as its own daemon.
- `services/claudemon/src/daemon/embedded.rs` — the engine as a library: named thread/runtime, bounded command API, owned listeners.
- `services/claudemon/src/session/state.rs` — SessionState enum machine driven by hook events; defines HookEventKind, SessionMode (Unknown/Input/Responding/Approval/Question/Stopped), Pending union, and Plan.
- `services/claudemon/src/daemon/mod.rs` — Axum HTTP servers: hook POST ingress, REST session endpoints, SSE /events stream, PTY wrapper WebSocket.
- `apps/tui/src/main.rs` — Terminal UI: connects to the hub bus (or claudemon REST+SSE), drives agents with vim-style keys.
- `apps/desktop/src/main/index.ts` — deprecated Electron main process.

## Failure modes

**Backend startup ordering:** `Backend::initialize` waits for the engine, then starts the hub, then (when an MCP listener is configured) waits for the facade catalog, then starts host chores. The native window opens before the backend is ready and shows startup and owned-service failures in the UI; local readiness has a 90-second budget.

**Second launch:** one native app runs per data folder. A second launch asks the first for its window and exits (`apps/native/src/instance.rs`) rather than contending for the session database lease.

**Message queue versus pending actions:** `/message` accepts live-session
messages immediately or queues them until ready, including while responding or
showing an approval/question. Stopped sessions return 409. `/approve` and
`/answer` still reject mismatched pending modes. See
[the HTTP API contract](../domains/claudemon-http-api.md).

**SessionState lifecycle:** When a Stop event fires while live_subagents > 0, an unblocked session stays Responding until SubagentStop drains all subagents; an existing Approval/Question mode is preserved — a misaligned Stop/SubagentStop sequence can strand the session in Responding or flip it to Input prematurely.

**Hook event parsing:** Unrecognized hook event names are ignored by the state machine. This does not prove readers accept new SessionMode values or changed snapshot shapes; test those wire changes separately and preserve optional-field compatibility.

## Gotchas

**IPC registration (Electron/web client only):** New channels require aligned main handlers, shared payload
shapes, preload methods, renderer declarations, and backend classification.
Await spawn results before consuming their returned session identity. Runtime
readiness remains the host launcher's responsibility, not something an awaited
renderer IPC alone proves. See [IPC boundary](../modules/ipc-boundary.md).

**SessionState transport field:** The transport enum (PTY vs Stream) is serialized onto every snapshot; clients must check it to hide PTY-only affordances (e.g. Term view) on stream sessions. It's back-compatible (defaults to PTY) but omission breaks stream-mode detection.

**Capability ownership:** The router is single-owner per method. In the Rust hub the built-in services register their methods in process; there is no brain provider to delegate to. A native app attached to an existing hub (no `--local`) registers nothing and starts no local hub.

**Subagent background-task bookkeeping:** live_subagents and parent_turn_ended are non-serialized internal state. They survive stop-then-resume ONLY if the SessionState object isn't reconstructed from disk; resuming a stopped session from SQLite must zero these fields to avoid idle false-negatives.

**Config mtime gate + lock:** Both the Rust config service (`services/hub-rs/src/services/config.rs`) and the deprecated Electron TS config service write config.yaml. Each mtime-gates its refresh, and since 2026-07-31 an O_EXCL cross-process lockfile (`config.yaml.lock`, held across refresh→merge→write by both writers; parameters pinned in `contracts/config-lock.json`) closes the interleaved-write window — see `domains/config.md`.

**TUI bus token discovery:** Explicit CLI/environment credentials take precedence
over configured/discovered tokens. The default discovery uses the shared
Workspacer config directory’s `remote-token`. A rejected credential is not a
working bus connection; startup may fall back for an unusable loopback bus,
while an explicitly remote endpoint remains the requested target. See
[the TUI guide](../domains/tui-client.md).
