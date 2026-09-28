# Native client validation — 2026-09-27

## Embedded backend and launch controls

Current change validation:

- `cargo test --locked --features ui-tests`: **48 passed** — 16 core/host,
  15 GPUI, and 17 protocol tests.
- `cargo test --locked --manifest-path services/claudemon/Cargo.toml --test embedded`:
  passed. Exercises channel commands, listener readiness, failed startup/restart,
  a real disposable PTY child, and shutdown with a stalled HTTP request body.
- `go test ./cmd/workspacer` and `go test -race ./cmd/workspacer`: passed.
- `go test ./internal/capspec`: passed.
- Formatting and diff whitespace checks: passed.
- `native-harness embedded-probe` against actual hub/brain/MCP binaries: passed
  with isolated configuration/database, four Claude family choices, zero agents
  launched, and all four owned listener ports released after shutdown.

Independent review fixed bounded shutdown admission, cleanup error preservation,
shared idle-usage configuration, and GPUI termination-hook ownership. The actual
stack smoke exposed Tokio `Child::wait` closing the parentwatch stdin pipe;
separate pipe ownership fixed it, a real-child regression passed, and the full
smoke passed afterward.

This run did not execute a real provider/model conversation or test macOS/Windows
on those operating systems. The provider and performance results below describe
the earlier client baseline, not the new embedded mode. The current native build
also emits an existing handoff formatter deprecation warning under its resolved
`time` version; compilation succeeds.

## Initial-client automated checks

- `cargo test --locked --features ui-tests`: **20 passed** — 4 model,
  11 WebSocket/controller/live-harness, and 5 GPUI tests.
- `cargo clippy --locked --all-targets --features ui-tests -- -D warnings`: passed.
- `cargo build --locked --release --bin wks-native`: passed, followed by the
  real-window smoke against the optimized executable.
- Rust formatting, Python harness syntax, workflow YAML, and diff whitespace:
  passed.
- Independent reviewer rechecked allocation bounds, stream reset/readiness races,
  revision collisions, draft handling, and live-test targeting. Reported issues
  were fixed and received regressions; no remaining reviewed blocker.

The tests use actual loopback WebSockets and GPUI's window/input harness. They
exercise protocol ordering and rendering behavior rather than asserting only
helper return values. Witness initially reported the new app as unmapped; the
complete native suite was run directly.

## Real-provider check

The repeatable `scripts/live-stack.py` command started an isolated hub, brain,
and claudemon using temporary configuration, database, and loopback ports. Global
hook installation was skipped and the tool facade was explicitly disabled for
the no-tool prompts. The running production hub's credentials were not changed.

**Codex passed:**

1. Launch a disposable stream session and receive `NATIVE_TEST_READY`.
2. Send through the production native controller and receive `NATIVE_FOLLOWUP_OK`.
3. Reconstruct the conversation in a fresh client, without its previous cache.
4. Open that session in the actual GPUI window and capture its rendered text.
5. Shut down the native window and isolated backend; temporary state was removed.

The final live controller exercise took **14.159 seconds**, including provider
startup and two model replies; the first reply arrived at 11.866 seconds. These
are not UI latency measurements. [The captured window](docs/live-codex.png) shows
both assistant responses and the connected test session.

Claude was attempted but could not authenticate: its local OAuth session was
expired and could not be refreshed. That test did not pass. The harness terminated
the failed disposable session and shut down its isolated backend.

## Performance scope

The reducer benchmark covers a 5,000-item initial snapshot, 20,000 streaming
events, and periodic immutable UI snapshots. Its bounds are asserted independently
of timing: at most 2,000 retained rows / 4 MiB text, with clipped allocations also
bounded. Run the JSON benchmark command in README for measurements on your machine.

Final optimized reducer sample on this Linux host: **12.1 µs p99** per event/
periodic snapshot handoff, 20,000 events in 24.97 ms, and 1,530 retained rows /
4,192,600 text bytes. Initial 5,000-item construction/folding took 65.89 ms.
This sample excludes text layout, GPU work, network traffic, and process RSS;
it is a reproducible workload measurement, not a hardware-independent guarantee.

Native window smoke passed under Linux Xvfb + Mesa software Vulkan, including
focus, resize, typing, send, and a nonblank pixel capture. This validates a real
window but is not representative hardware GPU performance. The release binary was
26,271,608 bytes. Its fixture smoke observed window appearance at 504 ms,
132,247,552 bytes RSS, and 12.7% of one CPU core over a three-second sample after
a three-second settle. This includes the in-process fixture and CPU-based Vulkan
rendering, and does not establish steady-state hardware idle CPU or a speedup
over Electron. Representative desktop GPU measurements remain necessary.

Windows/macOS build-and-test jobs are configured, but were not executed locally.
Live permissions/tool approvals, PTY terminals, and remote TLS/account setup are
outside this recorded live check. The first implementation's scope and remaining
features are listed in README.

## Native everyday-workflow pass (2026-09-27)

The full native UI/protocol suite and Clippy with warnings denied were run. New
regressions cover upload/session binding, failed-send drafts, stale reads, resume
and termination, model/context preservation, literal answers, setup return, text
paste, bitmap conversion, and lossless long-message history.

