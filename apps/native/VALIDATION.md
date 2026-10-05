# Native client validation — 2026-09-27

## Native visual redesign (2026-09-29)

The user's visual reference informed a flatter native layout. Dark uses near
black surfaces, neutral selection and restrained blue accents. The chat header
is a slim breadcrumb row with compact actions; project-first session rows replace
the old accent bars, and search sits directly in the sidebar. Settings and agent
setup use sections separated by rules instead of large filled cards.

The composer keeps attachment controls in its footer, displays a focus outline,
and labels messages sent during work as queued. Tool details start collapsed,
except failures, and preserve call-ID expansion and scroll anchors. Recorded
timestamps remain visible; absent timestamps no longer add placeholder labels.
File previews use quiet inline links, copy actions use tooltip-labelled icons,
and turn summaries include change totals and a link to the current Changes view.
Approvals retain expandable request details. The sidebar adapts to narrow
windows and explains empty filters with a reset.

Typography uses bundled Inter and JetBrains Mono with their OFL licenses.
Settings includes searchable interface/code font pickers, conversation sizes,
a live preview and reset. Preferences persist, older settings receive the new
defaults, and palette changes preserve selected fonts. A GPUI regression checks
font picker confirmation, draft preservation and typography across themes.

The final chat pass adds calmer Markdown spacing, round bullets, a restrained
heading scale, smaller icons, hover copy actions, rounded user messages and a
rounded composer with a circular send action. Successful sends no longer leave
“Request accepted” at the top; errors and queued setting changes remain visible.

Markdown file links now request a preview from the selected session's host and
show its contents above the composer. A small, documented GPUI Component patch
adds an optional link callback and list marker while preserving the upstream
Markdown parse cache and selection across paragraphs. An interaction regression
checks that selecting link text does not open it, clicking does request the file,
and the returned preview renders. A real X11 click also opened the README preview.

Validation of the final source:

- Application and harness build passed with the default Rust backend.
- Complete native suite: **125 passed** (53 library, 42 GPUI interaction,
  27 protocol, 3 Rust-host tests).
- Clippy with warnings denied, rustfmt and diff whitespace checks passed.
- Real Linux windows checked under isolated Xvfb with software Vulkan, using
  the rich-transcript fixture: dark, light and Nord; chat, tool details,
  settings and setup; sizes from 720 × 480 to 1200 × 800. Keyboard compose/send
  and scrolling were exercised. Captures caught and corrected setup helper
  text clipping and scrollback showing behind composer shortcut hints.
- Witness returned no native selection, so the full suite was used.
- Pulled main to `4fd429ab` before final checks. A fresh desktop window was
  launched with `--local`; its embedded claudemon and Rust hub expose four owned
  listeners, the facade reports embedded transport and launch readiness, and
  the window shows Connected. A read-only probe of that hub also passed. A
  separate Rust probe checked connection and joined shutdown with all four
  ports released; it launched no agents.
- Running tool indicators now rotate continuously using a stable call/session
  animation key; Done and Failed remain static. The rich fixture retains a
  running subagent for this check. An actual X11 window passed
  `--animation-region 326,295,16,16` at 1200 × 800: four distinct captures of
  only the spinner pixels, 130 ms apart. The full native suite and Clippy
  passed again after this fix. `scripts/smoke.py --animation-region` accepts
  x,y,width,height for repeating this visual regression check.

Saved captures: [chat](docs/ui-polish-chat.png),
[tool details](docs/ui-polish-tools.png), [compact chat](docs/ui-polish-compact.png),
[settings](docs/ui-polish-settings.png), [agent setup](docs/ui-polish-setup.png),
[file preview](docs/ui-polish-file-preview.png).
The standalone vendored component test command could not run offline because
its optional `rust_decimal` dependency was not cached; the native build and GPUI
interaction tests exercised the patched renderer.

These are fixture UI checks, not hardware performance measurements or fresh
Windows/macOS runtime verification.

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
Nord, including 720 × 480. The standalone vendored component test command could not run offline because
its optional `rust_decimal` dependency was not cached; the native build and GPUI
interaction tests exercised the patched renderer.

These are fixture UI checks, not live provider/account
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


## Centered chat and compact elevation (2026-09-30)

Transcript, header and composer now share the same centered 900 px content
column and gutters. Virtual-list items stay full width so their content can
center inside the viewport. At short, narrow sizes the sidebar uses 200 px,
approval controls share a compact heading row, and the composer omits the
shortcut footer. The dock retains its measured scroll geometry and has a
45 percent height cap in short windows. Floating composer, approval and jump
controls use palette-specific shadows; Allow once uses the primary color.

Validation on Linux:

- `cargo test --locked --features ui-tests --no-fail-fast -- --test-threads=1`:
  **126 passed** (53 library, 43 UI/lifetime, 27 protocol, 3 Rust-host).
- The new GPUI regression checks matching chat/composer edges, viewport center,
  and conversation space with an approval at 1600 × 900, 1000 × 700 and 720 × 480.
- Formatting and diff whitespace checks passed.
- Real X11 windows rendered the rich fixture in Dark, Light and Nord at
  720 × 480, plus Dark at 1600 × 900. Captures were visually inspected.
- Parallel full-suite runs failed the existing scroll-to-end follow test;
  the isolated test and complete serialized suite passed. The concurrency
  sensitivity remains unresolved and this pass does not claim parallel green.

The host needed an isolated Cargo cache and locally extracted Linux libraries.
These did not change repository dependencies or build configuration. Window
checks used Xvfb/software Vulkan, no real provider, and do not establish
hardware performance or Windows/macOS runtime behavior.

Captures: [wide](docs/ui-centered-wide.png),
[compact Dark](docs/ui-centered-compact-dark.png),
[compact Light](docs/ui-centered-compact-light.png),
[compact Nord](docs/ui-centered-compact-nord.png).


## Sidebar hierarchy and collapse (2026-09-30)

Expanded navigation now uses 64 px session rows: title first, then project and
non-routine status. Provider and full path remain available in tooltips. Selected
rows retain their highlight on hover. A per-window collapse toggle switches to a
56 px icon rail with two-character session initials, status indicators, navigation,
search expansion, settings and the paused-connection wake control. Both layouts
reuse the same filter, selection command and scroll handle. Search expansion
focuses the existing input; it does not replace the query or composer.

