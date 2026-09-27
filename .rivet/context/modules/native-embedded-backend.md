---
title: Native embedded backend lifetime and hub integration
tags: [native, gpui, claudemon, embedded, tokio, channels, hub, shutdown]
related_paths:
  - "apps/native/src/host.rs"
  - "apps/native/src/host/local.rs"
  - "apps/native/src/backend.rs"
  - "apps/native/src/controller.rs"
  - "services/claudemon/src/daemon/embedded.rs"
  - "services/hub/cmd/workspacer/serve.go"
last_reviewed: 2026-09-27
---

# Native embedded backend

`NativeHost` starts an owned OS thread and Tokio runtime without entering a
runtime on GPUI's thread. The UI sends bounded typed controller commands and
receives the latest immutable view through Tokio watch. Keeping the host alive
keeps the worker alive without a window; dropping a UI receiver is not shutdown.

Existing-hub and demo modes remain independent from local embedding. Only
explicit `--local` starts `EmbeddedDaemon` and owns a gateway process. That mode
requires workspacer/hub/brain/mcp binaries, uses a separate native database,
refuses occupied hub/MCP ports, waits for actual capability registration, and
never silently falls back to an unrelated running daemon. Custom hub/MCP ports
require an explicit database path at the GUI entry point.

The claudemon library has its own named thread/runtime, readiness/status watch,
bounded command/reply API, and a single-runtime lease protecting callback
addresses. Its retained HTTP/hook listeners let the Go brain and facade keep
working. Local message/approval/interrupt and stream-answer commands use the
in-process API; PTY answers retain the hub's keystroke path. Launch, model
catalog, enriched snapshots, and conversation events still go through hub
services. Embedding does not itself replace orchestration or agent tools.

`workspacer serve --external-claudemon` validates the selected loopback daemon's
health contract but never initializes hooks, opens its database, restarts it,
or shuts it down. Parent-death stdin EOF requests ordered shutdown of the
owned brain/facade/hub stack. Keep the stdin lease outside Tokio Child: polling
Child::wait closes its own stdin and would otherwise shut down healthy services. Native shutdown stops that stack before joining
the engine. Never infer ownership from a responsive port alone.

The embedded engine uses explicit shutdown rather than installing process
signal handlers or Windows job confinement for the host UI process. Spawn
admission and PTY reaping are fenced during shutdown. If runtime cleanup times
out, the process must not start a replacement engine over potentially surviving
work. Agent process crash survival is not guaranteed by using another thread.

Window close quits by default; `--keep-running` minimizes the existing window
so the taskbar/dock can restore it. Explicit Quit remains the shutdown action. GPUI quit observers synchronously
join the backend before returning their ready future: macOS termination need
not return from Application::run, and GPUI allows only 100ms for async quit work.
Native launches use stream transport; local startup does not mutate global
Claude hook settings.

Use the complete native UI/protocol suite plus claudemon lifecycle tests and
workspacer launcher tests for changes here. `native-harness embedded-probe`
exercises an actual embedded engine plus owned Go services without model calls;
use fresh XDG config/data, disable account polling, and choose isolated ports.
Witness currently leaves some native/embedded sources unmapped; a narrow
co-change test selection is not proof of lifecycle coverage.