The affected Go brain, MCP, bus and capability-specification packages passed.
Windows payload tests passed, including NSIS fixture compilation; the Windows
installation smoke now also asserts notification-identity registration/removal.
Actual Windows notification/bitmap-clipboard delivery and macOS OS integration
were not run on this Linux host. A separate claudemon answer-regression build
ran out of workspace disk space; the Go forwarding and native wire regressions
passed, but that additional daemon test did not execute.

Real GPUI windows were captured under Xvfb/software Vulkan in dark, light and
Nord, including 720 × 480. These are fixture UI checks, not live provider/account
or hardware-performance measurements.

Witness selects no native tests, so the complete native suite was used. Its CLI
runner also chose root-relative Go invocations and Jest for desktop TypeScript;
those generated invocations failed before testing. The affected Go packages
were run directly from services/hub instead.


## Rich transcript pass (2026-09-28)

- Full native suite: **83 passed** (34 core, 26 GPUI interaction, 23 protocol).
  Added cases cover paired and failed tools, bounded structured payloads,
  snapshot row reuse, lossless large tool history, thumbnail decoding, literal
  attachment/file targets, inert card HTML, queued acknowledgement, session
  reading restoration, and owner-validated response actions.
- Native Clippy with warnings denied and rustfmt checks passed. The embedded
  claudemon dependency still emits its pre-existing `time::format_description`
  deprecation warning; native code has no Clippy warnings.
- Linux real-window smoke passed under Xvfb/Mesa software Vulkan against the
  new `native-harness serve --rich-transcript` fixture. Inspected captures of
  the response card/table and attachment preview. This is not a hardware GPU
  performance measurement or a real-provider round trip.
- All three Windows payload tests passed locally, including compiling the NSIS
  fixture with the same checksum-verified compiler resolved for CI. Actual
  Windows install/backend/uninstall behavior remains the hosted release smoke's
  responsibility.
- Witness selected no native tests and reported the files as unmapped, so the
  full native suite was run rather than treating the empty selection as a pass.

The prior release's Windows packaging failure was a Chocolatey NSIS lookup
failure, before installer compilation. The release workflow now resolves NSIS
from the desktop packaging toolchain and carries its `NSISDIR` into packaging.

CI screenshot inspection also caught a GPUI Component 0.5.1 double-parse issue:
its HTML minifier emits decoded text without re-escaping it. Native literal text
and sanitized card text now encode for both parser passes. A regression verifies
that tags/entities remain literal and escaped image/script text cannot become
active nodes. The corrected literal text was checked in a real-window capture.

## Turn-summary helper measurement (2026-09-29)

The visible turn footer calls `transcript::turn_changes`. It previously called
`Tool::changes`, built every inline diff line, and discarded those strings while
aggregating file names and added/removed counts. The summary now uses the same
parser with diff construction disabled. Inline tool details still retain their
full diff. Semantic tests cover Edit, MultiEdit, Write, multi-file patches,
context-only files, repeated paths, failed tools and incomplete tools.

Reproduce the workload with `native-harness bench-turn-summary --tools 200
--lines 80 --iterations 200` (see README for the Cargo invocation). It uses
558,100 bytes of synthetic completed edit input, five warmups, and checks both
line totals and absence of diff payloads in its summaries.

On this Linux container, using the same **unoptimized development profile**
(`debug_assertions: true`) before and after the change:

| Metric | Before | After |
| --- | ---: | ---: |
| p50 helper time | 18.413 ms | 9.512 ms |
| p95 helper time | 18.789 ms | 9.827 ms |
| p99 helper time | 18.991 ms | 9.986 ms |
| 200 measured iterations | 3.693 s | 1.909 s |

This is one bounded helper comparison, not a GUI frame-time, release-build or
Electron comparison. It excludes layout, painting, GPU, network and provider
work. It does not establish that the native client feels faster than Electron.
The complete display-independent native library suite passed (37 tests); no
new GUI build or hardware measurement was performed for this change.

## Reconciled native chat polish (2026-09-29)

Integrated the `0c89afff` chat polish with main's rich transcript, native feature
screens and Rust backend. Individual highlighted tool cards now consume the
existing structured tool model, while response cards, guarded host actions,
attachment previews, retained history, reading bookmarks and turn-file summaries
continue through the shared rich renderer. Floating header/composer measurements
and live scroll-anchor remapping coexist with saved per-session reading state.

Validation on Linux:

- `cargo test --locked --features ui-tests --no-fail-fast`: **121 passed**
  (52 library, 39 GPUI interaction, 27 protocol, 3 Rust-host tests).
- `cargo clippy --locked --all-targets --features ui-tests -- -D warnings`:
  passed. Simplified an existing stale-request Boolean guard without changing
  its behavior; all 27 protocol tests passed again afterward.
- `cargo fmt --check` and `git diff --check`: passed.
- Native application and harness link checked with the default Rust backend.

The recovered regressions cover floating composer growth at normal and minimum
window sizes, scrollback through updates, keyboard paging, visible-tail follow,
call-ID expansion through snapshot/result updates, real syntax grammars, pinned
session creation, server timestamps and persisted durations. Additional coverage
preserves timestamp-free streaming row identity and namespaced/camel-case edit
inputs. Existing rich-content, attachment, history, response-action and backend
ownership tests remain enabled.

Witness returned no usable selection from its original-checkout index; the full
native suite was used. The X11 real-window smoke was not run on this host because
Xvfb/xdotool are unavailable. Windows/macOS runtime checks remain CI coverage;
this validation does not claim fresh cross-platform or hardware GPU results.