- Full native suite, serialized: **128 passed** (53 library, 45 UI/lifetime,
  27 protocol, 3 Rust-host). New interaction coverage verifies preservation of
  drafts/filters, search focus, rail width and explicit session selection.
- Rust formatting, diff whitespace and Python syntax checks passed.
- Real X11/software-Vulkan rich-fixture windows were inspected at 1200 × 800
  expanded and 720 × 480 collapsed. The smoke script moves the pointer away
  after its scripted click so a displaced control tooltip does not obscure the
  resulting layout.
- Witness left these files unmapped, so the full suite was run. This retains
  the serialized-test scope and cross-platform limitations recorded above.

Captures: [expanded](docs/ui-sidebar-expanded.png),
[collapsed](docs/ui-sidebar-collapsed.png).


## Loading, empty and recovery states (2026-09-30)

The conversation distinguishes workspace startup, session-list loading,
conversation loading, unavailable reads, a fresh conversation and an empty
workspace. Existing rows remain visible through automatic reconnection with a
status banner. Intentional pauses stop the progress animation and expose the
existing generation-bound wake action only for a resumable connection. Refresh
is disabled while a read is already running or ordinary reconnect is underway.
Drafts remain editable offline and retry/first-message actions retain them.
Pinned unavailable sessions and ended empty sessions have separate guidance.

`View.sessions_loading` follows actual fleet request admission, completion and
disconnection. Explicit Refresh publishes conversation loading; routine polling
does not disable the composer. Successful reads clear their matching read error
without clearing unrelated action feedback.

- Complete serialized native suite: **132 passed** (53 library, 48 UI/lifetime,
  28 protocol, 3 Rust-host).
- New GPUI interactions verify startup/loading/empty distinctions, retry and
  composer focus without draft loss, cached scrollback during reconnect, and an
  explicit click for a paused connection's wake action.
- A real loopback WebSocket regression delays replies, fails a conversation
  read, retries and verifies loading flags plus recovery/error clearing.
- Formatting and diff whitespace checks passed. Witness left these files
  unmapped, so the full native suite was run.
- Real X11/software-Vulkan windows were inspected for the empty workspace,
  compact first connection and reconnection retaining rich transcript content.
  Fixture connections only; no provider/model call or Windows/macOS runtime
  verification. The serialized-test and hardware-performance limits above apply.

Captures: [empty workspace](docs/ui-state-empty.png),
[connecting](docs/ui-state-connecting.png),
[reconnecting](docs/ui-state-reconnecting.png).


## Rounded surfaces and keyboard interaction (2026-10-01)

Native palettes now define shared 12 px control, 16 px panel and 24 px composer
radii, plus primary hover/pressed colors. Input outlines are quieter, file
previews use elevation instead of a hard frame, and compact dock spacing keeps
the full composer visible with the reserved focus stroke. Session rows budget
68 px for the stroke and two lines of readable metadata.

Shared controls, session/project rows, theme tiles, tool toggles and file actions
join Tab order when enabled. GPUI owns Enter/Space release activation through
the existing click callbacks. Focus changes a reserved transparent border to
an accent stroke without changing geometry. Session control IDs use session
identity rather than list position. Editing and control focus have distinct
sidebar hints.

The vendored component button renderer now uses its computed hover foreground,
replacing an accidental hardcoded red. Component focus rings use the native
accent at 80 percent opacity. Both changes are recorded in WORKSPACER-PATCHES.md.

- Full serialized native suite: **134 passed** (53 library, 50 UI/lifetime,
  28 protocol, 3 Rust-host).
- New interactions cover Tab traversal, disabled-action skipping, Enter/Space
  key-release activation, unchanged focused bounds, retained drafts and one
  explicit session-selection command. Existing send, input, popup and scroll
  regressions remain enabled.
- Rust formatting, diff whitespace and Python syntax checks passed. Witness
  left the sources unmapped, so the full native suite was run.
- Visually inspected actual X11/software-Vulkan captures: Dark keyboard focus,
  compact Light primary hover, and Nord settings/typography. The smoke CLI now
  accepts --keys and --hover for repeatable control-state captures.
- Pixel inspection caught a zero-blur shadow producing no focus ring in Blade;
  the reserved border avoids that shader path. The first-frame test also caught
  an unsafe dispatch-context lookup in the footer; it now reads owned focus
  handles. Both were fixed before publication and the complete suite passed.

These are fixture window checks, not real-provider or hardware-performance
measurements, and do not add Windows/macOS runtime verification. The serialized
suite limitation recorded above still applies. The standalone vendored component
suite was not run; native tests/builds exercised the patched component.

Captures: [Dark focus](docs/ui-rounded-focus-dark.png),
[compact Light](docs/ui-rounded-compact-light.png),
[Nord settings](docs/ui-rounded-settings-nord.png).

## Codex model display, chat parsing and ordinary child spawning (2026-10-01)

The new-session picker preserves live Codex labels and exact launch IDs, identifies
the reported default, and exposes discovery failures and empty catalogs. A read-only
query to the installed Codex app-server returned eight visible models; this checks
the installed CLI's catalog shape, not every account or future model availability.

Live chat and History now retain raw Codex patches, deleted-file unified patches,
per-file patch headers, namespaced write/edit aliases, command stderr and empty
failed results. Tool previews share patch boundaries with changed-file summaries.
Workspacer spawn calls show the child's first message and offer an explicit child
session action only when the successful receipt names an available fleet session.

- Complete serialized native suite (`cargo test --locked --features ui-tests --
  --test-threads=1`): **142 passed** (60 library, 51 UI/lifetime, 28 protocol,
  3 Rust-host). Coverage includes raw/MCP spawn receipts, failure guards, file
  summary parity, exact model selection and child navigation without draft loss.
- Shared hub `launch_instructions`, `local_spawn` and `models` targets:
  **13 passed**. Inert Claude/Codex fixtures exercise the parent's authenticated
  MCP `spawn_agent`, host-derived lineage, queued first message, recorded tool
  receipt, progress/completion wakes and Codex first-turn skill pointers.
