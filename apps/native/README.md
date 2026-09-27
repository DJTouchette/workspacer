# Workspacer Native

Experimental GPUI client for an existing Workspacer hub. Rust renders the
interface directly; no Electron, browser, or webview is used. GPUI and
GPUI Component are pinned together; the optional component webview feature is
disabled. The backend remains claudemon + hub + the existing capability provider.

## Run

Use a current stable Rust toolchain. On Debian/Ubuntu, install the native build
dependencies first:

```sh
sudo apt-get install libxkbcommon-dev libxkbcommon-x11-dev libfontconfig1-dev libwayland-dev libssl-dev libasound2-dev libvulkan1 mesa-vulkan-drivers
```

macOS needs Xcode command-line tools; Windows needs the MSVC Rust toolchain and
Visual Studio C++ build tools. Linux needs a working X11/Wayland display and
Vulkan driver. The CI workflow builds/tests all three platforms; local validation
results are recorded separately below.

From `apps/native`:

```sh
# Isolated demo: 100 sessions, 1,000 transcript entries, simulated streaming.
# Does not read credentials, launch agents, or call a model.
cargo run --locked -- --demo

# Connect to an already running desktop hub or workspacer serve.
cargo run --locked --release -- --bus ws://127.0.0.1:7895/bus

# Remote hub. Credentials are carried in an Authorization header.
cargo run --locked --release -- --bus wss://my-host/bus --token-file /path/to/token

# Open a specific session, without falling back to another session if absent.
cargo run --locked -- --session SESSION_ID
```

`WKS_HUB_BUS` can supply the URL. Credentials resolve from `--token-file`, then
`HUB_TOKEN`, then the existing Workspacer `remote-token` file **only for loopback
URLs**. Config directory rules match the existing clients (`APPDATA` on Windows;
`XDG_CONFIG_HOME`, otherwise `~/.config`, on Unix including macOS). Local credentials
are never automatically forwarded to an explicitly remote hub.

The client connects to existing sessions. It does not own or stop backend
processes on exit. The connected hub needs a provider for `sessions.snapshots`,
`sessions.conversation`, `agents.sendMessage`, `claude.approve`, `claude.answer`,
and `claude.signal`.

## First slice

Captured during a real Codex round trip through an isolated backend:

![Native client displaying the verified live conversation](docs/live-codex.png)

- Virtualized session sidebar with workspace paths and status.
- Selected conversation, selectable Markdown/code, per-message copy.
- Streaming messages, composer drafts per session, approvals, free-text answers,
  and interrupt controls. Failed/unknown-outcome sends preserve their drafts.
- Reconnection, snapshot reseeding, delta-gap recovery, and stale-response guards.
- `Alt+Up/Down` switches sessions; `Ctrl/Cmd+L` focuses the composer;
  `Ctrl/Cmd+Enter` sends; plain Enter inserts a newline; `Ctrl/Cmd+R` refreshes.
- Scrollback retains its position while new text arrives. Jump to latest resumes
  following the conversation.

This experiment intentionally starts with the connected hub's own sessions.
Federated/paired rows are excluded so a remote session cannot accidentally be
controlled through an unqualified local method. Terminal emulation, session
creation, historical pagination, attachments, model settings, full theme parity,
and packaging/updating remain follow-on work. Rich tool input/output is displayed
as text, without the Electron client's diff cards.

The native palette maps the desktop design language's semantic surfaces, accent,
status colors, spacing, and chat measure into Rust constants. Controls have text
labels; there is no parallel icon vocabulary. This is an intentional native
prototype theme, not support for the desktop CSS theme registry.

## Performance boundaries

- Network/JSON work lives in a two-thread Tokio runtime, off the GPUI UI thread.
- Subscribe to one selected conversation; release its topic on selection changes.
- UI updates use a single-slot latest-value mailbox, capped at approximately
  30 updates/second during streaming. The controller publishes no unchanged idle
  snapshots; GPUI manages cursor and platform repainting.
- Transcript: at most 2,000 rows / 4 MiB of retained text, with a 64 KiB per-row
  cap. Clipping is labeled. These are text-storage bounds, **not total RSS limits**.
- Immutable rows share allocations across frames. Only changed row measurements
  are invalidated; GPUI lays out visible transcript/sidebar rows.
- Bounded command/event queues, frame-size limits, call deadlines, and reconnect
  backoff. Mutations are never automatically replayed after connection loss.
- A proven push path suppresses fast polling. A 30-second reconciliation catches
  provider restarts and removed sessions; older providers fall back to a
  one-second selected-conversation fetch.

## Feedback loop

