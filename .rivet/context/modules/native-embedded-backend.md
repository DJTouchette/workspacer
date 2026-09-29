---
title: Native embedded backend lifetime and hub integration
tags: [native, gpui, claudemon, embedded, tokio, channels, hub, shutdown]
related_paths:
  - "apps/native/src/host.rs"
  - "apps/native/src/host/rust_local.rs"
  - "apps/native/src/backend.rs"
  - "apps/native/src/controller.rs"
  - "services/claudemon/src/daemon/embedded.rs"
  - "services/hub-rs/src/backend.rs"
last_reviewed: 2026-09-29
---

# Native embedded backend

`NativeHost` starts an owned OS thread and Tokio runtime without entering a
runtime on GPUI's thread. The UI sends bounded typed controller commands and
receives the latest immutable view through Tokio watch. Keeping the host alive
keeps the worker alive without a window; dropping a UI receiver is not shutdown.

Existing-hub and demo modes remain independent from local embedding. Explicit
`--local` or `--rust-local-dir` selects the shared Rust `Backend` (the `rust-hub`
feature is required). The default directory is `Workspacer Native Rust Preview`
in the platform data directory; an override owns its own config, hub data and
SQLite file. Old service-bundle/database overrides and custom hub/MCP ports are
refused. Bare GUI startup still attaches to an existing hub.

The local owner prepares claudemon and initializes the Rust hub/services/MCP
inside the application, then connects `Backend::in_process` to its handle. It
starts no Go gateway, brain or private Node companion. Claudemon retains owned
loopback API/hook listeners on assigned ports for external integrations; owned
listener receipts support shutdown checks. The same shared Rust backend can run
under the standalone `workspacer-rust serve` CLI. CLI signal/stdin/parent watching
is separate from library lifetime and never imposed on the GUI.

Local readiness follows successful initialization and in-process connection,
with a 90-second startup budget. Controller completion, requested shutdown or
backend failure all join the shared owner's cleanup. An unrelated responsive
listener is never adopted as the local owner's engine. Local provider accounts
and configured manual Claude hook integration still use the selected home;
isolated backend stores do not mean every external integration is isolated.

The claudemon library has its own named thread/runtime, readiness/status watch,
bounded command/reply API and exclusive store/listener ownership. UI commands
travel through the controller off GPUI's thread. External clients may still use
the supported HTTP/WebSocket interfaces; embedding removes private backend
process hops, not every network interface or third-party provider process.

The embedded engine uses explicit shutdown rather than installing process
signal handlers or Windows job confinement for the host UI process. Spawn
admission and PTY reaping are fenced during shutdown. If runtime cleanup times
out, the process must not start a replacement engine over potentially surviving
work. Agent process crash survival is not guaranteed by using another thread.

Window close quits by default; `--keep-running` minimizes the existing window
so the taskbar/dock can restore it. Explicit Quit remains the shutdown action. GPUI quit observers synchronously
join the backend before returning their ready future: macOS termination need
not return from Application::run, and GPUI allows only 100ms for async quit work.
Native launches use stream transport. The local owner explicitly configures
manual Claude hook integration; do not claim it never touches hook settings.

Use the native UI/protocol suite, claudemon lifecycle tests and shared Rust
backend/launcher tests for changes here. The native harness Rust embedded probe
uses the actual shared backend with isolated state and no model calls, checking
readiness and owned-listener release. This is a headless backend probe, not proof
that the installed GPUI window rendered or a perceived-latency benchmark. The
Windows release smoke checks package install/upgrade/uninstall plus that probe.
Witness leaves some embedded sources unmapped; co-change test selection alone
is not lifecycle coverage. Performance changes need a named reproducible
workload and explicit separation between parser/controller timings and actual
window rendering.
