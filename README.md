# Workspacer

**A control plane and IDE for coding agents.** Run a whole fleet of
long-lived coding agents — Claude Code, Codex, GitHub Copilot, and OpenCode — side by side
in one local-first desktop app. Watch every agent's state at a glance, step in
only when one actually needs you, and review and ship the code they write
without leaving the window.

It's not a chat box bolted onto an editor. Workspacer treats agents as
first-class, long-running processes: each one gets its own workspace (tabs,
terminals, a browser, a git review pane — notes are available via the Notes
plugin), lives in a background daemon whose sessions persist and resume across
app restarts — quitting the app pauses agents cleanly and the next launch picks
the most recent session back up, and under `workspacer serve` the daemon
outlives the desktop app entirely — and reports up into a single control plane
you can also check from your phone.

> Website & docs: open `landing/index.html` for the overview and
> `landing/docs.html` for the full user guide (both are static, no build step).

---

## Why it exists

One agent in a terminal is easy — you sit and watch it. That falls apart the
moment you have three, five, ten going at once. You lose track of which agent
is working, which is blocked on an approval, and which finished ten minutes
ago and is sitting idle burning nothing. Tabbing between terminals, most of the
switching you do doesn't pay off.

Workspacer keeps the ambient state of every agent in front of you so switching
becomes a decision you make, not a reflex you act on out of fear of missing
something:

- **See who needs you.** A per-agent status dot (idle / working /
  needs-approval / done), a live "N need you / N working" header, and OS
  notifications that stay quiet for the agent you're already watching.
- **Act without hunting.** A Triage Inbox collects every approval and question
  across the fleet like an email inbox; jump straight to the next agent waiting
  on you.
- **Close the loop.** When an agent finishes, review its diff, stage, commit,
  and push from a built-in review pane instead of dropping to a shell.

Typical uses: run several agents across different repos at once; babysit a long
refactor while doing other work; dispatch a Fleet Manager that coordinates the
rest of the fleet; clear approvals and questions from your phone.

## What you get

- **Multi-agent workspaces** — one agent = one long-lived session = one
  workspace, keyed by working directory, with its own tabs and panes.
- **Four agent backends** — Claude Code, Codex, GitHub Copilot, and OpenCode, all
  landing in the same fleet with the same state, telemetry, and approval
  prompts.
- **A real GUI for each agent** — approve/deny in one key, answer questions
  inline, read diffs as they land, follow a clean work log, attach files by
  drag/paste/picker. (Plus a raw terminal view for Claude.)
- **Pane types** — terminal, browser, git review/diff, a prompt/skill/agent
  library, analytics, an overview, markdown preview/editor, and plugin panes
  (per-agent notes ship as the Notes plugin).
- **Layout your way** — flip the chrome between full workspace mode and
  `focus` (distraction-free chat rail) without remounting live panes. An
  advanced Agent Overview is there when you want the whole board.
- **Attention & notifications** — status dots, aggregate counts, jump-to-next,
  and configurable OS notifications.
- **Phone access** — share this desktop with your phone when you need to step
  away: scan a QR code, clear approvals/questions, send quick messages, and
  keep the agents running on the desktop. Sharing is opt-in, token-authenticated,
  Tailscale-friendly, and capability-scoped (`view` / `triage` / `operator`).