- Rust formatting, diff whitespace and Python fixture syntax checks passed.
  Witness left changed sources/fixtures unmapped; the full native suite and
  explicitly selected backend integration targets were run. This is not a claim
  that the complete shared hub suite was run.
- Actual X11/software-Vulkan rich-fixture windows were inspected for Markdown,
  code highlighting, file-change estimates, approval controls and response-card
  tables. Linux build libraries were extracted into a temporary sysroot because
  this environment lacked system development packages.

No real provider session or model call was made. Windows/macOS runtime and
hardware-performance checks remain outside this validation. Existing serialized
suite and retained-content budget limits still apply.

Captures: [chat](docs/ui-chat-parsing-review.png),
[response card](docs/ui-chat-response-card-review.png).

Publication integration check: rebased onto `b06a333a` to retain its new-session
flow, home-directory model discovery and provider-echo reconciliation. Updated
three model test fixtures and the explicit-project model-request expectation.
The complete serialized native suite on the merged source passed **152 tests**
(62 library, 59 UI/lifetime, 28 protocol, 3 Rust-host). The captures above record
the earlier parsing batch; they were not regenerated for the merged UI.
The 13 selected shared-hub checks also passed on the merged source. Reviewed the
new `providers.listModels.useHomeDirectory` selector in the parameter-policy
fixture; the source scanner and all 34 capability-checker tests passed.

## Inline Workspacer and provider-native child cards (2026-10-01)

Children share compact dispatch cards with distinct bot/terminal source icons.
Reported status, activity, runtime model, cumulative usage and elapsed time update
without changing the parent's draft or reading position. Exact dispatch anchors
support several children; ambiguous children stay in a separate section. Native
provider IDs never become Workspacer session IDs. Child transcript previews are
bounded, read-only and parent-scoped, with explicit refresh and request fencing.

Standalone Claude discovers exact-session sidecars under registered roots without
requiring global hooks. Child hooks/artifacts enrich metadata without taking the
parent's mode, pending decisions or busy counters. Detached children survive
parent idle; completion requires child evidence. Artifact/meta reads reject
redirected paths and oversized payloads. Reported zero usage remains distinct
from absence, unknown starts omit duration, and trimming retains sequence offsets.

- Full serialized native suite against the extended backend: **164 passed**
  (72 library, 60 UI/lifetime, 29 protocol, 3 Rust-host).
- Full claudemon library suite: **908 passed, 4 ignored**. Subsequent focused
  checks covered scan/hook races and canonical path identity; the final exact
  metadata-anchor/subsecond update passed **3 child tests plus 2 Claude/Codex API
  tests**, using native-lock dependency versions in a temporary test crate.
  Tracked lock files were preserved. No real provider or model call was made.
- Formatting, diff whitespace and smoke-script syntax checks passed. Witness
  leaves native/new backend files unmapped, so the suites above were selected
  explicitly rather than reporting empty selection as a pass.
- Actual Linux X11/software-Vulkan windows were inspected at 1200 × 800 and
  720 × 480, including multiple children, distinct source icons and an opened
  native transcript with paired tool output. The smoke CLI supports repeated
  `--click` for capture sequences. GPUI's debug-bound map retains removed
  selectors; interaction tests use newly painted transition markers alongside
  actual request/ownership checks.

These are fixture/recorded-artifact checks, not live-account, GPU-performance or
Windows/macOS rendering verification. The final metadata-only backend refinement
was checked with focused daemon tests after the complete native run.

Captures: [child cards](docs/ui-inline-child-cards.png),
[native child transcript](docs/ui-native-child-transcript.png),
[compact](docs/ui-inline-child-cards-compact.png).

Publication lint follow-up: native platform tests passed, then CI's strict Clippy
gate identified identical color branches and a redundant fixture-init closure.
Combined the equivalent branches, used the direct init function, and moved the
fixture test module below production items. Full local native Clippy passed with
`--locked --all-targets --features ui-tests -- -D warnings` on Rust 1.98.1.

## Tool activity groups and timestamp footers (2026-10-01)

Tool headers retain reported descriptions and show concrete commands, paths and
search targets. Three or more consecutive regular calls collapse into activity
groups with category counts, latest action and running/failure status. Groups
contain at most 12 calls to bound expanded GPUI work. Skill, agent and workflow
dispatches remain independent. Expansion keeps call identities, drafts and raw
transcript indices; tool expansion invalidates the owning group slot.

Message timestamps now follow content in live chat, History and child previews,
including user/tool/plan/fleet and literal-text return paths. Tool output
timestamps follow output. Settings → Chat → 12-hour clock persists an AM/PM
preference with a backwards-compatible 24-hour default. Shared duration labels
use milliseconds below one second.

- Complete serialized native suite: **181 passed** (80 library, 66 UI/lifetime,
  3 background-process, 29 protocol, 3 Rust-host).
- Strict all-target Clippy with UI tests, application/harness build, Rust
  formatting and diff whitespace checks passed.
- Regressions cover grouping boundaries and bounded bursts, tool/group expansion
  without backend commands or draft loss, timestamp footer geometry, readable
  descriptions, clock preference round trips, date/AM/PM formatting and the
  999ms → 1s boundary.
- Witness returned no selection for these native files; the full native suite
  was used. Actual X11/software-Vulkan windows were inspected with the existing
  rich-transcript fixture, extended with a third adjacent call and timestamps.
  Separate X displays are required for simultaneous smoke runs; automatic
  display selection collided during an initial capture attempt.

Captures: [collapsed activity](docs/ui-tool-groups-collapsed.png),
[expanded activity](docs/ui-tool-groups-expanded.png). These are Linux fixture
checks, without live-provider/model calls or Windows/macOS rendering validation.

## Desktop-parity chat Markdown (2026-10-02)

Chat Markdown now matches the Electron renderer: bright bold/italic over a dimmer
`prose` body color, accent inline code on a faint background in the mono font,
accent bullets/muted ordered numbers, underlined h1/h2, 1px rules, and bordered
code blocks with a language header. Syntax colors follow the appearance:
GitHub Dark/Light Default (the desktop shiki themes) and a Nord palette.

Three defects found while verifying, all fixed:

- Bundled Inter/JetBrains Mono are variable fonts and GPUI renders only their
  default instance, so `**bold**` and semibold headings were regular weight.
  Static Medium/SemiBold/Bold (and Mono Bold) instances are now embedded and
  regenerated by `scripts/prepare-fonts.sh`.
- Vendored TextView gave every root block `is_last`, removing all paragraph gaps
  in chat Markdown.
- Vendored TextView kept its creation-time highlight theme across style
  updates; appearance switches now re-highlight existing code. Unstyled tool
  previews now pass the theme highlighter too.

- Complete serialized native suite: **183 passed** (80 library, 68 UI/lifetime,
  3 background-process, 29 protocol, 3 Rust-host), including new appearance →
  syntax-theme and rich-Markdown render coverage.
- Strict all-target Clippy with UI tests and Rust formatting passed.
- Real windows were captured with the `--rich-transcript` fixture on a headless
  Hyprland output (Wayland, not X11) in Dark, Light and Nord, before and after
  the font fix. Captures: [dark](docs/ui-chat-markdown-dark.png),
  [light + nord](docs/ui-chat-markdown-light-nord.png). No live-provider calls or
  Windows/macOS rendering checks.

## Work cards and quiet timestamps (2026-10-02)

Adjacent regular tool calls (now including single calls) render as one work card
modeled on the desktop WorkCard: summary header (`summarize_work`: files changed,
commands, reads, searches, +/−, running/failed, duration) over one-line steps with
category icons and session-relative targets; steps expand in place to the shared
tool details. Timestamps are right-aligned in the disabled tone.

- Complete serialized native suite: **184 passed** (81 library, 68 UI/lifetime,
  3 background-process, 29 protocol, 3 Rust-host); new summary unit coverage and
  updated work-card collapse test.
- Strict all-target Clippy with UI tests, formatting and whitespace passed.
- Real windows checked on a headless Hyprland output with the rich-transcript
  fixture in Dark and Light. No live-provider calls or Windows/macOS rendering.

## Land on latest; rounded work-card hover (2026-10-02)

Removed persisted reading positions and the unread banner from both clients:
desktop reverts db0dc488 (open snaps to the bottom again), native drops
`Settings.reading`/`Bookmark` (old preference files still load; the field is
ignored) and window-activation restores, so opening or switching lands on the
latest message. Work-card hover fills now round to the card's corners because
GPUI overflow clipping is rectangular.

- Native serialized suite **184 passed**; strict Clippy passed. Replaced the
  bookmark test with switch-back-lands-on-latest coverage.
- Desktop renderer typecheck passed; renderer Vitest **1936 passed** (206
  files); `chatTailPin` Playwright renderer spec **2 passed**.

## Floating title pill, inset sidebar, tighter chat (2026-10-02)

The full-width ruled header is now a content-sized floating pill (status,
project / title, provider·model chip, icon actions) over a fade, so history
scrolls softly beneath it. The sidebar and collapsed rail are inset rounded
panels on a chat-colored shell. Assistant copy moved from a reserved row above
each message to the hover-revealed timestamp line, and row padding tightened.

- Native serialized suite **184 passed**; strict Clippy and whitespace passed.
- Real windows checked on a headless Hyprland output in Dark, Light and Nord.

## Brand model badges, rounded inline code, one loader (2026-10-02)