```sh
# Fast core/protocol suite: works without a display or GUI libraries.
cargo test --locked --no-default-features

# Optional, read-only check of real hub contracts. Prints only counts.
cargo run --locked --no-default-features --bin native-harness -- probe --token-file /path/to/token

# GPUI's deterministic window/input harness plus all protocol tests.
cargo test --locked --features ui-tests
cargo clippy --locked --all-targets --features ui-tests -- -D warnings
cargo fmt --check

# Optimized reducer + immutable-frame handoff measurement, JSON output.
cargo run --locked --release --no-default-features --bin native-harness -- bench --events 20000

# Separate fixture process for measuring the UI's own RSS/CPU.
cargo run --locked --no-default-features --bin native-harness -- serve --sessions 1000 --turns 5000
cargo run --locked --release -- --bus ws://127.0.0.1:7896/bus

# Linux: real native window + keyboard input + PNG, with RSS/CPU observations.
# Requires xdotool and libX11; xvfb-run can provide a display in CI.
python3 scripts/smoke.py --binary target/release/wks-native
```

Protocol tests use actual loopback WebSockets, including deliberate disconnects,
missing deltas, a snapshot racing an event, and a delayed response after switching
sessions. GPUI tests exercise real key dispatch, Unicode composition, late send
receipts, and the number of rows constructed for a 2,000-entry transcript.
The fixture has no provider side effects. Tests against it do not prove live
Claude/Codex behavior or remote TLS/account configuration.

An explicit live-provider harness is available separately. It launches **one real
agent** in an existing absolute scratch directory, sends two short no-tool prompts,
verifies assistant output through the production controller, verifies a fresh
client can reconstruct the conversation, then sends SIGTERM to that session.
It requires an authorized operator credential and may incur provider usage:

```sh
cargo run --locked --no-default-features --bin native-harness -- live \
  --token-file /path/to/authorized-token --cwd /absolute/scratch-directory
```

`--provider codex` selects Codex; the default is Claude. `--keep-open` deliberately
leaves the disposable session available for window inspection with `--session`;
the caller then owns termination. The harness never retries an uncertain spawn.
An unknown-outcome spawn without an ID requires inspecting the backend before
retrying. Live mode is never invoked by demo, automated tests, or CI.

On Linux, `scripts/live-stack.py` can own the whole disposable stack instead of
using an existing hub. It requires installed `workspacer`, `hub`, `brain`, and
`claudemon` binaries plus an authenticated provider CLI. It chooses unused
loopback ports, uses temporary config/SQLite/plugin directories, skips global
hook installation, explicitly disables the tool facade for these no-tool prompts,
waits for capability registration, and shuts the stack down:

```sh
python3 scripts/live-stack.py --harness target/debug/native-harness --provider codex
# Also capture the real conversation in the native window (DISPLAY + xdotool):
python3 scripts/live-stack.py --harness target/debug/native-harness \
  --provider codex --native target/release/wks-native --output native-live.png
```

These commands make real model calls. Their isolated hub credential is private
to the temporary directory; they do not rotate or broaden the existing hub's
credentials. Provider authentication/history still belongs to the invoking user.

The benchmark reports reducer time, snapshot handoff time, and retained text.
It deliberately does not call those values GPU frame latency or startup time.
For comparisons with Electron, run both clients against the same backend and
workload, measuring client RSS, total-stack RSS, idle CPU, cold startup, and
input/scroll latency separately. Use release binaries on representative hardware.

The X11 smoke script drives focus, resize, typing and send, then captures native
pixels and fails on a blank window. It reports window-appearance time (not time
to first usable frame), sampled RSS, and idle CPU. Its default in-process demo
includes fixture memory; `--bus` selects a separately running fixture for cleaner
client-only measurements. Software Vulkan and debug builds are useful correctness
checks, not representative performance results.

Independent review found and corrected oversized string-capacity retention,
snapshot/reset ordering (including separate event/reply queues), the asynchronous
subscription-readiness gap, and virtual-list invalidation when revisions restart.
Each has a regression test. A GUI keyboard test also caught and corrected the
composer's Enter binding taking precedence over the send shortcut.

## Code map

| File | Responsibility |
| --- | --- |
| `src/bus.rs` | Socket ownership, authentication, subscriptions, deadlines, reconnects |
| `src/model.rs` | Session projection and bounded, sequence-aware transcript reducer |
| `src/controller.rs` | Selection, RPC lifecycle, snapshot/event reconciliation, UI mailbox |
| `src/ui.rs` | GPUI views, virtualization, keyboard dispatch, drafts |
| `src/harness.rs` | Isolated protocol fixture |
| `src/live.rs` | Explicit disposable live-provider exercise |
| `src/bin/native-harness.rs` | Repeatable fixture and benchmark commands |
| `tests/protocol.rs` | Wire-level and controller regressions |

Rivet/Witness may initially report these new files as unmapped. That is not a
test pass; run the complete native-client suites above.