- **Extensible** — a plugin system (drop in a manifest, get a supervised
  sidecar with its own panes) and an MCP facade that exposes the fleet as tools
  a Fleet Manager can drive. A dozen official plugins — fleet dashboards,
  policy auto-approval, budget brakes, Slack/phone bridges, and more — live in
  the [plugin catalog](https://github.com/DJTouchette/workspacer-plugins),
  installable from the command palette.

Not all of the above is in the native app yet: its New Agent starts Claude
and Codex sessions only, it lists only the connected hub's own sessions (not a
federated peer's), and plugin panes and the Agent Overview belong to the web
client (`/app`). For a feature-by-feature catalog with maturity levels, see
[`docs/features.md`](docs/features.md); for what the native app does today, see
[`apps/native/README.md`](apps/native/README.md).

---

## Quick start

Released builds are on the
[releases page](https://github.com/DJTouchette/workspacer/releases): a Windows
x64 installer, macOS `.dmg` files (Apple Silicon and Intel) and Linux
`.tar.gz` archives (x64 and arm64). From source:

```bash
make dev              # build and run the native app (debug) with its embedded backend
```

`make dev` builds `apps/native` with Cargo and starts `wks-native --local`. The
app runs the Rust hub, the MCP tool facade and the `claudemon` session engine in
its own process, so nothing else has to be running and there are no JavaScript
dependencies to install first. `./dev` does the same and passes its arguments
through. See [`apps/native/README.md`](apps/native/README.md) for build
prerequisites and everything the app does.

The Electron app in `apps/desktop` is deprecated; v0.170.0 was its last
release. Its renderer still ships as the `/app` web client in the server
bundle. To work on it, run `make install-desktop` and then `make dev-desktop`.

Claude sessions started from the app carry their hooks with each launch;
nothing is merged into `~/.claude/settings.json` (entries an earlier version
put there are removed at startup). Codex sessions are driven directly. The
native app's New Agent starts Claude and Codex sessions only. For a standalone
`claudemon`, the manual hook CLI still exists: `claudemon init` (`--dry-run`
to preview).

Phone access is off until you turn it on in **Settings → Remote**, which
shares the hub over Tailscale and creates pairing links. `make dev-share`
starts the Electron dev loop with sharing forced on
(`WORKSPACER_REMOTE_SHARE=1`); the native app does not read that variable.

### Advanced clients and server mode

The normal product is the desktop app plus phone access. Power-user surfaces
still exist when you need them:

- `wks-tui` is a terminal client for the same hub bus.
- `/remote` is the lightweight browser client.
- `/app/` is the full web client (the former Electron renderer), served by
  `workspacer serve` from the server bundle. The native app's packages do not
  include it; its hub serves `/app` only when `WORKSPACER_WEBAPP_DIR` or a `web`
  folder beside the binary holds a built web client.
- `workspacer serve` runs the control plane without the desktop app.

#### Headless server

Every release also ships a standalone server bundle
(`workspacer-server-<os>-<arch>.tar.gz`, zip on Windows) — the Rust backend
executable (`workspacer`, also present as `workspacer-rust`), `claudemon`,
example plugins and the built web app. Two lines and the fleet is up:

```bash
tar xzf workspacer-server-linux-x64.tar.gz
./workspacer-server/workspacer serve
```

`workspacer serve` runs the session engine, the hub, the desktop services and
the MCP facade in one process, and
prints the client URLs + pairing token: the phone client at `/m`, the
lightweight browser client at `/remote`, and the full web app at `/app`
(served from the bundled `web/` directory). It binds loopback by default; `--host 0.0.0.0` (or a
tailnet IP) is the explicit remote opt-in — pair it with Tailscale, never the
open internet. It runs in the foreground by design (systemd/tmux friendly).

Also in the CLI: `workspacer status` (what's running), `workspacer install-cli`
(put the binary on PATH), and `workspacer token create/list/revoke`
for capability-scoped bus tokens (`--scope view|triage|operator`). From source,
`make build-cli` builds the same backend executable.

To use the native app as a client of a running server instead of its own
backend, start `wks-native` without `--local` (it connects to
`ws://127.0.0.1:7895/bus`) or pass `--bus <url>` with `--token-file`.

## Architecture at a glance

You launch one thing, the native app, and it owns the backend in its own
process:

```
apps/
  native/      Rust/GPUI app (wks-native) with an embedded Rust backend — the
               standard client; what `make dev` and releases build
  desktop/     Electron + React app — deprecated (final release v0.170.0);
               its renderer is still built as the /app web client
  tui/         wks-tui — Rust terminal client over the hub bus
services/
  claudemon/   Rust session daemon: owns sessions/PTYs, runs per-provider
               adapters, streams conversation and usage (git review is
               handled by the clients)
  hub-rs/      Shared Rust backend: hub, MCP, plugins, desktop services and
               the standalone workspacer-rust launcher
plugins/       bundled plugin examples
docs/          specs, design notes, and the feature catalog
landing/       the marketing site + user docs (static HTML)
```

The native GUI embeds the Rust backend and session
engine; `workspacer-rust serve` runs the same services as a standalone process.
The Go hub, `brain` and the Node companion are retired; the Go sources live
only in the pinned historical checkout described by
`scripts/reference/README.md`. Some hub tests, generators and contract
fixtures still read files under `apps/desktop` (see
[the migration gates](services/hub-rs/MIGRATION.md)).

The advanced clients (`wks-tui`, `/remote`,
`/app/`) and the phone client all connect to the same `hub` bus, so a session
running on one client can be observed and driven from another.

## Common tasks (from the repo root)

| Command                | What it does                                          |
| ---------------------- | ----------------------------------------------------- |
| `make dev`             | Native app (debug) with its embedded Rust backend     |
| `./dev`                | Same as `make dev`; arguments pass through to `wks-native` |
| `make dev-native`      | Native app against an already running hub             |
| `make dev-desktop`     | Deprecated Electron app in dev mode (needs `make install-desktop`) |
| `make dev-share`       | Electron dev loop with remote sharing forced on       |
| `make dev-tui`         | Run wks-tui (debug); builds the Rust services first |
| `make run-tui`         | Run wks-tui (release); builds the Rust services and TUI first |
| `make build`           | Build all components                             |
| `make build-hub`       | Build the standalone shared Rust backend |
| `make build-cli`       | Build `workspacer-rust` (backend and administration CLI) |
| `make build-claudemon` | `cargo build --release` for claudemon                 |
| `make build-tui`       | `cargo build --release` for wks-tui                   |
| `make test`            | Desktop + hub + claudemon + tui + native test suites  |
| `make install`         | Linux: build the native app and install it under `~/.local` |
| `make package`         | Native download for this OS, into `apps/native/target/package/` |
| `make package-desktop` | Deprecated Electron installers (not part of a release) |
| `make check-version`   | `VERSION` and `apps/desktop/package.json` must agree  |
| `make clean`           | Remove build artifacts across all components          |

Each component also builds independently from its own directory — see the
per-component READMEs (`apps/native/README.md`, `apps/desktop/README.md`, `apps/tui/README.md`,
`services/claudemon/README.md`, `services/hub-rs/MIGRATION.md`).

## Toolchains

Rust is via the standard `cargo`/`rustup` toolchain; the native app and the
backend need nothing else. [`mise`](https://mise.jdx.dev) (`mise.toml`) pins
Node 22 for repo scripts, native Windows packaging and the web client in
`apps/desktop`, and Go 1.25 for the optional historical reference tooling only.

The release version is the root `VERSION` file. `apps/desktop/package.json`
carries a copy for the web client, and `scripts/check-version.mjs` fails when
the two differ.

---

## Contributing

Contributions are welcome. Start with **[CONTRIBUTING.md](CONTRIBUTING.md)** for
the dev setup, how the codebase is laid out, the test/lint expectations, and the
pull-request flow. TL;DR: fork or branch, `make build`, make your
change with tests, run `make test`, and open a PR against `main`.

Found a security-sensitive issue? See [`SECURITY.md`](SECURITY.md) — please
don't file those as public issues.

## License

Workspacer is **open source** under the [MIT License](LICENSE) — read it, build
it, modify it, redistribute it, use it commercially, no strings attached. The
only requirement is that the copyright notice and license text travel with
copies or substantial portions of the software. See the [LICENSE](LICENSE) file
for the full terms.
