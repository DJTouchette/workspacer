---
title: wks-tui: bus-first terminal client with claudemon direct mode
tags: [tui, rust, claudemon-client, sse, vim, overlay]
related_paths:
  - "apps/tui/src/**/*.rs"
  - "apps/tui/Cargo.toml"
owner: Damien Touchette
last_reviewed: 2026-09-26
---

# Standalone wks-tui client

## Startup and connection ownership

`apps/tui` is the standalone ratatui client, separate from embedded
`claudemon watch`. `main.rs` selects bus mode by default; `--direct` bypasses
it. A nondefault CLI/environment bus URL wins; when it equals the compiled
default, `tui.json` can supply `hubUrl`. Thus an explicitly supplied default
URL is not distinguished from an omitted default. Token precedence is explicit
CLI/environment, config, then the local saved hub token.

`daemons.rs` optionally ensures claudemon and a hub with supervised full-scope
brain. `--no-spawn` disables this. Each URL's loopback status is checked
separately: selecting a remote bus does not by itself suppress local claudemon
bootstrap if that URL is still local. Claudemon bootstrap probes/launches the
fixed 7891 API and 7890 hook ports; it is not a general custom-port launcher.
The `Daemons` guard kills/waits only for children this TUI started, leaving
adopted processes alone. Port availability is not provider health proof.

An unusable loopback bus can cause startup fallback to direct claudemon. An
explicit remote bus does not fall back to a different local machine. This is
a startup choice, not per-call silent failover. The module header still contains
older direct-only prose; executable bootstrap is authoritative.

## Transports and state

`claudemon.rs` implements raw TCP HTTP/SSE with a fresh connection per ordinary
request. It is intended for the daemon's HTTP surface; stripping an HTTPS scheme
in its host parser does not add TLS. There is no ordinary-request timeout
wrapper. Event/statusline/conversation SSE loops reconnect with backoff from
500 ms to 8 seconds. PTY 404 becomes `NoPty` and falls back to transcript view;
transport disconnection is retriable. Do not treat every missing PTY as a daemon
failure.

`bus.rs` contains the reconnecting WebSocket client and `Driver`. Local methods
use bus when a BusClient is configured, otherwise direct REST. A disconnected
configured bus returns failure; it does not automatically retry via REST.
Remote-owned drivers qualify methods and explicitly refuse local fallback.
Pending calls fail on a dropped connection. A returned error does not guarantee
the remote side effect was absent.

The local agent list remains a direct claudemon fetch (`apps/tui/src/app/tasks.rs`), even
in bus mode; federation has its own row adapter and remote store. Consequently
an arbitrary remote bus URL alone does not make every local-data path a pure
remote client. Main merges keys, daemon events/statuslines/conversation,
bus messages and PTY bytes into App reducers. Base64 PTY payloads from direct
SSE and bus topics decode independently into the same terminal emulator.

## Driving agents and federation

Keep each `Driver` verb, direct REST method and corresponding hub capability
consistent. Model switching uses the daemon's durable selection contract and
preserves queued/applied outcomes. Legacy PTY responses requiring slash commands
become upgrade-required errors; do not restore an untracked client-side `/model`
write. New Claude launches carry the TUI's configured transport explicitly.
Provider permission choices are launch configuration, not Workspacer tool grants.

`federation.rs` seeds/reseeds peer fleets, accepts sparse rows, preserves richer
fields and keeps offline tombstones. Live summaries exclude stopped backlog.
Remote model/message/signal operations qualify to the owner. Remote answers
use selected text (multi-answers joined in order) through `agents.sendMessage`;
raw numeric remote answers are refused. Terminal/git access, permission-mode
changes and handoff retain local-only restrictions at their respective entry
points. See [handoff](cross-provider-handoff.md) for its separate send semantics.

The bus hello is republished as `_bus.hello`; App uses its scope for UI gates
such as node wake, with absent scope denying that affordance. A scope label is
not authenticated-host provenance and an advertised method is not proof a
provider is live. Server authorization remains authoritative.

## UI and failure behavior

`apps/tui/src/app/input/` owns key routing, pickers, navigation, questions and dialogs;
`keys.rs` owns action/name mapping and configurable keybindings. Changes must
round-trip enum and string forms. App dispatch helpers toast and refresh on
success, toast on failure; message send failures can restore composer content.
Do not describe that as transactional rollback of a launched successor or
remote mutation.

`ui/` separates chrome, sidebar, dashboard, detail/chat, panes, overlays, review
and runs. Use `modal_rect` to clamp overlay rectangles and test very small
terminals. Rectangle safety does not prove readable text: wrapping must subtract
indentation, and fixed-height prose can push other notices offscreen. Rendered
buffer checks catch those losses; helper-only tests do not. Keep visibility of
node costs, stopping states and crash notices when changing overlay copy.

## Validation scope

The full `apps/tui` suite passed: 451 tests, 2 ignored, including draw-path,
protocol, federation and model-selection cases. This is controlled local
validation, not a live remote TLS/account/provider or interactive-terminal test.