Models render with the provider mark from desktop `agentLogos.tsx` (Claude in
brand clay #D97757, OpenAI mark for Codex) and a product name from
`model_display_name` (`claude-opus-5-5` → "Opus 5.5"). `Session.runtime_model`
holds the resolved id from `statusLine.modelDisplay`/`usage.model` and is never
cleared by alias-only or null snapshots; the selection still wins right after a
switch until the runtime reports the new family. Inline code gets rounded,
padded fills with thin-space margins. Sidebar cards put model and folder on one
line (64px rows) and drop the redundant "Working" label; the title pill shows a
static status dot so the composer line owns the only animated loader.

- Native serialized suite **186 passed**; strict Clippy passed.
- Checked in the live release app against the real session (headless capture
  skipped: the headless output would have taken the user's workspace).

## Tables, quotes, orchestration cards, one card per turn (2026-10-02)

Prose-mode Markdown tables (`render_prose_table` in vendored `node.rs`) and
blockquotes follow desktop `markdown.tsx`. Skill/Subagent/Workflow calls use the
work-card shell with their detail inside the card (`tools::card` `extras`).
`group_span(rows, ix, merge_turn)` folds interior assistant notes into the card
when `Settings.merge_turn_tools` is on (cap 48 rows; 12 when off). Nord
`disabled` moved from nord3 0x4c566a (~1.9:1 on chat) to 0x616e88 (~2.8:1).

- Native serialized suite **190 passed** (new: grouping with notes, orchestration
  card shell, merged turn card, quiet-text contrast across themes); strict
  Clippy and rustfmt passed.
- Rich harness fixture now carries a table, blockquote, interior note and a long
  subagent brief.
- No screenshots: the Hyprland headless output took the user's occupied
  workspace 5 both times it was created (even with a workspace-9 rule), so it was
  removed at once and the visual pass was skipped.

## Context meter, settings categories and search (2026-10-02)

`ContextUsage` on `Session` merges `statusLine`/`usage`/`resolvedContextWindow`
(camelCase hub and snake_case claudemon spellings) and `reading()` twins TUI
`derive_stats` (2% drift tolerance, waiting state). `chrome::context_meter`
renders it in the composer toolbar. Settings moved to `src/ui/settings.rs`:
entries are data (section, title, description, keywords, control), so the rail,
category view and search all filter one list. Sidebar child rows fit the 64px
`uniform_list` slot and use the shared `chrome::brand_badge`.

- Native serialized suite **195 passed**; strict Clippy and rustfmt passed.
- GPUI test debug bounds can outlive rows removed by a later frame, so the
  settings UI test asserts presence only; `settings_match` pins filtering.

## Paged conversations and account usage (2026-10-02)

claudemon `/conversation?limit=N` returns the newest N items plus
`window_first_seq` (`ConversationStore::snapshot_window`, replacing
`snapshot_since`); hub `sessions.conversation` validates and forwards `limit`
beside `sinceSeq`. The native controller reads `CONVERSATION_PAGE` (200) items,
widens by a page on `Command::LoadOlder` and re-folds the wider window, which
keeps existing row keys so the scroll anchor holds. Reconciliation and gap
repairs re-read the same window. The first chat row triggers the next page as
it renders (the list overdraws 250px). On a 4,000-item fixture the first read
dropped from 599 KB to 30 KB.

Usage: `Backend::usage_report` polls `usage.report` on connect and every 60s,
keeping the last reading on failure; `usage::accounts` twins desktop
`usagePacingRows` filtering; the sidebar renders one row per account.

- claudemon: new window unit test + `?limit=` HTTP test; strict Clippy passed.
- hub-rs: `conversation_query` unit test, `limit` validation case in
  `engine_adapter`.
- Native serialized suite **200 passed**; strict Clippy and rustfmt passed.


## UI and UX refresh (2026-10-03)

Baseline `bbe68428`; code at `5302cfc1` (later commits are docs only).
Audited the whole native app against its own design system first. Settings
and New Agent were already polished, and the other screens had drifted from
them.

Findings and what changed:

- **Secondary screens were unfinished.** Changes, Session details, Agent setup,
  Change model and Session history used five different title sizes, a plain
  "Back to chat" text button floating top-right, stacked ghost buttons with no
  primary action, and loading text shown in the warning color. They now share
  `page_view` / `page_header` / `card` / `notice_line` with Settings and
  Projects. Primary actions are filled, End session and Forget are
  error-toned, and Changes is a file list with status chips and a diff panel
  (deletions use `error`, not `warning`).
- **Windows caption collisions.** The app-drawn caption (priority 1) covered
  New Agent's ✕, page actions and the docked file viewer's Copy/Close (the
  known "right preview header clipping"). The modal sheet covered the caption
  buttons. Pages and the docked viewer now start below the caption and have a
  drag strip, and the sheet's backdrop leaves the caption free. Tests:
  `secondary_pages_keep_actions_clear_of_the_caption_and_drag_from_the_top`,
  `file_viewer_controls_stay_clear_of_the_caption` (fails without the fix:
  close at y=27 under a 32px caption).
- **Nord dividers were invisible.** `border` equalled `surface`. Now nord2,
  and `card_dividers_show_on_every_surface` pins it for all themes.
- **Tables.** Inline-code pills in right/center-aligned cells were painted at
  left-aligned positions (GPUI `position_for_index` ignores alignment), and
  narrow chats split short headers mid-word ("Statu/s"). Both are fixed in the
  vendored text code (see `WORKSPACER-PATCHES.md`).
- **Brand consistency.** New Agent and Agent setup used generic Bot/terminal
  icons where the rest of the app shows the Claude/OpenAI marks. Create
  actions now say "agent" (New agent, Start an agent) and lists keep
  "sessions".
- **Responsive.** The Settings category rail shrinks to icons with tooltips
  beside a wide sidebar, and the title stacks above search. At 480px tall,
  Projects drops its explanations so the list keeps its height. History rows
  wrap their actions instead of truncating titles. The New Agent
  missing-folder warning wraps instead of clipping its recovery step.
- **Feedback and keys.** Enter saves the session name. Settings errors show at
  the top instead of below the fold. A shared notice tone keeps "Name saved"
  from reading as a warning. The approval details panel no longer blends into
  its card (`code_block` on `surface`).
- **Launch recency across windows (P3).** A window whose launch failed kept its
  folder, and another window's successful receipt then recorded that folder
  as recently used. Only the launching window records recency now (extended
  `new_session_click_leaves_a_pinned_view_and_creates_the_selected_session`).
- **Parallel UI-test flake: root cause found.** The test zoom (`ZOOM_BITS`)
  and caption preview (`FORCE_CAPTION`) were process-wide, so concurrent GPUI
  tests laid out at another zoom or with caption chrome. Both are per-thread
  in UI-test builds. Parallel `--bin wks-native` went from 4/4 failing runs to
  4/4 passing.

Checks at `5302cfc1` in a scrubbed environment (`env -i`, no `WKS_*` or
provider binaries on `PATH`): `cargo fmt --check`; `cargo clippy --locked
--all-targets --features ui-tests -- -D warnings`; serialized
`cargo test --locked --features ui-tests -- --test-threads=1` (132 + 108 UI
(1 ignored) + 3 + 1 + 37 + 3 pass); 4 parallel UI runs pass;
`cargo build --locked --release --bins`; witness-selected hub-rs
`--test models` passes. Visual: before (debug `bbe68428`) and after (release
`5302cfc1`) captures with private Xvfb, Openbox and lavapipe, using
`native-harness serve --sessions 6 --rich-transcript` and a throwaway
HOME/XDG. Dark, Light and Nord at 1400×900 and 720×480, plus the Windows
caption preview (`WKS_NATIVE_CAPTION=1`). No model or provider ran.

Not done or still open: italics remain upright, because no Inter Italic face is
bundled and the desktop has no source asset (adding one is an asset decision).
"Tooltips above preview" and focus after closing the docked viewer were not
reproduced. No Windows/macOS GPU, real-caption or screen-reader checks.

## Windows caption dragging repair (2026-10-02)

Source and harness evidence establish a cancellation path in the original
`cd7a5028` integration, not just missing geometry:

- The original `Workspace::shell` in `src/ui/navigation.rs` used `track_focus`.
  Pinned GPUI 0.2.2 `src/elements/div.rs:2024-2036` installs an automatic
  mouse-down focus listener that calls `window.prevent_default()`.
- GPUI `src/platform/windows/events.rs:974-982` dispatches the nonclient
  mouse-down through those listeners and returns `Some(0)` when default is
  prevented. That skips the native default processing that starts HTCAPTION
  movement. `events.rs:868-878` already maps Drag to HTCAPTION correctly.
- Original `drag_region` did not occlude, so the focusable shell remained hit.
  `original_non_occluding_drag_is_cancelled_by_shell_focus` reproduces default
  prevention, then proves that occluding the drag hitbox removes it. The flag
  is read through GPUI's public `Window::default_prevented()` after test-platform
  mouse-down dispatch; the private Windows callback is not invoked on Linux.
- Contrary to the first repair's learning, `src/window.rs:775-793` stops its
  reverse hit test at `BlockMouse`, and `window.rs:1133-1146` only considers
  retained IDs for native control areas. Occluding pills/buttons protect their
  bounds. An element ID is not required. `start_window_move` is not a Windows
  fallback in this pinned release.

The repair makes the drag regions themselves occluding, restores the 56px chat
header region behind its occluding title pill, includes expanded-sidebar row
padding, and adds a nonshrinking 40x32px logo grab area to the collapsed rail.
The sidebar remains available on every screen, including New Session. Caption
buttons keep their original native control areas and deferred occluding group.

`app_drawn_caption_drag_surfaces_survive_layouts_and_exclude_controls` covers
720/1000/1600px windows, both sidebar states, all nine screens, and New Session
with minimum/default/maximum preferred sidebar widths (200/304/520px), including
all three at the 720px window minimum where the larger widths are clamped. It
checks positive drag dimensions,
at least 40px beside the expanded controls, usable row padding, title/caption
geometry, no default prevention on drag mouse-down, exclusion of sidebar and
caption buttons and the title pill, no drag below the header, and a working
sidebar toggle. A test-only mouse listener observes the actual Div hitbox;
it does not change propagation/default handling. Forced-caption Linux tests
exercise layout and occlusion, not Windows control callbacks or OS movement.

Final Linux checks (Rust 1.94.1, run from the repository root):

- `cargo test --locked --manifest-path apps/native/Cargo.toml --features ui-tests -- --test-threads=1`:
  **226 passed** (102 library, 86 native binary, 3 background process, 32 protocol,
  3 Rust hub). Serialized because the existing UI suite shares global test state.
- `cargo clippy --locked --manifest-path apps/native/Cargo.toml --all-targets --features ui-tests -- -D warnings`:
  passed.
- `cargo fmt --manifest-path apps/native/Cargo.toml --check` and `git diff --check`:
  passed.
- Rivet `witness.select` and `witness.run` returned empty text for the changed
  Rust files. Selection was unproven; the complete native suite was run instead.

Windows target/toolchain inspection: Rust 1.94.1 has
`x86_64-pc-windows-msvc` installed. The UI-only cross-check
`cargo check --locked --target x86_64-pc-windows-msvc --no-default-features --features ui`
failed in dependency builds because `lib.exe` was unavailable. Retrying with
`CC_x86_64_pc_windows_msvc=clang-cl AR_x86_64_pc_windows_msvc=llvm-lib` progressed
to `ring` but failed because the MSVC CRT header `assert.h` was unavailable.
Neither attempt reached application checking. No Windows runtime was available;
there is no claim of Windows compile or end-to-end success.

Manual Windows acceptance checklist (still required):

1. Build this branch with the Windows MSVC/Visual Studio C++ toolchain and launch
   a separate demo window (`wks-native.exe --demo`), leaving existing app/fleet
   processes alone. Record the built SHA, Windows version, and display scaling.
2. At 720px width and normal/maximized widths, drag the expanded sidebar logo,
   header padding, and blank space around the chat pill. Double-click those
   surfaces to maximize/restore and drag a maximized window to restore/move it.
3. Collapse the sidebar and drag the rail logo. Repeat on Conversation, Projects,
   Settings, Recent, Changes, History, Session, Setup, Model, and New Session.
   Repeat with minimum/default sidebar widths and 100%/150% display scaling.
4. Click sidebar actions, toggle, search, title-pill actions, and minimize/
   maximize/restore/close. Check Snap Layouts on maximize hover, close behavior,
   and return from fullscreen if used. Buttons must not initiate a move.
5. Select transcript text, scroll history, edit/select composer text, and resize
   the sidebar/window. These content interactions must not move the window.
   Check a child conversation's title actions and drag space too.

Integration: at inspection, main was `ca1183e7`; its relevant native source files
had no uncommitted changes. This branch touches the shared `ui.rs`, `chrome.rs`,
`sidebar.rs`, validation notes and Rivet learnings, so recheck overlap before
integration. Main was read only; no merge, push, application restart, or fleet
mutation was performed.

## Wide Markdown tables scroll instead of clipping (2026-10-03)

Review of acb28484 (P2): the whole-word column minimums in `render_prose_table`
could add up to more than the available width. The frame hid the overflow, so
at 720×480 the fourth column of an ordinary table (Component / Implementation /
Validation / Observation) was at x666–778.5, past the window and unreachable.

UX choice (desktop parity, `overflow-x: auto`): a table whose whole-word
minimums fit fills the width and wraps between words, with no scroll
affordance. A table whose minimums cannot fit (many columns, narrow chat or
preview, large interface size) keeps them and scrolls sideways inside its fixed
frame. Words are not broken and columns are not clipped. Scrolling uses the
trackpad, Shift+wheel, an always-visible draggable scrollbar strip under the
rows, or Left/Right after clicking or tabbing to the table. The table is a tab
stop only while it scrolls, and a plain vertical wheel still scrolls the page.
Minimums are now shaped in the cell's own font: semibold header, bold/italic
marks, and monospace code with its thin-space margins. They are capped at 12em,
so URLs and long tokens still wrap inside their column; that is the only
remaining mid-word break. Chat and the Markdown file preview share this
renderer (sheet, docked and popped-out window).

- New UI tests: narrow chat at 720 (selector-free geometry that fails on
  acb28484 with the review's x666–778.5 cell; wheel, keyboard, Tab and
  scrollbar drag to the last column; vertical wheel leaves the table alone),
  tables that fit, 2/3/5/6/8 columns with URLs, inline code, a 38-character
  word and left/center/right alignment, inline code at column minimum (fails
  without the margin fix), two tables with independent scroll, a larger
  interface size, and the Markdown preview as a sheet, docked, and a 420px
  popped-out window. Each column must be seen whole inside the viewport, and
  header cells stay one line.
- Native serialized suite **291 passed, 1 ignored** (132 library, 115 UI,
  3 background, 1 project, 37 protocol, 3 Rust hub) on 6ee3cfea; parallel UI
  binary 115 passed, 1 ignored; strict Clippy, rustfmt and `git diff --check`
  passed. Rustfmt on the vendored `node.rs` reports only three older hunks.
- Private Xvfb/Openbox/lavapipe captures at 720×480 (chat, caption preview,
  sidebar docked), the 720 preview sheet, the 1400×800 docked preview and a
  520×480 popped-out preview. They used a temporary harness copy serving the
  table and a Markdown file; production source was unchanged. Linux only: no
  Windows/macOS, hardware GPU or screen-reader verification.

## Visual pass: tables, quotes, orchestration cards, merged turns (2026-10-03)

The 2026-10-02 follow-up pass had no screenshots. Captured the release build
against `native-harness serve --sessions 3 --turns 0 --rich-transcript` under
private Xvfb + lavapipe (`WAYLAND_DISPLAY` unset, or GPUI opens on the host
compositor): Dark/Light/Nord at 1400 wide, Dark at 720, Dark scrolled back, and
Dark with `merge_turn_tools` (seeded with the new `smoke.py --setting KEY=JSON`).

Fine as built: tables (stripes, header, right alignment, inline code in cells),
blockquote rail, Skill/Spawn/subagent work-card shells and the folded dispatch
brief, the merged turn card (Read, Edit, note and search under one "3 steps"
header), and approval-button contrast in Nord. Fixed from the captures:

- Response-card actions were full-width borderless buttons stacked with large
  gaps, reading as plain text. They are now a wrapping footer row under a rule
  of outlined buttons with the desktop's Lucide icons (`assets/icons/lucide/`)
  and the desktop's effect tooltips.
- Durations over an hour read "3882m 11s"; `duration_label` now gives "64h 42m".
- At 720 the orchestration card title (`flex_shrink_0`, 320px cap) pushed the
  status badge off the card ("Runnin"); the title now shrinks first.
- Scrolled back, history showed through the gaps between the approval card,
  composer and hint line. The dock has a chat-colored backdrop that fades over
  the 12px above it (inside the transcript's dock padding).

- Native serialized suite **291 passed, 1 ignored**; strict Clippy and rustfmt passed.
- Captures: [dock + card actions](docs/ui-visual-pass-dock-card-actions.png),
  [720 wide](docs/ui-visual-pass-narrow.png).

The three items left open here were fixed the same day (next section).

## Ellipsis, italics and 0.9em inline code (2026-10-03)

- **Ellipsis: GPUI 0.2.2 bugs, now patched in `vendor/gpui`** (see its
  `WORKSPACER-PATCHES.md`). `TextLayout` reused any cached size for unwrapped
  text, so `truncate()` text measured unconstrained never re-measured when flex
  shrank it: clipped, never "…". Turning wrapping on (a one-line clamp) to
  dodge the cache collapsed non-growing `min_w_0` boxes (history titles and
  paths vanished) and exposed a second bug: `truncate_line` rewrote the shared
  runs, so a later pass bolded only "Nat" of "Native client experiment". The
  patch keys the cache on truncation width and clones runs per measurement;
  every existing `truncate()` call now ellipsizes. CI path filters include
  `vendor/gpui/**`.
- **Italics:** Inter Italic and Bold Italic are static instances of the official
  Inter 4.1 `InterVariable-Italic.ttf` (same version and OFL license as the
  bundled variable font); `prepare-fonts.sh` downloads it checksum-pinned and
  reproduces both files exactly. Blockquotes and `*emphasis*` are now italic.
- **Inline code at 0.9em:** "JetBrains Mono Inline" is JetBrains Mono with
  outlines and advances scaled to 90% inside the same em and line metrics, so
  it sits on the prose baseline smaller. Vendored gpui-component gained
  `TextViewStyle::inline_code_family` (inline spans and table-width
  measurement); native sets it only while the code font is the bundled
  JetBrains Mono, and hides the face from the font pickers.

- New tests: `truncated_text_ellipsizes_at_its_flex_width_and_recovers` (fails
  on upstream 0.2.2: "narrow title was clipped, not ellipsized"),
  `html_card_actions_are_a_compact_wrapping_row`,
  `inline_code_face_follows_bundled_mono_and_stays_out_of_pickers`.
- Native serialized suite **294 passed, 1 ignored**; strict Clippy and rustfmt passed.
- Release-build captures, Dark/Light/Nord and 720 chat + history:
  [themes](docs/ui-visual-pass-themes.png), [truncation](docs/ui-visual-pass-truncation.png).
  The run-cloning fix has no unit test (the test text system ignores fonts);
  the captures show the bold run intact.

Noticed, not changed: at 720×600 the sidebar's last session row draws under
the usage meters. Linux only: no Windows/macOS, hardware GPU or screen reader.

## Editor, file explorer, agent terminals and Git-style review (2026-10-05)

- **Editor.** The file viewer's source is editable and saves through the hub:
  `file-save` reads the file first and writes only if it still equals what the
  editor loaded, then reads it back. Not atomic: a writer between the compare
  and the write is overwritten without notice (the read-back sees our text);
  a hub-side compare-and-write with a lock would be needed for that. Conflicts offer Overwrite / Reload from
  disk / Keep editing. Unsaved edits are guarded on explorer/Back/document
  navigation, an incoming chat link (deferred until answered), ✕/Esc/backdrop
  and main-window close (`on_window_should_close` →
  `Workspace::confirm_window_close`); Pop out and Dock carry edits and an
  in-flight save. Quit is explicit and does not ask.
- **Explorer.** Lazy `fs.listEntries` tree in the editor (docked, sheet and
  popped-out window) and in the review. Hub `fs.listEntries` gained an
  additive `includeIgnored` option and echoes it; `.git` stays omitted.
  "Git-ignored files are hidden" is shown until Show ignored.
- **Terminal.** One hub-owned shell per agent (`terminals.create` in the
  agent's cwd, `sessions.attachTerminal` / keepalive / input / resize,
  `pty.bytes.<id>`), rendered by a `vt100` emulator in a panel under the chat
  or its own window. Keys go to the shell ahead of every binding (keystroke
  interceptor). Shell rows are filtered out of the session list; the agent →
  shell pairing is remembered per hub. Hub `toggle-terminal` / `new-terminal`
  UI actions drive it; `facade.openTerminal` commands are still not run.
- **Review.** Changes is a Git-style diff (old/new gutters, hunks, tints,
  sideways scrolling) with Changed / Files on the right. `git.status` now
  reports the work-tree `root` (older hubs: the session cwd).

Evidence (Linux, this worktree, serialized UI tests):

- Native `cargo test --locked --features ui-tests -- --test-threads=1`: lib 144,
  GPUI/bin 124 (1 ignored), protocol 37, rust_hub 4, projects_hub 1,
  background_process 3 — all passed. `--no-default-features`: 142 + 37 passed.
  Strict Clippy (`--all-targets --features ui-tests -D warnings`) and rustfmt
  passed.
- New GPUI tests: `editor_saves_through_the_hub_and_never_drops_unsaved_edits`,
  `editor_explorer_lists_the_session_folder_and_opens_files`,
  `review_shows_git_diffs_beside_a_right_hand_file_explorer`,
  `agent_terminal_takes_keys_and_follows_the_selected_agent` (includes pop
  out / dock), `unsaved_edits_move_with_the_editor_into_its_own_window`.
  Three existing viewer tests were updated from "read-only" to the editable
  contract (keys still never reach the workspace underneath).
- Real embedded hub, no model provider:
  `editor_explorer_and_agent_terminal_round_trip_through_the_owned_hub` runs a
  real login shell in the project folder (`echo`/`pwd` output), hides and
  re-attaches to the same shell with its output replayed, confirms the shell
  never appears in sessions, restarts it, verifies a save, refuses a stale
  save (file untouched), forces an overwrite, and lists with/without ignored
  entries.
- hub-rs: `services::files` 5, `tests/git.rs` 8 (new `root` assertion), and
  witness-selected `tests/files.rs` 8, `tests/snapshots.rs` 10,
  `tests/models.rs` 6, federation routing 2 — passed. hub-rs strict Clippy
  fails on ~200 findings that predate this change (none on the changed lines).
- Debug-build captures under private Xvfb against `native-harness serve`
  (whose fixture now answers `fs.listEntries`, `fs.write` in memory and an
  echo-only fake shell that runs nothing):
  [editor, unsaved](docs/ui-editor-unsaved.png),
  [unsaved-changes guard](docs/ui-editor-guard.png),
  [review diff](docs/ui-review-diff.png),
  [review files + editor](docs/ui-review-files.png),
  [terminal panel](docs/ui-terminal-panel.png),
  [terminal window](docs/ui-terminal-window.png).

Not covered: Windows/macOS rendering and ConPTY shells, hardware GPU, a
remote (TLS) hub, IME composition in the terminal, mouse selection/reporting
in the terminal, and the Quit shortcut over unsaved edits.

## Omarchy themes, readable selection, fleet wake cards (2026-10-05)

Items #1, #4, #13, #14, #15, #16, #19, #20 and #21 from the native feedback
round, on worktree branch `wks/native-themes-visuals-usage`.

- **Selection (#1):** vendored gpui-component `Inline::paint` painted the
  selection quads after the text, and native used the opaque row highlight as
  `theme.selection`, so selected chat/Markdown text (chat, viewer sheet/dock
  and popped-out viewer share `Inline`) vanished. Selection now paints under
  the glyphs (patch noted in `vendor/gpui-component/WORKSPACER-PATCHES.md`),
  and every palette has a translucent `selection` token. Tests composite it
  over chat, code, user and card surfaces and require 4.5:1 text; syntax
  tokens must stay ≥3:1 when selected in the new themes.
- **Themes (#16):** Tokyo Night, Catppuccin Mocha/Latte, Gruvbox and
  Everforest from Omarchy `colors.toml` (read only), with syntax sets in their
  editor conventions, `Appearance::is_dark()` for the light palettes, a
  wrapping eight-tile picker, `t` cycling and persistence. New themes meet
  7:1 text, 4.5:1 prose and primary labels, 3:1 tones; the three pre-existing
  palette shortfalls (Dark primary label 3.68, Light success on base 3.00,
  Nord error on cards 2.46) are pinned as named exceptions, unchanged.
- **Fleet wakes (#20):** the parser ports desktop `ENTRY_RE` and passes every
  case in `contracts/fleet-message-cases.json`. Wakes render as worker cards:
  tone rail and kind overline, titled with the worker's name (or "N
  sessions") instead of "You", live status (Needs approval → Resolved),
  model/folder/short-ID metadata, Open (direct workers only, rechecked on
  click), draft-preserving Reply, Last reply and Original wake disclosures.
- **Nesting (#19):** project filter and search now match whole lineages, so
  workers in `~/.workspacer/worktrees/...` stay under a manager whose checkout
  is the open project (the reported "SESSIONS 1").
- **Usage (#14):** logins without a reading are listed (Sign in again /
  Refresh failed / No reading yet, with the hub's reason); a failed
  `usage.report` with nothing cached shows "Usage unavailable".
- **#13** context window is an even segmented control (Default no longer
  removes 1M); **#4** the requested-session notice is a themed card with icon
  actions; **#15** no Connected label/dot while healthy; **#21** timestamp
  footers keep 6px above / 8px below and share the right edge on tool cards.

Checks (`CARGO_BUILD_JOBS=4`): `cargo fmt --check`, strict Clippy and the
serialized suite (`--test-threads=1`) passed: **313 passed, 1 ignored**
(139 library, 130 UI, 3 background, 1 project, 37 protocol, 3 Rust hub). New GPUI tests cover each item. GPUI never clears `debug_bounds`
between frames, so disappearance is asserted on state, not bounds.

Private Xvfb + lavapipe captures (debug build, fixture harness with two wakes,
a 1M selection and a sign-in-needed login): [themes](docs/ui-omarchy-themes-dark.png),
[Latte/Nord/Light/Dark](docs/ui-omarchy-themes-latte-baseline.png),
[selection](docs/ui-selection-readable.png), [wake cards](docs/ui-fleet-wake-cards.png),
[context window](docs/ui-context-window-segmented.png),
[usage](docs/ui-usage-unmeasured.png), [theme picker](docs/ui-theme-picker.png).
Not captured: the request notice (needs a hub UI request; covered by a GPUI
geometry test). Linux only: no Windows/macOS, hardware GPU or live provider.
