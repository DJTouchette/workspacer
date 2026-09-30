---
title: TUI transport mocks do not establish Rust backend cutover
date: 2026-09-30
confidence: high
suggested_doc: tui-client
related_paths:
  - apps/tui/src/bus.rs
  - apps/tui/src/daemons.rs
  - .github/workflows/ci.yml
promoted: false
---

# TUI transport mocks do not establish Rust backend cutover

## Observation
apps/tui/src/bus.rs call/event/reconnect tests use a fake accept_async WebSocket server. daemons.rs ownership test drops a shell child waiting on stdin, while the tui CI job runs only the TUI crate. These establish protocol behavior and parent-pipe ownership separately but not actual BusClient calls/events against workspacer-rust or owned Rust backend restart/shutdown. A test-only daemons child module can own an isolated actual backend and reuse production BusClient/Daemons without adding a hub dependency or launching provider models.

## Recommendation
Add an explicitly invoked backend smoke command with required WKS_RUST_BACKEND_BIN, actual session/event operations, same-endpoint reconnect/resubscribe and process/port shutdown assertions. Keep interactive rendering and fixed-port ensure bootstrap coverage separate from this bounded transport/ownership proof.

## Implemented proof
The named test-tui-rust-backend target now executes the actual TUI BusClient and
Daemons owner against a supplied built Rust backend. Linux explicit probe1 passed
with actual session events/calls, same-client reconnect/resubscription and two
exit-status0/four-closed-port receipts. Borrowed-owner drop leaves the service
callable. A cfg(test)-only optional exit receipt observes the production Drop's
Child::wait, defaulting to None and absent from release builds. Ordinary owner6,
formatting and all-target clippy passed. Hub CI reuses its already-built executable
and existing shared cache action, without serializing the normal TUI unit job.
